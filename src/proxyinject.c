/*
 * proxyinject.dll - per-process transparent TCP proxying for Windows.
 *
 * Injected into ONE process. Rewrites that process's outbound TCP connections
 * to go through the currently selected proxy (SOCKS5 / SOCKS4 / HTTP CONNECT).
 * Everything else on the machine - including a Clash TUN adapter - is
 * untouched, because we hook the process's own socket calls, not the network.
 *
 * Technique
 *   - 5-byte rel32 jmp patched onto the prologues of ws2_32 connect,
 *     WSAConnect, sendto and WSASendTo. Each function is fully reimplemented,
 *     so no trampoline / instruction decoder is needed.
 *   - SIO_GET_EXTENSION_FUNCTION_POINTER is intercepted by rewriting the
 *     WSAIoctl entry in every loaded module's import table, so apps that use
 *     ConnectEx (Node/Electron, Chrome, .NET async) stay covered.
 *   - Upstream connects use the real mswsock ConnectEx captured at init.
 *
 * Config (next to this DLL):
 *   config.ini    behaviour
 *   proxies.txt   one proxy per line
 *   state.txt     runtime state, written by the DLL
 *   logs\PID.log  per-process event log
 */

#define WIN32_LEAN_AND_MEAN
#define _CRT_SECURE_NO_WARNINGS
#include <winsock2.h>
#include <windows.h>
#include <ws2tcpip.h>
#include <mswsock.h>
#include <tlhelp32.h>
#include <stdio.h>
#include <stdlib.h>
#include <stdint.h>
#include <string.h>
#include <stdarg.h>

/* local copy of WSAID_CONNECTEX so we never need uuid.lib */
static const GUID k_connectex_guid =
    {0x25a207b9, 0xddf3, 0x4660, {0x8e, 0xe9, 0x76, 0xe5, 0x8c, 0x74, 0x06, 0x3e}};

typedef BOOL(WINAPI *PFN_CONNECTEX)(SOCKET, const struct sockaddr *, int,
                                    PVOID, DWORD, LPDWORD, LPOVERLAPPED);

#define MAX_PROXIES   4096
#define MAX_TRACKED   1024
#define LOG_MAX_BYTES (4 * 1024 * 1024)

/* -------------------------------------------------------------- state ---- */

typedef struct {
    char host[160];
    int  port;
    int  proto; /* 0 socks5, 1 http, 2 socks4 */
    char user[96];
    char pass[96];
    struct sockaddr_storage ss;
    int  sslen;
    DWORD resolved_at;
} Proxy;

static Proxy g_px[MAX_PROXIES];
static int   g_npx = 0;
static LONG  g_cur = 0;

static int g_enabled     = 1;
static int g_fail_closed = 1;
static int g_block_udp   = 1;
static int g_dns_udp     = 1;
static int g_log_on      = 1;
static HANDLE g_ready_evt = NULL;   /* set once hooks are live (injector waits) */
static int g_rotate_sec  = 0;
static int g_retry       = 1;
static int g_timeout_ms  = 10000;
static int g_default_proto = -1;
static int g_hook_connectex = 1;
static int g_forced = 0;

static char    g_dir[MAX_PATH];
static char    g_logpath[MAX_PATH];
static wchar_t g_dirw[MAX_PATH];

static CRITICAL_SECTION g_cs;
static CRITICAL_SECTION g_logcs;
static int g_cs_ok = 0, g_logcs_ok = 0;

static LONG  g_ready = 0;
static DWORD g_last_rotate = 0;
static DWORD g_last_reload_check = 0;
static uint64_t g_sig_cfg = 0, g_sig_px = 0;
static DWORD g_pid = 0;

static PFN_CONNECTEX g_connectex = NULL;
static HINSTANCE g_self = NULL;

/* forward declarations (used by the IAT sweeper and each other) */
static int  WSAAPI hooked_connect(SOCKET s, const struct sockaddr *name, int namelen);
static BOOL WINAPI hooked_ConnectEx(SOCKET, const struct sockaddr *, int, PVOID,
                                    DWORD, LPDWORD, LPOVERLAPPED);
static int  WSAAPI hooked_WSAIoctl(SOCKET, DWORD, LPVOID, DWORD, LPVOID, DWORD,
                                   LPDWORD, LPWSAOVERLAPPED,
                                   LPWSAOVERLAPPED_COMPLETION_ROUTINE);
static int(WSAAPI *g_real_wsaioc)(SOCKET, DWORD, LPVOID, DWORD, LPVOID, DWORD,
                                  LPDWORD, LPWSAOVERLAPPED,
                                  LPWSAOVERLAPPED_COMPLETION_ROUTINE) = NULL;

/* ---------------------------------------------------------- utilities ---- */

static void log_line(const char *s)
{
    if (!g_log_on || !g_logcs_ok || !g_logpath[0]) return;
    EnterCriticalSection(&g_logcs);
    HANDLE h = CreateFileA(g_logpath, FILE_APPEND_DATA,
                           FILE_SHARE_READ | FILE_SHARE_WRITE, NULL,
                           OPEN_ALWAYS, FILE_ATTRIBUTE_NORMAL, NULL);
    if (h != INVALID_HANDLE_VALUE) {
        LARGE_INTEGER sz;
        if (GetFileSizeEx(h, &sz) && sz.QuadPart > LOG_MAX_BYTES) {
            CloseHandle(h);
            char oldp[MAX_PATH + 8];
            _snprintf(oldp, sizeof(oldp) - 1, "%s.1", g_logpath);
            MoveFileExA(g_logpath, oldp, MOVEFILE_REPLACE_EXISTING);
            h = CreateFileA(g_logpath, FILE_APPEND_DATA,
                            FILE_SHARE_READ | FILE_SHARE_WRITE, NULL,
                            CREATE_ALWAYS, FILE_ATTRIBUTE_NORMAL, NULL);
        }
        if (h != INVALID_HANDLE_VALUE) {
            DWORD wr = 0;
            WriteFile(h, s, (DWORD)strlen(s), &wr, NULL);
            CloseHandle(h);
        }
    }
    LeaveCriticalSection(&g_logcs);
}

static void logf_(const char *fmt, ...)
{
    if (!g_log_on) return;
    char body[1024];
    va_list ap;
    va_start(ap, fmt);
    _vsnprintf(body, sizeof(body) - 1, fmt, ap);
    va_end(ap);
    body[sizeof(body) - 1] = 0;

    SYSTEMTIME st;
    GetLocalTime(&st);
    char line[1200];
    _snprintf(line, sizeof(line) - 1,
              "[%04d-%02d-%02d %02d:%02d:%02d.%03d] [%lu] %s\r\n",
              st.wYear, st.wMonth, st.wDay, st.wHour, st.wMinute, st.wSecond,
              st.wMilliseconds, (unsigned long)g_pid, body);
    line[sizeof(line) - 1] = 0;
    log_line(line);
}

static void str_trim(char *s)
{
    char *p = s;
    while (*p == ' ' || *p == '\t' || *p == '\r' || *p == '\n') p++;
    if (p != s) memmove(s, p, strlen(p) + 1);
    size_t n = strlen(s);
    while (n && (s[n - 1] == ' ' || s[n - 1] == '\t' || s[n - 1] == '\r' ||
                 s[n - 1] == '\n'))
        s[--n] = 0;
}

static int read_file_utf8(const wchar_t *path, char *buf, int cap)
{
    HANDLE h = CreateFileW(path, GENERIC_READ, FILE_SHARE_READ | FILE_SHARE_WRITE,
                           NULL, OPEN_EXISTING, FILE_ATTRIBUTE_NORMAL, NULL);
    if (h == INVALID_HANDLE_VALUE) return -1;
    DWORD rd = 0;
    int total = 0;
    while (total < cap - 1) {
        if (!ReadFile(h, buf + total, (DWORD)(cap - 1 - total), &rd, NULL) || rd == 0)
            break;
        total += (int)rd;
    }
    buf[total] = 0;
    CloseHandle(h);
    if (total >= 3 && (unsigned char)buf[0] == 0xEF &&
        (unsigned char)buf[1] == 0xBB && (unsigned char)buf[2] == 0xBF)
        memmove(buf, buf + 3, (size_t)(total - 3) + 1);
    return total;
}

static uint64_t file_sig(const wchar_t *path)
{
    WIN32_FILE_ATTRIBUTE_DATA fad;
    if (!GetFileAttributesExW(path, GetFileExInfoStandard, &fad)) return 0;
    uint64_t t = ((uint64_t)fad.ftLastWriteTime.dwHighDateTime << 32) |
                 fad.ftLastWriteTime.dwLowDateTime;
    uint64_t s = ((uint64_t)fad.nFileSizeHigh << 32) | fad.nFileSizeLow;
    return (t * 1000003ULL) ^ (s + 0x9E3779B97F4A7C15ULL);
}

static void b64enc(const char *in, char *out, int outcap)
{
    static const char *t =
        "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    const unsigned char *p = (const unsigned char *)in;
    size_t n = strlen(in), i = 0;
    char *o = out;
    if (outcap < 8) { if (outcap) out[0] = 0; return; }
    while (i + 2 < n && (o - out) < outcap - 5) {
        unsigned v = ((unsigned)p[i] << 16) | ((unsigned)p[i + 1] << 8) | p[i + 2];
        *o++ = t[(v >> 18) & 63];
        *o++ = t[(v >> 12) & 63];
        *o++ = t[(v >> 6) & 63];
        *o++ = t[v & 63];
        i += 3;
    }
    if (i < n && (o - out) < outcap - 5) {
        unsigned v = (unsigned)p[i] << 16;
        if (i + 1 < n) v |= (unsigned)p[i + 1] << 8;
        *o++ = t[(v >> 18) & 63];
        *o++ = t[(v >> 12) & 63];
        *o++ = (i + 1 < n) ? t[(v >> 6) & 63] : '=';
        *o++ = '=';
    }
    *o = 0;
}

/* ------------------------------------------------------ proxy parsing ---- */

static const char *skip_scheme(const char *spec, int *proto)
{
    if (!strncmp(spec, "socks5h://", 10) || !strncmp(spec, "socks5://", 9)) {
        *proto = 0;
        return strstr(spec, "://") + 3;
    }
    if (!strncmp(spec, "socks4a://", 10) || !strncmp(spec, "socks4://", 9)) {
        *proto = 2;
        return strstr(spec, "://") + 3;
    }
    if (!strncmp(spec, "http://", 7) || !strncmp(spec, "https://", 8)) {
        *proto = 1;
        return strstr(spec, "://") + 3;
    }
    if (!strncmp(spec, "socks://", 8)) {
        *proto = 0;
        return spec + 8;
    }
    return NULL;
}

/* host:port | host:port:user:pass | user:pass@host:port */
static int parse_proxy_spec(const char *raw, Proxy *out)
{
    char tmp[512];
    _snprintf(tmp, sizeof(tmp) - 1, "%s", raw);
    tmp[sizeof(tmp) - 1] = 0;
    str_trim(tmp);
    if (!tmp[0] || tmp[0] == '#' || tmp[0] == ';') return 0;

    char *s = tmp;
    int proto = g_default_proto;
    const char *after = skip_scheme(s, &proto);
    if (after) s = (char *)after;

    char user[96] = "", pass[96] = "", host[160] = "", portbuf[16] = "";

    char *at = strrchr(s, '@');
    if (at) {
        *at = 0;
        char *colon = strchr(s, ':');
        if (colon) {
            *colon = 0;
            _snprintf(user, sizeof(user) - 1, "%s", s);
            _snprintf(pass, sizeof(pass) - 1, "%s", colon + 1);
        } else {
            _snprintf(user, sizeof(user) - 1, "%s", s);
        }
        s = at + 1;
    }

    char *parts[6];
    int np = 0;
    char *p = s;
    while (np < 6) {
        parts[np++] = p;
        char *c = strchr(p, ':');
        if (!c) break;
        *c = 0;
        p = c + 1;
    }
    if (np < 2) return 0;

    _snprintf(host, sizeof(host) - 1, "%s", parts[0]);
    _snprintf(portbuf, sizeof(portbuf) - 1, "%s", parts[1]);
    if (np >= 4 && !user[0]) {
        _snprintf(user, sizeof(user) - 1, "%s", parts[2]);
        _snprintf(pass, sizeof(pass) - 1, "%s", parts[3]);
    }
    str_trim(host);
    str_trim(portbuf);
    if (!host[0]) return 0;
    int port = atoi(portbuf);
    if (port < 1 || port > 65535) return 0;

    if (proto < 0) {
        if (port == 1080 || port == 1081 || port == 9050 || port == 9150)
            proto = 0;
        else
            proto = 1;
    }

    memset(out, 0, sizeof(*out));
    _snprintf(out->host, sizeof(out->host) - 1, "%s", host);
    _snprintf(out->user, sizeof(out->user) - 1, "%s", user);
    _snprintf(out->pass, sizeof(out->pass) - 1, "%s", pass);
    out->port = port;
    out->proto = proto;
    return 1;
}

static int is_private_v4(const unsigned char *b)
{
    if (b[0] == 127) return 1;
    if (b[0] == 10) return 1;
    if (b[0] == 172 && b[1] >= 16 && b[1] <= 31) return 1;
    if (b[0] == 192 && b[1] == 168) return 1;
    if (b[0] == 169 && b[1] == 254) return 1;
    if (b[0] == 100 && b[1] >= 64 && b[1] <= 127) return 1;
    if (b[0] == 0) return 1;
    if (b[0] >= 224) return 1;
    return 0;
}

static int is_private_v6(const unsigned char *b)
{
    static const unsigned char lo[16] = {0};
    if (!memcmp(b, lo, 16)) return 1;
    static const unsigned char map[12] = {0,0,0,0,0,0,0,0,0,0,0xff,0xff};
    if (!memcmp(b, map, 12)) return is_private_v4(b + 12);
    if (b[0] == 0xfe && (b[1] & 0xc0) == 0x80) return 1;
    if ((b[0] & 0xfe) == 0xfc) return 1;
    if (b[0] == 0xff) return 1;
    return 0;
}

/* --------------------------------------------------------- destination ---- */

typedef struct {
    int  atyp;
    unsigned char ip[16];
    char domain[256];
    int  port;
    char text[300];
} Dest;

static int dest_from_sockaddr(const struct sockaddr *sa, int len, Dest *d)
{
    memset(d, 0, sizeof(*d));
    if (!sa) return 0;
    if (sa->sa_family == AF_INET && len >= (int)sizeof(struct sockaddr_in)) {
        const struct sockaddr_in *in = (const struct sockaddr_in *)sa;
        d->atyp = 1;
        memcpy(d->ip, &in->sin_addr, 4);
        d->port = ntohs(in->sin_port);
        char buf[64] = "";
        InetNtopA(AF_INET, (void *)&in->sin_addr, buf, sizeof(buf) - 1);
        _snprintf(d->text, sizeof(d->text) - 1, "%s:%d", buf, d->port);
        return 1;
    }
    if (sa->sa_family == AF_INET6 && len >= (int)sizeof(struct sockaddr_in6)) {
        const struct sockaddr_in6 *in6 = (const struct sockaddr_in6 *)sa;
        d->port = ntohs(in6->sin6_port);
        const unsigned char *b = (const unsigned char *)&in6->sin6_addr;
        static const unsigned char map[12] = {0,0,0,0,0,0,0,0,0,0,0xff,0xff};
        char buf[96] = "";
        if (!memcmp(b, map, 12)) {
            d->atyp = 1;
            memcpy(d->ip, b + 12, 4);
            InetNtopA(AF_INET, (void *)d->ip, buf, sizeof(buf) - 1);
            _snprintf(d->text, sizeof(d->text) - 1, "%s:%d", buf, d->port);
        } else {
            d->atyp = 4;
            memcpy(d->ip, b, 16);
            InetNtopA(AF_INET6, (void *)d->ip, buf, sizeof(buf) - 1);
            _snprintf(d->text, sizeof(d->text) - 1, "[%s]:%d", buf, d->port);
        }
        return 1;
    }
    return 0;
}

static int dest_is_local(const Dest *d)
{
    if (d->atyp == 1) return is_private_v4(d->ip);
    if (d->atyp == 4) return is_private_v6(d->ip);
    return 0;
}

/* -------------------------------------------------------- name -> ip map -- */
/* Transparent proxy setups (Clash TUN, fake-ip) hand the app a synthetic
 * address such as 198.18.1.169. Sending that to an upstream proxy is useless,
 * so we remember which hostname produced which address and hand the proxy the
 * hostname instead. The proxy then resolves it at the exit node. */

#define MAX_MAP 2048

typedef struct {
    unsigned char ip[16];
    int   atyp;
    char  name[200];
} MapEnt;

static MapEnt g_map[MAX_MAP];
static int    g_mapn = 0;

static void map_add(const unsigned char *ip, int atyp, const char *name)
{
    if (!g_cs_ok || !name || !name[0]) return;
    if (atyp != 1 && atyp != 4) return;
    size_t n = strlen(name);
    if (n > sizeof(g_map[0].name) - 1) return;
    if (!_stricmp(name, "localhost")) return;

    EnterCriticalSection(&g_cs);
    for (int i = 0; i < g_mapn; i++) {
        if (g_map[i].atyp == atyp && !memcmp(g_map[i].ip, ip, atyp == 1 ? 4 : 16)) {
            _snprintf(g_map[i].name, sizeof(g_map[i].name) - 1, "%s", name);
            goto done;
        }
    }
    if (g_mapn < MAX_MAP) {
        MapEnt *e = &g_map[g_mapn++];
        e->atyp = atyp;
        memcpy(e->ip, ip, atyp == 1 ? 4 : 16);
        _snprintf(e->name, sizeof(e->name) - 1, "%s", name);
    } else {
        /* drop the oldest quarter */
        int drop = MAX_MAP / 4;
        memmove(&g_map[0], &g_map[drop], sizeof(MapEnt) * (MAX_MAP - drop));
        g_mapn = MAX_MAP - drop;
        MapEnt *e = &g_map[g_mapn++];
        e->atyp = atyp;
        memcpy(e->ip, ip, atyp == 1 ? 4 : 16);
        _snprintf(e->name, sizeof(e->name) - 1, "%s", name);
    }
done:
    LeaveCriticalSection(&g_cs);
}

static int map_lookup(const unsigned char *ip, int atyp, char *out, int cap)
{
    int found = 0;
    if (!g_cs_ok) return 0;
    EnterCriticalSection(&g_cs);
    for (int i = 0; i < g_mapn; i++) {
        if (g_map[i].atyp != atyp) continue;
        if (memcmp(g_map[i].ip, ip, atyp == 1 ? 4 : 16)) continue;
        _snprintf(out, (size_t)cap - 1, "%s", g_map[i].name);
        out[cap - 1] = 0;
        found = 1;
        break;
    }
    LeaveCriticalSection(&g_cs);
    return found;
}


/* --------------------------------------------------- live socket table ---- */

static SOCKET g_tsock[MAX_TRACKED];
static USHORT g_tport[MAX_TRACKED];
static int    g_tn = 0;

static void track_socket(SOCKET s)
{
    struct sockaddr_in local;
    int l = (int)sizeof(local);
    USHORT port = 0;
    if (getsockname(s, (struct sockaddr *)&local, &l) == 0) port = local.sin_port;
    if (!g_cs_ok) return;
    EnterCriticalSection(&g_cs);
    if (g_tn < MAX_TRACKED) {
        g_tsock[g_tn] = s;
        g_tport[g_tn] = port;
        g_tn++;
    } else {
        memmove(&g_tsock[0], &g_tsock[1], sizeof(SOCKET) * (MAX_TRACKED - 1));
        memmove(&g_tport[0], &g_tport[1], sizeof(USHORT) * (MAX_TRACKED - 1));
        g_tsock[MAX_TRACKED - 1] = s;
        g_tport[MAX_TRACKED - 1] = port;
    }
    LeaveCriticalSection(&g_cs);
}

static void kill_tracked_sockets(void)
{
    if (!g_cs_ok) return;
    SOCKET socks[MAX_TRACKED];
    USHORT ports[MAX_TRACKED];
    EnterCriticalSection(&g_cs);
    int n = g_tn;
    g_tn = 0;
    memcpy(socks, g_tsock, sizeof(SOCKET) * (size_t)n);
    memcpy(ports, g_tport, sizeof(USHORT) * (size_t)n);
    LeaveCriticalSection(&g_cs);

    int killed = 0;
    for (int i = 0; i < n; i++) {
        struct sockaddr_in local;
        int l = (int)sizeof(local);
        if (getsockname(socks[i], (struct sockaddr *)&local, &l) != 0) continue;
        if (local.sin_port != ports[i]) continue; /* handle reused */
        shutdown(socks[i], SD_BOTH);
        killed++;
    }
    if (killed) logf_("ROTATE dropped %d live connection(s)", killed);
}

/* -------------------------------------------------------- config load ---- */

static char *ini_get(const char *blob, const char *key)
{
    /* non-destructive: the blob is scanned again for every key */
    static char out[512];
    const char *p = blob;
    while (p && *p) {
        const char *eol = strchr(p, '\n');
        size_t len = eol ? (size_t)(eol - p) : strlen(p);
        if (len < sizeof(out)) {
            char line[512];
            memcpy(line, p, len);
            line[len] = 0;
            char *s = line;
            str_trim(s);
            if (s[0] && s[0] != '#' && s[0] != ';') {
                char *eq = strchr(s, '=');
                if (eq) {
                    *eq = 0;
                    char *k = s;
                    str_trim(k);
                    if (!_stricmp(k, key)) {
                        char *v = eq + 1;
                        str_trim(v);
                        _snprintf(out, sizeof(out) - 1, "%s", v);
                        out[sizeof(out) - 1] = 0;
                        return out;
                    }
                }
            }
        }
        p = eol ? eol + 1 : NULL;
    }
    return NULL;
}
static int ini_int(char *blob, const char *key, int dflt)
{
    char *v = ini_get(blob, key);
    if (!v || !v[0]) return dflt;
    return atoi(v);
}

static void load_proxies_locked(void)
{
    wchar_t path[MAX_PATH];
    _snwprintf(path, MAX_PATH - 1, L"%ls\\proxies.txt", g_dirw);
    path[MAX_PATH - 1] = 0;

    static char buf[768 * 1024];
    int n = read_file_utf8(path, buf, sizeof(buf));
    if (n < 0) { g_npx = 0; return; }

    int count = 0;
    char *p = buf;
    while (p && *p && count < MAX_PROXIES) {
        char *eol = strchr(p, '\n');
        if (eol) *eol = 0;
        Proxy tmp;
        if (parse_proxy_spec(p, &tmp)) g_px[count++] = tmp;
        if (!eol) break;
        p = eol + 1;
    }
    g_npx = count;
}

static void load_config_locked(int first)
{
    wchar_t path[MAX_PATH];
    _snwprintf(path, MAX_PATH - 1, L"%ls\\config.ini", g_dirw);
    path[MAX_PATH - 1] = 0;

    static char buf[64 * 1024];
    int n = read_file_utf8(path, buf, sizeof(buf));
    LONG old_cur = g_cur;

    if (n > 0) {
        g_enabled       = ini_int(buf, "enabled", g_enabled);
        g_fail_closed   = ini_int(buf, "fail_closed", g_fail_closed);
        g_block_udp     = ini_int(buf, "block_udp", g_block_udp);
        g_dns_udp       = ini_int(buf, "allow_dns_udp", g_dns_udp);
        g_log_on        = ini_int(buf, "log", g_log_on);
        g_rotate_sec    = ini_int(buf, "rotate_sec", g_rotate_sec);
        g_retry         = ini_int(buf, "retry_next", g_retry);
        g_timeout_ms    = ini_int(buf, "timeout_ms", g_timeout_ms);
        g_hook_connectex= ini_int(buf, "hook_connectex", g_hook_connectex);
        if (g_timeout_ms < 1000) g_timeout_ms = 1000;

        char *sch = ini_get(buf, "default_scheme");
        if (sch) {
            if (!_stricmp(sch, "socks5")) g_default_proto = 0;
            else if (!_stricmp(sch, "http")) g_default_proto = 1;
            else if (!_stricmp(sch, "socks4")) g_default_proto = 2;
            else g_default_proto = -1;
        }
        load_proxies_locked();

        int cur = ini_int(buf, "current", (int)g_cur);
        if (g_npx > 0) {
            if (cur < 0) cur = 0;
            if (cur >= g_npx) cur = g_npx - 1;
            g_cur = cur;
        } else {
            g_cur = 0;
        }
    } else {
        load_proxies_locked();
    }

    if (first) g_last_rotate = GetTickCount();
    if (!first && old_cur != g_cur && g_enabled) {
        logf_("CONFIG proxy #%ld of %d", (long)g_cur, g_npx);
        kill_tracked_sockets();
    }
}

static void maybe_reload(void)
{
    if (g_forced) return;
    DWORD now = GetTickCount();
    if ((DWORD)(now - g_last_reload_check) < 400) return;
    g_last_reload_check = now;

    wchar_t cfg[MAX_PATH], pxf[MAX_PATH];
    _snwprintf(cfg, MAX_PATH - 1, L"%ls\\config.ini", g_dirw);
    cfg[MAX_PATH - 1] = 0;
    _snwprintf(pxf, MAX_PATH - 1, L"%ls\\proxies.txt", g_dirw);
    pxf[MAX_PATH - 1] = 0;

    uint64_t sc = file_sig(cfg), sp = file_sig(pxf);
    if (sc == g_sig_cfg && sp == g_sig_px) return;

    EnterCriticalSection(&g_cs);
    g_sig_cfg = sc;
    g_sig_px = sp;
    load_config_locked(0);
    LeaveCriticalSection(&g_cs);
}

static void write_state(void)
{
    char sdir[MAX_PATH + 8], tmp[MAX_PATH + 16], path[MAX_PATH + 16], body[768];
    char exe[160] = "-";
    wchar_t wpath[MAX_PATH];

    if (GetModuleFileNameW(NULL, wpath, MAX_PATH)) {
        wchar_t *slash = wcsrchr(wpath, L'\\');
        const wchar_t *base = slash ? slash + 1 : wpath;
        WideCharToMultiByte(CP_UTF8, 0, base, -1, exe, sizeof(exe) - 1, NULL, NULL);
        exe[sizeof(exe) - 1] = 0;
    }

    _snprintf(sdir, sizeof(sdir) - 1, "%s\\state", g_dir);
    sdir[sizeof(sdir) - 1] = 0;
    CreateDirectoryA(sdir, NULL);

    _snprintf(tmp, sizeof(tmp) - 1, "%s\\%lu.tmp", sdir, (unsigned long)g_pid);
    tmp[sizeof(tmp) - 1] = 0;
    _snprintf(path, sizeof(path) - 1, "%s\\%lu.txt", sdir, (unsigned long)g_pid);
    path[sizeof(path) - 1] = 0;
    _snprintf(body, sizeof(body) - 1,
              "pid=%lu\r\nexe=%s\r\ncurrent=%ld\r\nproxies=%d\r\nenabled=%d\r\n"
              "rotated_ms=%lu\r\n",
              (unsigned long)g_pid, exe, (long)g_cur, g_npx, g_enabled,
              (unsigned long)g_last_rotate);
    body[sizeof(body) - 1] = 0;
    HANDLE h = CreateFileA(tmp, GENERIC_WRITE, 0, NULL, CREATE_ALWAYS,
                           FILE_ATTRIBUTE_NORMAL, NULL);
    if (h != INVALID_HANDLE_VALUE) {
        DWORD wr = 0;
        WriteFile(h, body, (DWORD)strlen(body), &wr, NULL);
        CloseHandle(h);
        MoveFileExA(tmp, path, MOVEFILE_REPLACE_EXISTING);
    }
}

/* -------------------------------------------------------------- socket ---- */

static DWORD deadline_left(DWORD deadline)
{
    DWORD now = GetTickCount();
    if ((int32_t)(deadline - now) <= 0) return 0;
    return deadline - now;
}

static int wait_io(SOCKET s, int for_write, DWORD deadline)
{
    for (;;) {
        DWORD left = deadline_left(deadline);
        if (!left) return -1;
        fd_set fs;
        FD_ZERO(&fs);
        FD_SET(s, &fs);
        struct timeval tv;
        tv.tv_sec = (long)(left / 1000);
        tv.tv_usec = (long)((left % 1000) * 1000);
        int r = select(0, for_write ? NULL : &fs, for_write ? &fs : NULL, NULL, &tv);
        if (r > 0) return 0;
        if (r == 0) return -1;
        if (WSAGetLastError() != WSAEINTR) return -1;
    }
}

static int x_send(SOCKET s, const void *buf, int len, DWORD deadline)
{
    const char *p = (const char *)buf;
    while (len > 0) {
        int r = send(s, p, len, 0);
        if (r > 0) { p += r; len -= r; continue; }
        int e = WSAGetLastError();
        if (e == WSAEWOULDBLOCK) {
            if (wait_io(s, 1, deadline) != 0) return -1;
            continue;
        }
        return -1;
    }
    return 0;
}

static int x_recv(SOCKET s, void *buf, int len, DWORD deadline)
{
    for (;;) {
        int r = recv(s, (char *)buf, len, 0);
        if (r >= 0) return r;
        int e = WSAGetLastError();
        if (e == WSAEWOULDBLOCK) {
            if (wait_io(s, 0, deadline) != 0) return -1;
            continue;
        }
        if (e == WSAEINTR) continue;
        return -1;
    }
}

static int x_recv_exact(SOCKET s, void *buf, int len, DWORD deadline)
{
    char *p = (char *)buf;
    int got = 0;
    while (got < len) {
        int r = x_recv(s, p + got, len - got, deadline);
        if (r <= 0) return -1;
        got += r;
    }
    return 0;
}

static void hs_timeouts_push(SOCKET s, int ms, DWORD *oldrcv, DWORD *oldsnd)
{
    int l = (int)sizeof(DWORD);
    *oldrcv = 0;
    *oldsnd = 0;
    int have_r = getsockopt(s, SOL_SOCKET, SO_RCVTIMEO, (char *)oldrcv, &l) == 0;
    l = (int)sizeof(DWORD);
    int have_s = getsockopt(s, SOL_SOCKET, SO_SNDTIMEO, (char *)oldsnd, &l) == 0;
    if (!have_r) *oldrcv = 0;
    if (!have_s) *oldsnd = 0;
    DWORD v = (DWORD)ms;
    setsockopt(s, SOL_SOCKET, SO_RCVTIMEO, (char *)&v, sizeof(v));
    setsockopt(s, SOL_SOCKET, SO_SNDTIMEO, (char *)&v, sizeof(v));
}

static void hs_timeouts_pop(SOCKET s, DWORD oldrcv, DWORD oldsnd)
{
    setsockopt(s, SOL_SOCKET, SO_RCVTIMEO, (char *)&oldrcv, sizeof(oldrcv));
    setsockopt(s, SOL_SOCKET, SO_SNDTIMEO, (char *)&oldsnd, sizeof(oldsnd));
}

/* ----------------------------------------------------------- handshakes --- */

static int hs_socks5(SOCKET s, const Dest *d, const Proxy *p, DWORD deadline)
{
    unsigned char req[512];
    int n = 0;
    int need_auth = (p->user[0] != 0);

    req[n++] = 5;
    req[n++] = (unsigned char)(need_auth ? 2 : 1);
    req[n++] = 0x00;
    if (need_auth) req[n++] = 0x02;
    if (x_send(s, req, n, deadline) != 0) return -1;

    unsigned char rep[2];
    if (x_recv_exact(s, rep, 2, deadline) != 0) return -2;
    if (rep[0] != 5) return -3;
    if (rep[1] == 0xFF) return -4;
    if (rep[1] == 0x02) {
        unsigned char ar[512];
        int an = 0;
        size_t ul = strlen(p->user), pl = strlen(p->pass);
        if (ul > 255) ul = 255;
        if (pl > 255) pl = 255;
        ar[an++] = 1;
        ar[an++] = (unsigned char)ul;
        memcpy(ar + an, p->user, ul); an += (int)ul;
        ar[an++] = (unsigned char)pl;
        memcpy(ar + an, p->pass, pl); an += (int)pl;
        if (x_send(s, ar, an, deadline) != 0) return -5;
        if (x_recv_exact(s, rep, 2, deadline) != 0) return -6;
        if (rep[1] != 0) return -7;
    } else if (rep[1] != 0x00) {
        return -8;
    }

    unsigned char c[300];
    int m = 0;
    c[m++] = 5;
    c[m++] = 1;
    c[m++] = 0;
    if (d->atyp == 3) {
        size_t dl = strlen(d->domain);
        if (dl > 255) dl = 255;
        c[m++] = 3;
        c[m++] = (unsigned char)dl;
        memcpy(c + m, d->domain, dl);
        m += (int)dl;
    } else if (d->atyp == 1) {
        c[m++] = 1;
        memcpy(c + m, d->ip, 4);
        m += 4;
    } else {
        c[m++] = 4;
        memcpy(c + m, d->ip, 16);
        m += 16;
    }
    c[m++] = (unsigned char)(d->port >> 8);
    c[m++] = (unsigned char)(d->port & 0xff);
    if (x_send(s, c, m, deadline) != 0) return -9;

    unsigned char resp[4];
    if (x_recv_exact(s, resp, 4, deadline) != 0) return -10;
    if (resp[0] != 5) return -11;
    if (resp[1] != 0) return -100 - resp[1];
    int addrlen;
    if (resp[3] == 1) addrlen = 4;
    else if (resp[3] == 4) addrlen = 16;
    else if (resp[3] == 3) {
        unsigned char l;
        if (x_recv_exact(s, &l, 1, deadline) != 0) return -12;
        addrlen = l;
    } else return -13;
    unsigned char skip[260];
    if (x_recv_exact(s, skip, addrlen + 2, deadline) != 0) return -14;
    return 0;
}

static int hs_socks4(SOCKET s, const Dest *d, const Proxy *p, DWORD deadline)
{
    if (d->atyp != 1) return -1;
    unsigned char c[320];
    int m = 0;
    c[m++] = 4;
    c[m++] = 1;
    c[m++] = (unsigned char)(d->port >> 8);
    c[m++] = (unsigned char)(d->port & 0xff);
    memcpy(c + m, d->ip, 4); m += 4;
    size_t ul = strlen(p->user);
    if (ul > 100) ul = 100;
    memcpy(c + m, p->user, ul); m += (int)ul;
    c[m++] = 0;
    if (x_send(s, c, m, deadline) != 0) return -2;
    unsigned char rep[8];
    if (x_recv_exact(s, rep, 8, deadline) != 0) return -3;
    if (rep[1] != 0x5A) return -4;
    return 0;
}

static int hs_http(SOCKET s, const Dest *d, const Proxy *p, DWORD deadline)
{
    char req[1024];
    if (p->user[0]) {
        char cred[256], enc[420];
        _snprintf(cred, sizeof(cred) - 1, "%s:%s", p->user, p->pass);
        cred[sizeof(cred) - 1] = 0;
        b64enc(cred, enc, sizeof(enc));
        _snprintf(req, sizeof(req) - 1,
                  "CONNECT %s HTTP/1.1\r\nHost: %s\r\n"
                  "Proxy-Authorization: Basic %s\r\n"
                  "Proxy-Connection: keep-alive\r\n\r\n",
                  d->text, d->text, enc);
    } else {
        _snprintf(req, sizeof(req) - 1,
                  "CONNECT %s HTTP/1.1\r\nHost: %s\r\n"
                  "Proxy-Connection: keep-alive\r\n\r\n",
                  d->text, d->text);
    }
    req[sizeof(req) - 1] = 0;
    if (x_send(s, req, (int)strlen(req), deadline) != 0) return -1;

    char resp[2048];
    int got = 0;
    while (got < (int)sizeof(resp) - 1) {
        int r = x_recv(s, resp + got, 1, deadline);
        if (r <= 0) return -2;
        got += r;
        resp[got] = 0;
        if (got >= 4 && !memcmp(resp + got - 4, "\r\n\r\n", 4)) break;
        if (got >= 2 && !memcmp(resp + got - 2, "\n\n", 2)) break;
    }
    resp[got] = 0;
    if (!strncmp(resp, "HTTP/1.", 7)) {
        char *sp = strchr(resp, ' ');
        if (sp && atoi(sp + 1) == 200) return 0;
        return -3;
    }
    return -4;
}

/* ------------------------------------------------------ connect plumbing -- */

static int bind_any(SOCKET s, int family)
{
    if (family == AF_INET) {
        struct sockaddr_in a;
        memset(&a, 0, sizeof(a));
        a.sin_family = AF_INET;
        return bind(s, (struct sockaddr *)&a, sizeof(a));
    }
    struct sockaddr_in6 a6;
    memset(&a6, 0, sizeof(a6));
    a6.sin6_family = AF_INET6;
    return bind(s, (struct sockaddr *)&a6, sizeof(a6));
}

static int connect_ex_to(SOCKET s, const struct sockaddr *sa, int len, DWORD deadline)
{
    if (!g_connectex) return -1;

    OVERLAPPED ov;
    memset(&ov, 0, sizeof(ov));
    ov.hEvent = WSACreateEvent();
    if (!ov.hEvent) return -1;

    bind_any(s, sa->sa_family); /* required by ConnectEx; ignore "already bound" */

    DWORD sent = 0;
    BOOL ok = g_connectex(s, sa, len, NULL, 0, &sent, &ov);
    int err = WSAGetLastError();
    if (!ok && (err == WSA_IO_PENDING || err == WSAEWOULDBLOCK)) {
        DWORD left = deadline_left(deadline);
        if (!left) left = 1;
        if (WaitForSingleObject(ov.hEvent, left) == WAIT_OBJECT_0) {
            DWORD flags = 0, bytes = 0;
            if (WSAGetOverlappedResult(s, &ov, &bytes, FALSE, &flags)) ok = TRUE;
        }
    }
    WSACloseEvent(ov.hEvent);
    return ok ? 0 : -1;
}

static int is_stream_socket(SOCKET s)
{
    int type = 0;
    int l = (int)sizeof(type);
    if (getsockopt(s, SOL_SOCKET, SO_TYPE, (char *)&type, &l) != 0) return 1;
    return type == SOCK_STREAM;
}

static int resolve_proxy(Proxy *p, struct sockaddr_storage *ss, int *slen)
{
    DWORD now = GetTickCount();
    if (p->sslen && (DWORD)(now - p->resolved_at) < 300000) {
        memcpy(ss, &p->ss, sizeof(*ss));
        *slen = p->sslen;
        return 0;
    }
    struct sockaddr_storage out;
    int outlen = 0;
    memset(&out, 0, sizeof(out));

    struct in_addr a4;
    struct in6_addr a6;
    if (InetPtonA(AF_INET, p->host, &a4) == 1) {
        struct sockaddr_in *in = (struct sockaddr_in *)&out;
        in->sin_family = AF_INET;
        in->sin_port = htons((u_short)p->port);
        in->sin_addr = a4;
        outlen = (int)sizeof(*in);
    } else if (InetPtonA(AF_INET6, p->host, &a6) == 1) {
        struct sockaddr_in6 *in6 = (struct sockaddr_in6 *)&out;
        in6->sin6_family = AF_INET6;
        in6->sin6_port = htons((u_short)p->port);
        in6->sin6_addr = a6;
        outlen = (int)sizeof(*in6);
    } else {
        struct addrinfo hints, *res = NULL;
        memset(&hints, 0, sizeof(hints));
        hints.ai_family = AF_UNSPEC;
        hints.ai_socktype = SOCK_STREAM;
        char portstr[16];
        _snprintf(portstr, sizeof(portstr) - 1, "%d", p->port);
        if (getaddrinfo(p->host, portstr, &hints, &res) != 0 || !res) return -1;
        struct addrinfo *pick = res;
        for (struct addrinfo *it = res; it; it = it->ai_next)
            if (it->ai_family == AF_INET) { pick = it; break; }
        memcpy(&out, pick->ai_addr, pick->ai_addrlen);
        outlen = (int)pick->ai_addrlen;
        freeaddrinfo(res);
    }
    memcpy(&p->ss, &out, sizeof(out));
    p->sslen = outlen;
    p->resolved_at = now;
    memcpy(ss, &out, sizeof(out));
    *slen = outlen;
    return 0;
}

static int do_proxy_connect(SOCKET s, const Dest *d, int *used_idx, DWORD *elapsed,
                            int *fail_rc)
{
    EnterCriticalSection(&g_cs);
    int n = g_npx;
    int idx = (int)g_cur;
    int retry = g_retry;
    Proxy px;
    memset(&px, 0, sizeof(px));
    if (n > 0) px = g_px[idx];
    LeaveCriticalSection(&g_cs);
    if (n <= 0) { *fail_rc = -99; return -1; }

    DWORD t0 = GetTickCount();
    int maxa = retry ? 2 : 1;
    int last_rc = -1;

    for (int attempt = 0; attempt < maxa; attempt++) {
        struct sockaddr_storage ss;
        int slen = 0;
        if (resolve_proxy(&px, &ss, &slen) != 0) {
            last_rc = -20;
        } else {
            DWORD dl = GetTickCount() + (DWORD)g_timeout_ms;
            if (connect_ex_to(s, (struct sockaddr *)&ss, slen, dl) != 0) {
                last_rc = -21;
            } else {
                DWORD hl = GetTickCount() + (DWORD)g_timeout_ms;
                DWORD orcv, osnd;
                hs_timeouts_push(s, g_timeout_ms, &orcv, &osnd);
                int rc;
                if (px.proto == 0) rc = hs_socks5(s, d, &px, hl);
                else if (px.proto == 2) rc = hs_socks4(s, d, &px, hl);
                else rc = hs_http(s, d, &px, hl);
                hs_timeouts_pop(s, orcv, osnd);
                if (rc == 0) {
                    *used_idx = idx;
                    if (elapsed) *elapsed = GetTickCount() - t0;
                    return 0;
                }
                last_rc = rc;
            }
        }

        if (attempt + 1 >= maxa) break;
        /* advance for the retry and for all future connections */
        EnterCriticalSection(&g_cs);
        if (g_npx > 1) {
            idx = (idx + 1) % g_npx;
            g_cur = idx;
            g_last_rotate = GetTickCount();
            px = g_px[idx];
        }
        LeaveCriticalSection(&g_cs);
        logf_("CONNECT %s proxy failed rc=%d, retry #%d", d->text, last_rc, idx);
        break; /* socket is mid-handshake; let the app retry on a new socket */
    }
    *fail_rc = last_rc;
    if (g_npx > 1 && g_retry) {
        EnterCriticalSection(&g_cs);
        g_cur = (g_cur + 1) % g_npx;
        g_last_rotate = GetTickCount();
        LeaveCriticalSection(&g_cs);
        write_state();
    }
    return -1;
}

static int raw_connect_direct(SOCKET s, const struct sockaddr *name, int namelen)
{
    DWORD deadline = GetTickCount() + 30000;
    if (connect_ex_to(s, name, namelen, deadline) == 0) return 0;
    WSASetLastError(WSAECONNREFUSED);
    return SOCKET_ERROR;
}

/* ------------------------------------------------------------- hooks ------ */

typedef int(WSAAPI *FN_connect)(SOCKET, const struct sockaddr *, int);
typedef int(WSAAPI *FN_WSASendTo)(SOCKET, LPWSABUF, DWORD, LPDWORD, DWORD,
                                  const struct sockaddr *, int, LPWSAOVERLAPPED,
                                  LPWSAOVERLAPPED_COMPLETION_ROUTINE);

typedef struct {
    const char *name;
    void       *target;
    void       *stub;
    unsigned char orig[8];
    int         installed;
} HookSlot;

static HookSlot g_hooks[12];
static int      g_nhooks = 0;
static unsigned char *g_stub_area = NULL;
static int      g_stub_used = 0;

static void *alloc_near(void *target, size_t size)
{
    SYSTEM_INFO si;
    GetSystemInfo(&si);
    uintptr_t gran = si.dwAllocationGranularity ? si.dwAllocationGranularity : 0x10000;
    uintptr_t base = (uintptr_t)target;
    for (uintptr_t off = gran; off < 0x60000000ULL; off += gran) {
        uintptr_t cand[2] = {base + off, (base > off ? base - off : 0)};
        for (int i = 0; i < 2; i++) {
            uintptr_t a = cand[i];
            if (a < 0x10000) continue;
            void *p = VirtualAlloc((void *)a, size, MEM_COMMIT | MEM_RESERVE,
                                   PAGE_EXECUTE_READWRITE);
            if (!p) continue;
            intptr_t delta = (intptr_t)p - (intptr_t)(base + 5);
            if (delta >= (intptr_t)INT32_MIN && delta <= (intptr_t)INT32_MAX) return p;
            VirtualFree(p, 0, MEM_RELEASE);
        }
    }
    return NULL;
}

static void *make_stub(void *fn)
{
    if (!g_stub_area || g_stub_used + 16 > 0x1000) return NULL;
    unsigned char *p = g_stub_area + g_stub_used;
    g_stub_used += 16;
#ifdef _WIN64
    p[0] = 0x48; p[1] = 0xB8;
    *(uint64_t *)(p + 2) = (uint64_t)(uintptr_t)fn;
    p[10] = 0xFF; p[11] = 0xE0;
#else
    p[0] = 0xB8;
    *(uint32_t *)(p + 1) = (uint32_t)(uintptr_t)fn;
    p[5] = 0xFF; p[6] = 0xE0;
#endif
    FlushInstructionCache(GetCurrentProcess(), p, 16);
    return p;
}

static int suspend_others(HANDLE *out, int cap)
{
    int n = 0;
    HANDLE snap = CreateToolhelp32Snapshot(TH32CS_SNAPTHREAD, 0);
    if (snap == INVALID_HANDLE_VALUE) return 0;
    THREADENTRY32 te;
    memset(&te, 0, sizeof(te));
    te.dwSize = sizeof(te);
    DWORD me = GetCurrentThreadId();
    if (Thread32First(snap, &te)) {
        do {
            if (te.th32OwnerProcessID != g_pid || te.th32ThreadID == me) {
                memset(&te, 0, sizeof(te));
                te.dwSize = sizeof(te);
                continue;
            }
            if (n >= cap) break;
            HANDLE th = OpenThread(THREAD_SUSPEND_RESUME, FALSE, te.th32ThreadID);
            if (th) {
                if (SuspendThread(th) != (DWORD)-1) out[n++] = th;
                else CloseHandle(th);
            }
            memset(&te, 0, sizeof(te));
            te.dwSize = sizeof(te);
        } while (Thread32Next(snap, &te));
    }
    CloseHandle(snap);
    return n;
}

static void resume_handles(HANDLE *h, int n)
{
    for (int i = 0; i < n; i++) {
        ResumeThread(h[i]);
        CloseHandle(h[i]);
    }
}

static int patch_bytes(void *target, const unsigned char *bytes, int len, int suspend)
{
    DWORD old = 0;
    if (!VirtualProtect(target, (SIZE_T)len, PAGE_EXECUTE_READWRITE, &old)) return -1;
    HANDLE th[600];
    int nt = suspend ? suspend_others(th, 600) : 0;
    memcpy(target, bytes, (size_t)len);
    FlushInstructionCache(GetCurrentProcess(), target, (size_t)len);
    if (suspend) resume_handles(th, nt);
    DWORD tmp = 0;
    VirtualProtect(target, (SIZE_T)len, old, &tmp);
    return 0;
}

static int build_patch(void *target, void *stub, unsigned char *out)
{
    intptr_t delta = (intptr_t)stub - ((intptr_t)target + 5);
    if (delta < (intptr_t)INT32_MIN || delta > (intptr_t)INT32_MAX) return -1;
    int32_t d32 = (int32_t)delta;
    out[0] = 0xE9;
    memcpy(out + 1, &d32, 4);
    return 0;
}

static int hook_install(const char *name, void *myfn)
{
    HMODULE ws = GetModuleHandleA("ws2_32.dll");
    if (!ws) ws = LoadLibraryA("ws2_32.dll");
    if (!ws) return -1;
    if (!g_stub_area) {
        g_stub_area = (unsigned char *)alloc_near((void *)ws, 0x1000);
        if (!g_stub_area) {
            logf_("INIT no near page for stubs; hooks disabled");
            return -1;
        }
    }
    void *target = (void *)GetProcAddress(ws, name);
    if (!target) return -1;
    void *stub = make_stub(myfn);
    if (!stub) return -1;

    HookSlot *h = &g_hooks[g_nhooks];
    h->name = name;
    h->target = target;
    h->stub = stub;
    memcpy(h->orig, target, 5);

    unsigned char patch[5];
    if (build_patch(target, stub, patch) != 0) return -1;
    if (patch_bytes(target, patch, 5, 1) != 0) return -1;
    h->installed = 1;
    g_nhooks++;
    return 0;
}

/* ---- IAT rewrite: point every module's WSAIoctl import at our thunk ---- */

static void iat_patch_module(HMODULE mod, const char *dllname, const char *fn,
                             void *replace, int *count)
{
    if (!mod) return;
    unsigned char *b = (unsigned char *)mod;
    IMAGE_DOS_HEADER *dos = (IMAGE_DOS_HEADER *)b;
    if (dos->e_magic != IMAGE_DOS_SIGNATURE) return;
    IMAGE_NT_HEADERS *nt = (IMAGE_NT_HEADERS *)(b + dos->e_lfanew);
    if (nt->Signature != IMAGE_NT_SIGNATURE) return;

    DWORD rva = nt->OptionalHeader.DataDirectory[IMAGE_DIRECTORY_ENTRY_IMPORT].VirtualAddress;
    if (!rva) return;
    IMAGE_IMPORT_DESCRIPTOR *imp = (IMAGE_IMPORT_DESCRIPTOR *)(b + rva);

    for (; imp->Name; imp++) {
        const char *mod_name = (const char *)(b + imp->Name);
        if (_stricmp(mod_name, dllname) != 0) continue;
        IMAGE_THUNK_DATA *orig = (IMAGE_THUNK_DATA *)(b + imp->OriginalFirstThunk);
        IMAGE_THUNK_DATA *first = (IMAGE_THUNK_DATA *)(b + imp->FirstThunk);
        if (!orig || !first) continue;
        for (int i = 0; orig[i].u1.AddressOfData; i++) {
            if (IMAGE_SNAP_BY_ORDINAL(orig[i].u1.Ordinal)) continue;
            IMAGE_IMPORT_BY_NAME *ibn =
                (IMAGE_IMPORT_BY_NAME *)(b + orig[i].u1.AddressOfData);
            if (strcmp((const char *)ibn->Name, fn) != 0) continue;
            void **slot = (void **)&first[i];
            if (*slot == replace) continue;
            DWORD old = 0;
            if (VirtualProtect(slot, sizeof(void *), PAGE_READWRITE, &old)) {
                *slot = replace;
                DWORD tmp = 0;
                VirtualProtect(slot, sizeof(void *), old, &tmp);
                (*count)++;
            }
        }
    }
}

static void iat_sweep(void)
{
    HANDLE snap = CreateToolhelp32Snapshot(
        TH32CS_SNAPMODULE | TH32CS_SNAPMODULE32, g_pid);
    if (snap == INVALID_HANDLE_VALUE) return;
    MODULEENTRY32W me;
    memset(&me, 0, sizeof(me));
    me.dwSize = sizeof(me);
    int count = 0;
    if (Module32FirstW(snap, &me)) {
        do {
            HMODULE m = (HMODULE)me.modBaseAddr;
            if (m == (HMODULE)g_self) {
                memset(&me, 0, sizeof(me));
                me.dwSize = sizeof(me);
                continue;
            }
            iat_patch_module(m, "ws2_32.dll", "WSAIoctl", (void *)&hooked_WSAIoctl,
                             &count);
            memset(&me, 0, sizeof(me));
            me.dwSize = sizeof(me);
        } while (Module32NextW(snap, &me));
    }
    CloseHandle(snap);
    if (count) logf_("IAT patched %d WSAIoctl slot(s)", count);
}

/* ------------------------------------------------------------ API hooks --- */

static int WSAAPI hooked_connect(SOCKET s, const struct sockaddr *name, int namelen)
{
    if (g_ready && g_enabled && g_npx > 0 && is_stream_socket(s)) {
        Dest d;
        if (dest_from_sockaddr(name, namelen, &d) && !dest_is_local(&d)) {
            maybe_reload();
            /* fake-ip / TUN setups: talk to the proxy by hostname, not by the
             * synthetic address the app was handed. */
            char hn[200];
            if (map_lookup(d.ip, d.atyp, hn, sizeof(hn))) {
                d.atyp = 3;
                _snprintf(d.domain, sizeof(d.domain) - 1, "%s", hn);
                _snprintf(d.text, sizeof(d.text) - 1, "%s:%d", hn, d.port);
            }
            int idx = -1, rc_fail = 0;
            DWORD ms = 0;
            if (do_proxy_connect(s, &d, &idx, &ms, &rc_fail) == 0) {
                track_socket(s);
                logf_("CONNECT %s via #%d ok %lums", d.text, idx, (unsigned long)ms);
                return 0;
            }
            if (!g_fail_closed) {
                logf_("CONNECT %s proxy rc=%d -> DIRECT (fail_open)", d.text, rc_fail);
                return raw_connect_direct(s, name, namelen);
            }
            logf_("CONNECT %s proxy rc=%d -> BLOCKED", d.text, rc_fail);
            WSASetLastError(WSAECONNREFUSED);
            return SOCKET_ERROR;
        }
    }
    return raw_connect_direct(s, name, namelen);
}

static int WSAAPI hooked_WSAConnect(SOCKET s, const struct sockaddr *name, int namelen,
                                    LPWSABUF a, LPWSABUF b, LPQOS c, LPQOS d)
{
    (void)a; (void)b; (void)c; (void)d;
    return hooked_connect(s, name, namelen);
}

static BOOL WINAPI hooked_ConnectEx(SOCKET s, const struct sockaddr *name, int namelen,
                                    PVOID buf, DWORD len, LPDWORD sent,
                                    LPOVERLAPPED ov)
{
    (void)buf; (void)len;
    if (sent) *sent = 0;
    int crc = hooked_connect(s, name, namelen);
    if (crc == 0) {
        if (ov) {
            ov->Internal = 0;
            ov->InternalHigh = 0;
            if (ov->hEvent) SetEvent(ov->hEvent);
        }
        return TRUE;
    }
    return FALSE;
}

static int udp_allowed(const struct sockaddr *to, int tolen)
{
    if (!g_block_udp) return 1;
    Dest d;
    if (!dest_from_sockaddr(to, tolen, &d)) return 1;
    if (dest_is_local(&d)) return 1;
    if (g_dns_udp && d.port == 53) return 1;
    return 0;
}

static int raw_sendto_msg(SOCKET s, LPWSABUF bufs, DWORD nbufs, int flags,
                          const struct sockaddr *to, int tolen, DWORD *sent)
{
    WSAMSG msg;
    memset(&msg, 0, sizeof(msg));
    msg.name = (LPSOCKADDR)to;
    msg.namelen = tolen;
    msg.lpBuffers = bufs;
    msg.dwBufferCount = nbufs;
    DWORD bytes = 0;
    int rc = WSASendMsg(s, &msg, (DWORD)flags, &bytes, NULL, NULL);
    if (sent) *sent = bytes;
    return rc;
}

static int WSAAPI hooked_sendto(SOCKET s, const char *buf, int len, int flags,
                                const struct sockaddr *to, int tolen)
{
    if (!g_ready || udp_allowed(to, tolen)) {
        WSABUF b;
        b.buf = (char *)buf;
        b.len = (DWORD)len;
        DWORD sent = 0;
        if (raw_sendto_msg(s, &b, 1, flags, to, tolen, &sent) == 0) return (int)sent;
        return SOCKET_ERROR;
    }
    static LONG once = 0;
    if (InterlockedIncrement(&once) < 8)
        logf_("UDP blocked (%d bytes) - app should fall back to TCP", len);
    WSASetLastError(WSAEACCES);
    return SOCKET_ERROR;
}

static int WSAAPI hooked_WSASendTo(SOCKET s, LPWSABUF bufs, DWORD nbufs, LPDWORD sent,
                                   DWORD flags, const struct sockaddr *to, int tolen,
                                   LPWSAOVERLAPPED ov,
                                   LPWSAOVERLAPPED_COMPLETION_ROUTINE fn)
{
    (void)ov; (void)fn;
    if (!g_ready || udp_allowed(to, tolen)) {
        DWORD bytes = 0;
        if (raw_sendto_msg(s, bufs, nbufs, (int)flags, to, tolen, &bytes) == 0) {
            if (sent) *sent = bytes;
            return 0;
        }
        return SOCKET_ERROR;
    }
    WSASetLastError(WSAEACCES);
    return SOCKET_ERROR;
}

static int WSAAPI hooked_WSAIoctl(SOCKET s, DWORD code, LPVOID in, DWORD inlen,
                                  LPVOID out, DWORD outlen, LPDWORD ret,
                                  LPWSAOVERLAPPED ov,
                                  LPWSAOVERLAPPED_COMPLETION_ROUTINE fn)
{
    if (g_hook_connectex && code == SIO_GET_EXTENSION_FUNCTION_POINTER && in &&
        inlen >= sizeof(GUID) && out && outlen >= sizeof(void *)) {
        if (!memcmp(in, &k_connectex_guid, sizeof(GUID))) {
            *(void **)out = (void *)&hooked_ConnectEx;
            if (ret) *ret = sizeof(void *);
            return 0;
        }
    }
    static int idx = -1;
    (void)idx;
    if (!g_real_wsaioc) {
        WSASetLastError(WSAEINVAL);
        return SOCKET_ERROR;
    }
    return g_real_wsaioc(s, code, in, inlen, out, outlen, ret, ov, fn);
}

/* ---- resolver observation: remember which name produced which address ---- */

static INT(WSAAPI *g_clean_gai)(PCSTR, PCSTR, const ADDRINFOA *, PADDRINFOA *) = NULL;
static INT(WSAAPI *g_clean_gaiw)(PCWSTR, PCWSTR, const ADDRINFOW *, PADDRINFOW *) = NULL;
static INT(WSAAPI *g_clean_gaiexw)(PCWSTR, PCWSTR, DWORD, LPGUID, const ADDRINFOEXW *,
                                   PADDRINFOEXW *, PTIMEVAL, LPOVERLAPPED,
                                   LPLOOKUPSERVICE_COMPLETION_ROUTINE,
                                   LPHANDLE) = NULL;
static struct hostent *(WSAAPI *g_clean_ghbn)(const char *) = NULL;
static void(WSAAPI *g_clean_freeexw)(PADDRINFOEXW) = NULL;

static int wide_to_utf8(const wchar_t *w, char *out, int cap)
{
    if (!w || !w[0]) return 0;
    int n = WideCharToMultiByte(CP_UTF8, 0, w, -1, out, cap, NULL, NULL);
    if (n <= 0) { out[0] = 0; return 0; }
    return n - 1;
}

static int name_is_numeric(const char *name)
{
    struct in_addr a4;
    struct in6_addr a6;
    return InetPtonA(AF_INET, name, &a4) == 1 || InetPtonA(AF_INET6, name, &a6) == 1;
}

static void record_ai_a(const char *name, const ADDRINFOA *res)
{
    for (const ADDRINFOA *p = res; p; p = p->ai_next) {
        if (p->ai_family == AF_INET && p->ai_addr)
            map_add((const unsigned char *)&((const struct sockaddr_in *)p->ai_addr)->sin_addr,
                    1, name);
        else if (p->ai_family == AF_INET6 && p->ai_addr)
            map_add((const unsigned char *)&((const struct sockaddr_in6 *)p->ai_addr)->sin6_addr,
                    4, name);
    }
}

static void record_ai_w(const char *name, const ADDRINFOW *res)
{
    for (const ADDRINFOW *p = res; p; p = p->ai_next) {
        if (p->ai_family == AF_INET && p->ai_addr)
            map_add((const unsigned char *)&((const struct sockaddr_in *)p->ai_addr)->sin_addr,
                    1, name);
        else if (p->ai_family == AF_INET6 && p->ai_addr)
            map_add((const unsigned char *)&((const struct sockaddr_in6 *)p->ai_addr)->sin6_addr,
                    4, name);
    }
}

static INT WSAAPI hooked_getaddrinfo(PCSTR name, PCSTR svc, const ADDRINFOA *hints,
                                     PADDRINFOA *res)
{
    if (!g_clean_gai) return EAI_FAIL;

    INT rc = g_clean_gai(name, svc, hints, res);

    if (rc == 0 && name && res && *res && !name_is_numeric(name))
        record_ai_a(name, *res);
    return rc;
}

static INT WSAAPI hooked_GetAddrInfoW(PCWSTR name, PCWSTR svc, const ADDRINFOW *hints,
                                      PADDRINFOW *res)
{
    if (!g_clean_gaiw) return EAI_FAIL;
    INT rc = g_clean_gaiw(name, svc, hints, res);
    if (rc == 0 && name && res && *res) {
        char nb[256];
        if (wide_to_utf8(name, nb, sizeof(nb)) > 0 && !name_is_numeric(nb))
            record_ai_w(nb, *res);
    }
    return rc;
}

/* Our own ADDRINFOEXW blocks (async callers). Kept in a registry so the free
 * hook can tell them apart from lists produced by the system resolver. */
#define MAX_OWN 256
static void *g_own[MAX_OWN];
static int   g_ownn = 0;

static void own_add(void *p)
{
    EnterCriticalSection(&g_cs);
    if (g_ownn < MAX_OWN) g_own[g_ownn++] = p;
    LeaveCriticalSection(&g_cs);
}

static int own_take(void *p)
{
    int hit = 0;
    EnterCriticalSection(&g_cs);
    for (int i = 0; i < g_ownn; i++) {
        if (g_own[i] == p) {
            g_own[i] = g_own[--g_ownn];
            hit = 1;
            break;
        }
    }
    LeaveCriticalSection(&g_cs);
    return hit;
}

static void free_own_exw(PADDRINFOEXW p)
{
    while (p) {
        PADDRINFOEXW n = p->ai_next;
        HeapFree(GetProcessHeap(), 0, p);
        p = n;
    }
}

/* build an ADDRINFOEXW chain from an ADDRINFOA chain, one block per node */
static PADDRINFOEXW conv_to_exw(const ADDRINFOA *a)
{
    PADDRINFOEXW head = NULL, tail = NULL;
    for (const ADDRINFOA *p = a; p; p = p->ai_next) {
        int cnlen = 0;
        if (p->ai_canonname)
            cnlen = MultiByteToWideChar(CP_ACP, 0, p->ai_canonname, -1, NULL, 0);
        SIZE_T cb = sizeof(ADDRINFOEXW) + p->ai_addrlen + (SIZE_T)cnlen * sizeof(wchar_t);
        BYTE *blk = (BYTE *)HeapAlloc(GetProcessHeap(), HEAP_ZERO_MEMORY, cb);
        if (!blk) break;
        PADDRINFOEXW e = (PADDRINFOEXW)blk;
        e->ai_flags = p->ai_flags;
        e->ai_family = p->ai_family;
        e->ai_socktype = p->ai_socktype;
        e->ai_protocol = p->ai_protocol;
        e->ai_addrlen = p->ai_addrlen;
        e->ai_addr = (struct sockaddr *)(blk + sizeof(ADDRINFOEXW));
        if (p->ai_addr && p->ai_addrlen) memcpy(e->ai_addr, p->ai_addr, p->ai_addrlen);
        if (cnlen > 0) {
            wchar_t *w = (wchar_t *)(blk + sizeof(ADDRINFOEXW) + p->ai_addrlen);
            MultiByteToWideChar(CP_ACP, 0, p->ai_canonname, -1, w, cnlen);
            e->ai_canonname = w;
        }
        if (tail) tail->ai_next = e; else head = e;
        tail = e;
    }
    return head;
}

static INT WSAAPI hooked_GetAddrInfoExW(PCWSTR name, PCWSTR svc, DWORD ns, LPGUID prov,
                                        const ADDRINFOEXW *hints, PADDRINFOEXW *res,
                                        PTIMEVAL timeout, LPOVERLAPPED ov,
                                        LPLOOKUPSERVICE_COMPLETION_ROUTINE cb,
                                        LPHANDLE handle)
{
    if (!g_clean_gaiexw) return EAI_FAIL;
    if (!ov) return g_clean_gaiexw(name, svc, ns, prov, hints, res, timeout, ov, cb, handle);

    /* Async callers: the duplicated ws2_32 never completes its overlapped, so
     * resolve synchronously and hand back a finished list. */
    char nb[256];
    int have_name = 0;
    if (name && wide_to_utf8(name, nb, sizeof(nb)) > 0 && !name_is_numeric(nb))
        have_name = 1;

    char svcb[32] = "";
    if (svc) wide_to_utf8(svc, svcb, sizeof(svcb));

    ADDRINFOA h;
    memset(&h, 0, sizeof(h));
    if (hints) {
        h.ai_flags = hints->ai_flags;
        h.ai_family = hints->ai_family;
        h.ai_socktype = hints->ai_socktype;
        h.ai_protocol = hints->ai_protocol;
    }
    PADDRINFOA a = NULL;
    int rc = g_clean_gai(name ? nb : NULL, svcb[0] ? svcb : NULL,
                         hints ? &h : NULL, &a);
    if (rc != 0) {
        WSASetLastError(rc == EAI_MEMORY ? WSAENOBUFS : WSAHOST_NOT_FOUND);
        return WSAHOST_NOT_FOUND;
    }
    if (have_name) record_ai_a(nb, a);

    if (res) {
        PADDRINFOEXW ex = conv_to_exw(a);
        if (ex) own_add(ex);
        *res = ex;
    }
    freeaddrinfo(a);

    if (ov) {
        ov->Internal = 0;
        ov->InternalHigh = 0;
        if (ov->hEvent) SetEvent(ov->hEvent);
    }
    return 0; /* completed immediately */
}

static void WSAAPI hooked_FreeAddrInfoExW(PADDRINFOEXW p)
{
    if (!p) return;
    if (own_take(p)) {
        free_own_exw(p);
        return;
    }
    if (g_clean_freeexw) g_clean_freeexw(p);
}
static struct hostent *WSAAPI hooked_gethostbyname(const char *name)
{
    if (!g_clean_ghbn) { WSASetLastError(WSANOTINITIALISED); return NULL; }
    struct hostent *he = g_clean_ghbn(name);
    if (he && name && !name_is_numeric(name)) {
        for (int i = 0; he->h_addr_list && he->h_addr_list[i]; i++)
            map_add((const unsigned char *)he->h_addr_list[i], he->h_length == 16 ? 4 : 1,
                    name);
    }
    return he;
}

/* Load a private copy of ws2_32 so the original resolver stays callable. */
static int load_clean_ws2(void)
{
    wchar_t tmp[MAX_PATH], dir[MAX_PATH], copy[MAX_PATH];
    if (!GetTempPathW(MAX_PATH, tmp)) return -1;
    _snwprintf(dir, MAX_PATH - 1, L"%lsproxyinject", tmp);
    dir[MAX_PATH - 1] = 0;
    CreateDirectoryW(dir, NULL);
    _snwprintf(copy, MAX_PATH - 1, L"%ls\\ws2_32_%lu.dll", dir,
               (unsigned long)g_pid);
    copy[MAX_PATH - 1] = 0;

    if (!CopyFileW(L"C:\\Windows\\System32\\ws2_32.dll", copy, FALSE)) return -1;
    HMODULE m = LoadLibraryW(copy);
    if (!m) return -1;

    typedef INT(WSAAPI * PFN_SU)(WORD, LPWSADATA);
    PFN_SU su = (PFN_SU)GetProcAddress(m, "WSAStartup");
    WSADATA wd;
    if (!su || su(MAKEWORD(2, 2), &wd) != 0) return -1;

    g_clean_gai = (void *)GetProcAddress(m, "getaddrinfo");
    g_clean_gaiw = (void *)GetProcAddress(m, "GetAddrInfoW");
    g_clean_gaiexw = (void *)GetProcAddress(m, "GetAddrInfoExW");
    g_clean_ghbn = (void *)GetProcAddress(m, "gethostbyname");
    g_clean_freeexw = (void *)GetProcAddress(m, "FreeAddrInfoExW");
    return (g_clean_gai != NULL) ? 0 : -1;
}

/* ------------------------------------------------------------- bootstrap -- */

static int has_config_w(const wchar_t *dir)
{
    wchar_t p[MAX_PATH];
    _snwprintf(p, MAX_PATH - 1, L"%ls\\config.ini", dir);
    p[MAX_PATH - 1] = 0;
    DWORD a = GetFileAttributesW(p);
    return a != INVALID_FILE_ATTRIBUTES && !(a & FILE_ATTRIBUTE_DIRECTORY);
}

static void pick_data_dir(wchar_t *out, DWORD cap, const wchar_t *dll_dir)
{
    wchar_t env[MAX_PATH], parent[MAX_PATH];
    DWORD n = GetEnvironmentVariableW(L"PROXYINJECT_DIR", env, MAX_PATH);
    if (n > 0 && n < MAX_PATH && has_config_w(env)) {
        _snwprintf(out, cap - 1, L"%ls", env);
        out[cap - 1] = 0;
        return;
    }
    if (has_config_w(dll_dir)) {
        _snwprintf(out, cap - 1, L"%ls", dll_dir);
        out[cap - 1] = 0;
        return;
    }
    _snwprintf(parent, MAX_PATH - 1, L"%ls", dll_dir);
    parent[MAX_PATH - 1] = 0;
    {   /* bin\proxyinject.dll -> repository root */
        wchar_t *slash = wcsrchr(parent, L'\\');
        if (slash && slash != parent) *slash = 0;
    }
    if (has_config_w(parent)) {
        _snwprintf(out, cap - 1, L"%ls", parent);
        out[cap - 1] = 0;
        return;
    }
    _snwprintf(out, cap - 1, L"%ls", dll_dir);
    out[cap - 1] = 0;
}

static void rotate_internal(const char *why)
{
    if (g_npx <= 0) return;
    EnterCriticalSection(&g_cs);
    g_cur = (g_cur + 1) % g_npx;
    g_last_rotate = GetTickCount();
    Proxy p = g_px[g_cur];
    long idx = g_cur;
    LeaveCriticalSection(&g_cs);
    logf_("ROTATE (%s) -> #%ld %s:%d proto=%d", why, idx, p.host, p.port, p.proto);
    kill_tracked_sockets();
    write_state();
}

static DWORD WINAPI worker(LPVOID param)
{
    (void)param;
    g_pid = GetCurrentProcessId();
    {   /* tell the injector we are fully armed before the app gets to run */
        wchar_t evn[64];
        _snwprintf(evn, 63, L"Local\\proxyinject-ready-%lu", (unsigned long)g_pid);
        evn[63] = 0;
        g_ready_evt = OpenEventW(EVENT_MODIFY_STATE, FALSE, evn);
    }

    HMODULE hinst = (HMODULE)g_self;
    wchar_t mod[MAX_PATH];
    if (!GetModuleFileNameW(hinst, mod, MAX_PATH)) mod[0] = 0;
    wchar_t *slash = wcsrchr(mod, L'\\');
    if (slash) *slash = 0;
    pick_data_dir(g_dirw, MAX_PATH, mod);
    if (!WideCharToMultiByte(CP_UTF8, 0, g_dirw, -1, g_dir, MAX_PATH - 1, NULL, NULL))
        g_dir[0] = 0;
    g_dir[MAX_PATH - 1] = 0;

    char logs[MAX_PATH + 8];
    _snprintf(logs, sizeof(logs) - 1, "%s\\logs", g_dir);
    CreateDirectoryA(logs, NULL);
    _snprintf(g_logpath, sizeof(g_logpath) - 1, "%s\\logs\\%lu.log", g_dir,
              (unsigned long)g_pid);

    InitializeCriticalSection(&g_logcs);
    g_logcs_ok = 1;
    InitializeCriticalSection(&g_cs);
    g_cs_ok = 1;

    WSADATA wsa;
    WSAStartup(MAKEWORD(2, 2), &wsa);

    EnterCriticalSection(&g_cs);
    load_config_locked(1);
    LeaveCriticalSection(&g_cs);

    {   /* baseline signatures so the first connect does not trigger a reload */
        wchar_t cfgp[MAX_PATH], pxp[MAX_PATH];
        _snwprintf(cfgp, MAX_PATH - 1, L"%ls\\config.ini", g_dirw);
        cfgp[MAX_PATH - 1] = 0;
        _snwprintf(pxp, MAX_PATH - 1, L"%ls\\proxies.txt", g_dirw);
        pxp[MAX_PATH - 1] = 0;
        g_sig_cfg = file_sig(cfgp);
        g_sig_px = file_sig(pxp);
        g_last_reload_check = GetTickCount();
    }

    char forced[512];
    DWORD fnlen = GetEnvironmentVariableA("PROXYINJECT_FORCE", forced, sizeof(forced));
    if (fnlen > 0 && fnlen < sizeof(forced)) {
        Proxy p;
        if (parse_proxy_spec(forced, &p)) {
            EnterCriticalSection(&g_cs);
            g_px[0] = p;
            g_npx = 1;
            g_cur = 0;
            g_enabled = 1;
            g_forced = 1;
            LeaveCriticalSection(&g_cs);
            logf_("INIT forced proxy %s:%d (selftest)", p.host, p.port);
        }
    }
    if (GetEnvironmentVariableA("PROXYINJECT_LOG", forced, sizeof(forced)) > 0)
        g_log_on = atoi(forced);

    logf_("INIT dir=%s proxies=%d current=%ld enabled=%d", g_dir, g_npx,
          (long)g_cur, g_enabled);

    /* capture the real ConnectEx and WSAIoctl before patching anything */
    HMODULE ws = GetModuleHandleA("ws2_32.dll");
    if (!ws) ws = LoadLibraryA("ws2_32.dll");
    g_real_wsaioc = (void *)GetProcAddress(ws, "WSAIoctl");

    if (load_clean_ws2() == 0)
        logf_("INIT clean resolver loaded (fake-ip compatible)");
    else
        logf_("INIT WARNING: clean resolver unavailable, DNS hooks disabled");

    SOCKET tmp = WSASocketW(AF_INET, SOCK_STREAM, IPPROTO_TCP, NULL, 0,
                            WSA_FLAG_OVERLAPPED);
    if (tmp != INVALID_SOCKET && g_real_wsaioc) {
        DWORD bytes = 0;
        g_real_wsaioc(tmp, SIO_GET_EXTENSION_FUNCTION_POINTER,
                      (void *)&k_connectex_guid, sizeof(GUID), &g_connectex,
                      sizeof(g_connectex), &bytes, NULL, NULL);
        closesocket(tmp);
    }
    if (!g_connectex) logf_("INIT WARNING: real ConnectEx unavailable");

    hook_install("connect", (void *)&hooked_connect);
    hook_install("WSAConnect", (void *)&hooked_WSAConnect);
    hook_install("sendto", (void *)&hooked_sendto);
    hook_install("WSASendTo", (void *)&hooked_WSASendTo);
    if (g_clean_gai) {
        hook_install("getaddrinfo", (void *)&hooked_getaddrinfo);
        hook_install("GetAddrInfoW", (void *)&hooked_GetAddrInfoW);
        hook_install("GetAddrInfoExW", (void *)&hooked_GetAddrInfoExW);
        hook_install("gethostbyname", (void *)&hooked_gethostbyname);
        hook_install("FreeAddrInfoExW", (void *)&hooked_FreeAddrInfoExW);
    }

    g_ready = 1;
    logf_("INIT %d inline hooks installed", g_nhooks);
    for (int i = 0; i < g_nhooks; i++) {
        volatile unsigned char *b = (volatile unsigned char *)g_hooks[i].target;
        logf_("HOOK %s target=%p stub=%p bytes=%02X %02X %02X", g_hooks[i].name,
              g_hooks[i].target, g_hooks[i].stub, b[0], b[1], b[2]);
    }

    iat_sweep();
    write_state();

    if (g_ready_evt) {
        SetEvent(g_ready_evt);
        CloseHandle(g_ready_evt);
        g_ready_evt = NULL;
    }

    DWORD last_sweep = GetTickCount();
    for (;;) {
        Sleep(250);
        maybe_reload();
        if (g_enabled && g_rotate_sec > 0 && g_npx > 1) {
            DWORD now = GetTickCount();
            if ((DWORD)(now - g_last_rotate) >= (DWORD)g_rotate_sec * 1000)
                rotate_internal("timer");
        }
        if ((DWORD)(GetTickCount() - last_sweep) > 3000) {
            last_sweep = GetTickCount();
            iat_sweep();
        }
    }
    return 0;
}

BOOL WINAPI DllMain(HINSTANCE inst, DWORD reason, LPVOID reserved)
{
    (void)reserved;
    if (reason == DLL_PROCESS_ATTACH) {
        g_self = inst;
        DisableThreadLibraryCalls(inst);
        HANDLE t = CreateThread(NULL, 0, worker, NULL, 0, NULL);
        if (t) CloseHandle(t);
    }
    return TRUE;
}

__declspec(dllexport) DWORD ProxyInjectVersion(void) { return 2; }
