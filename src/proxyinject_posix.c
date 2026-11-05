/*
 * proxyinject_posix.c - LD_PRELOAD / DYLD_INSERT_LIBRARIES shim.
 *
 * Hooks connect() and getaddrinfo() in the target process only. The socket the
 * application talks to is the socket to the proxy (SOCKS5 / SOCKS4 / HTTP
 * CONNECT), so TCP traffic goes through the exit node while the rest of the
 * machine keeps its normal route.
 *
 * Launched as:
 *   LD_PRELOAD=/path/proxyinject.so PROXYINJECT_DIR=/path app [args]
 *   DYLD_INSERT_LIBRARIES=/path/proxyinject.dylib PROXYINJECT_DIR=/path app [args]
 *
 * Files, shared with the Windows build:
 *   proxies.db        SQLite database, read-only here, re-read on change:
 *                     table proxies(id, text) is the list,
 *                     table settings(key, value) is the behaviour
 *   state/<pid>.txt   what this process applies
 *   logs/<pid>.log    event log
 *
 * SQLite is compiled in with hidden symbols so it never interposes the
 * target's own libsqlite3. Build:
 *   zig cc -target x86_64-linux-gnu -O2 -shared -fPIC \
 *     -DSQLITE_API='__attribute__((visibility("hidden")))' -DSQLITE_THREADSAFE=0 \
 *     -o proxyinject.so src/proxyinject_posix.c src/sqlite/sqlite3.c -ldl
 */

#define _GNU_SOURCE 1

#include <arpa/inet.h>
#include <ctype.h>
#include <dlfcn.h>
#include <errno.h>
#include <fcntl.h>
#include <netdb.h>
#include <netinet/in.h>
#include <poll.h>
#include <pthread.h>
#include <stdarg.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <strings.h>
#include <sys/socket.h>
#include <sys/stat.h>
#include "sqlite/sqlite3.h"
#include <sys/types.h>
#include <time.h>
#include <unistd.h>

#ifndef RTLD_NEXT
#define RTLD_NEXT ((void *)-1L)
#endif
#ifndef MSG_NOSIGNAL
#define MSG_NOSIGNAL 0
#endif

#define MAX_PROXIES 512
#define MAX_DIR 512
#define MAX_HOST 256
#define MAP_SLOTS 512

enum { PROTO_SOCKS5 = 0, PROTO_HTTP = 1, PROTO_SOCKS4 = 2 };

typedef struct {
  int proto;
  char host[MAX_HOST];
  int port;
  char user[128];
  char pass[128];
} Proxy;

typedef struct {
  int used;
  int family;
  unsigned char addr[16];
  char host[MAX_HOST];
  uint64_t stamp;
} HostMap;

static int (*real_connect)(int, const struct sockaddr *, socklen_t) = NULL;
static int (*real_getaddrinfo)(const char *, const char *, const struct addrinfo *,
                               struct addrinfo **) = NULL;
static int (*real_close)(int) = NULL;

static Proxy g_px[MAX_PROXIES];
static int g_npx = 0;
static int g_cur = 0;
static int g_enabled = 1;
static int g_fail_closed = 1;
static int g_block_udp = 1;
static int g_allow_dns_udp = 1;
static int g_rotate_sec = 0;
static int g_retry_next = 1;
static int g_timeout_ms = 10000;
static int g_log_on = 1;
static int g_forced = 0;

static char g_dir[MAX_DIR] = ".";
static char g_logpath[MAX_DIR + 32];
static pid_t g_pid = 0;
static uint64_t g_last_rotate = 0;
static uint64_t g_last_check = 0;
static unsigned long g_sig_px = 0;

static pthread_mutex_t g_lock = PTHREAD_MUTEX_INITIALIZER;
static pthread_mutex_t g_loglock = PTHREAD_MUTEX_INITIALIZER;
static HostMap g_map[MAP_SLOTS];
static int g_map_slot = 0;
static int g_initialized = 0;

/* ------------------------------------------------------------------ utils -- */

static uint64_t now_ms(void)
{
  struct timespec ts;
  clock_gettime(CLOCK_MONOTONIC, &ts);
  return (uint64_t)ts.tv_sec * 1000u + (uint64_t)(ts.tv_nsec / 1000000);
}

static void trim(char *s)
{
  char *p = s;
  while (*p && isspace((unsigned char)*p)) p++;
  if (p != s) memmove(s, p, strlen(p) + 1);
  size_t n = strlen(s);
  while (n && isspace((unsigned char)s[n - 1])) s[--n] = 0;
}

static void logf_(const char *fmt, ...)
{
  if (!g_log_on || !g_logpath[0]) return;
  pthread_mutex_lock(&g_loglock);
  FILE *f = fopen(g_logpath, "a");
  if (f) {
    struct timespec ts;
    clock_gettime(CLOCK_REALTIME, &ts);
    struct tm tm;
    localtime_r(&ts.tv_sec, &tm);
    fprintf(f, "%02d:%02d:%02d.%03ld ", tm.tm_hour, tm.tm_min, tm.tm_sec,
            (long)(ts.tv_nsec / 1000000));
    va_list ap;
    va_start(ap, fmt);
    vfprintf(f, fmt, ap);
    va_end(ap);
    fputc('\n', f);
    fclose(f);
  }
  pthread_mutex_unlock(&g_loglock);
}

static unsigned long file_sig(const char *path)
{
  struct stat st;
  if (stat(path, &st) != 0) return 0;
  return (unsigned long)st.st_mtime ^ ((unsigned long)st.st_size << 16);
}

static int read_file(const char *path, char *buf, size_t cap)
{
  FILE *f = fopen(path, "rb");
  if (!f) return -1;
  size_t n = fread(buf, 1, cap - 1, f);
  fclose(f);
  buf[n] = 0;
  return (int)n;
}

/* ----------------------------------------------------------------- parser -- */

static int scheme_from_port(int port)
{
  if (port == 1080 || port == 1081 || port == 1082 || port == 9050 || port == 9150)
    return PROTO_SOCKS5;
  return PROTO_HTTP;
}

static int parse_proxy_spec(const char *raw, Proxy *out)
{
  char tmp[512];
  snprintf(tmp, sizeof(tmp), "%s", raw);
  trim(tmp);
  if (!tmp[0] || tmp[0] == '#' || tmp[0] == ';') return 0;

  int proto = -1;
  char *s = tmp;
  if (strstr(s, "://")) {
    char *sep = strstr(s, "://");
    *sep = 0;
    if (!strcasecmp(s, "socks5")) proto = PROTO_SOCKS5;
    else if (!strcasecmp(s, "socks4")) proto = PROTO_SOCKS4;
    else if (!strcasecmp(s, "http")) proto = PROTO_HTTP;
    else return 0;
    s = sep + 3;
  } else if (!strncasecmp(s, "socks5:", 7)) {
    proto = PROTO_SOCKS5;
    s += 7;
  } else if (!strncasecmp(s, "socks4:", 7)) {
    proto = PROTO_SOCKS4;
    s += 7;
  } else if (!strncasecmp(s, "http:", 5)) {
    proto = PROTO_HTTP;
    s += 5;
  }

  char user[128] = "", pass[128] = "", host[MAX_HOST] = "", portbuf[16] = "";
  char *at = strrchr(s, '@');
  if (at) {
    *at = 0;
    char *colon = strchr(s, ':');
    if (colon) {
      *colon = 0;
      snprintf(user, sizeof(user), "%s", s);
      snprintf(pass, sizeof(pass), "%s", colon + 1);
    } else {
      snprintf(user, sizeof(user), "%s", s);
    }
    s = at + 1;
  }

  if (s[0] == '[') {
    char *close = strchr(s, ']');
    if (!close) return 0;
    *close = 0;
    snprintf(host, sizeof(host), "%s", s + 1);
    snprintf(portbuf, sizeof(portbuf), "%s", close[1] == ':' ? close + 2 : "");
  } else {
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
    snprintf(host, sizeof(host), "%s", parts[0]);
    snprintf(portbuf, sizeof(portbuf), "%s", parts[1]);
    if (np >= 4 && !user[0]) {
      snprintf(user, sizeof(user), "%s", parts[2]);
      snprintf(pass, sizeof(pass), "%s", parts[3]);
    }
  }

  trim(host);
  trim(portbuf);
  if (!host[0] || !portbuf[0]) return 0;
  int port = atoi(portbuf);
  if (port < 1 || port > 65535) return 0;
  if (proto < 0) proto = scheme_from_port(port);
  if (proto == PROTO_SOCKS4 && (user[0] || pass[0])) return 0;

  memset(out, 0, sizeof(*out));
  out->proto = proto;
  snprintf(out->host, sizeof(out->host), "%s", host);
  snprintf(out->user, sizeof(out->user), "%s", user);
  snprintf(out->pass, sizeof(out->pass), "%s", pass);
  out->port = port;
  return 1;
}

/* ---------------------------------------------------------------- config --- */

static int g_cfg_load_failed = 0;

/* Renders table settings(key, value) as "key=value" lines.
 * Returns the length, -1 when missing or empty, -2 when the database is busy. */
static int read_settings_db(char *buf, int cap)
{
  char path[MAX_DIR + 32];
  snprintf(path, sizeof(path), "%s/proxies.db", g_dir);

  sqlite3 *db = NULL;
  if (sqlite3_open_v2(path, &db, SQLITE_OPEN_READONLY, NULL) != SQLITE_OK) {
    if (db) sqlite3_close(db);
    return -1;
  }
  sqlite3_busy_timeout(db, 300);

  sqlite3_stmt *st = NULL;
  if (sqlite3_prepare_v2(db, "SELECT key, value FROM settings", -1, &st, NULL) != SQLITE_OK) {
    int busy = sqlite3_errcode(db) == SQLITE_BUSY || sqlite3_errcode(db) == SQLITE_LOCKED;
    sqlite3_close(db);
    return busy ? -2 : -1;
  }

  int total = 0, rc;
  while ((rc = sqlite3_step(st)) == SQLITE_ROW) {
    const char *k = (const char *)sqlite3_column_text(st, 0);
    const char *v = (const char *)sqlite3_column_text(st, 1);
    if (!k || !v) continue;
    int w = snprintf(buf + total, (size_t)(cap - total), "%s=%s\n", k, v);
    if (w < 0 || total + w >= cap - 1) break;
    total += w;
  }
  buf[total] = 0;
  sqlite3_finalize(st);
  sqlite3_close(db);
  if (rc != SQLITE_ROW && rc != SQLITE_DONE) return -2;
  return total > 0 ? total : -1;
}

static void load_config_locked(void)
{
  char buf[8192];
  int n = read_settings_db(buf, sizeof(buf));
  g_cfg_load_failed = (n == -2);
  if (n <= 0) return;

  char *line = buf;
  while (line && *line) {
    char *nl = strchr(line, '\n');
    if (nl) *nl = 0;
    char *s = line;
    trim(s);
    if (*s && *s != '#' && *s != ';' && *s != '=') {
      char *eq = strchr(s, '=');
      if (eq) {
        *eq = 0;
        char *key = s, *val = eq + 1;
        trim(key);
        trim(val);
        if (!strcasecmp(key, "enabled")) g_enabled = atoi(val) != 0;
        else if (!strcasecmp(key, "fail_closed")) g_fail_closed = atoi(val) != 0;
        else if (!strcasecmp(key, "block_udp")) g_block_udp = atoi(val) != 0;
        else if (!strcasecmp(key, "allow_dns_udp")) g_allow_dns_udp = atoi(val) != 0;
        else if (!strcasecmp(key, "rotate_sec")) g_rotate_sec = atoi(val);
        else if (!strcasecmp(key, "retry_next")) g_retry_next = atoi(val) != 0;
        else if (!strcasecmp(key, "timeout_ms")) g_timeout_ms = atoi(val);
        else if (!strcasecmp(key, "current")) g_cur = atoi(val);
        else if (!strcasecmp(key, "log")) g_log_on = atoi(val) != 0;
      }
    }
    line = nl ? nl + 1 : NULL;
  }
  if (g_timeout_ms < 500) g_timeout_ms = 500;
}

static int g_px_load_failed = 0;

/* Joins every row of proxies.db into one newline-separated string.
 * Returns the length, -1 when the database is missing, -2 when it is busy. */
static int read_proxy_db(char *buf, int cap)
{
  char path[MAX_DIR + 32];
  snprintf(path, sizeof(path), "%s/proxies.db", g_dir);

  sqlite3 *db = NULL;
  if (sqlite3_open_v2(path, &db, SQLITE_OPEN_READONLY, NULL) != SQLITE_OK) {
    if (db) sqlite3_close(db);
    return -1;
  }
  sqlite3_busy_timeout(db, 300);

  sqlite3_stmt *st = NULL;
  if (sqlite3_prepare_v2(db, "SELECT text FROM proxies ORDER BY id", -1, &st, NULL) != SQLITE_OK) {
    int busy = sqlite3_errcode(db) == SQLITE_BUSY || sqlite3_errcode(db) == SQLITE_LOCKED;
    sqlite3_close(db);
    return busy ? -2 : -1;
  }

  int total = 0, rc;
  while ((rc = sqlite3_step(st)) == SQLITE_ROW) {
    const unsigned char *t = sqlite3_column_text(st, 0);
    if (!t) continue;
    int n = (int)strlen((const char *)t);
    if (total + n + 2 >= cap) break;
    memcpy(buf + total, t, (size_t)n);
    total += n;
    buf[total++] = '\n';
  }
  buf[total] = 0;
  sqlite3_finalize(st);
  sqlite3_close(db);
  if (rc != SQLITE_ROW && rc != SQLITE_DONE) return -2;
  return total;
}

static void load_proxies_locked(void)
{
  static char buf[128 * 1024];
  int n = read_proxy_db(buf, sizeof(buf));
  if (n == -2) { g_px_load_failed = 1; return; }
  g_px_load_failed = 0;
  g_npx = 0;
  if (n <= 0) return;

  char *line = buf;
  while (line && *line && g_npx < MAX_PROXIES) {
    char *nl = strchr(line, '\n');
    if (nl) *nl = 0;
    if (nl && nl > line && nl[-1] == '\r') nl[-1] = 0;
    if (parse_proxy_spec(line, &g_px[g_npx])) g_npx++;
    line = nl ? nl + 1 : NULL;
  }
  if (g_npx > 0) g_cur %= g_npx;
  if (g_cur < 0) g_cur = 0;
}

static void write_state(void);

static void reload_if_changed(void)
{
  if (g_forced) return;
  uint64_t t = now_ms();
  if (t - g_last_check < 400) return;
  g_last_check = t;

  char pxf[MAX_DIR + 32];
  snprintf(pxf, sizeof(pxf), "%s/proxies.db", g_dir);
  unsigned long sp = file_sig(pxf);
  if (sp == g_sig_px) return;

  pthread_mutex_lock(&g_lock);
  g_sig_px = sp;
  load_config_locked();
  load_proxies_locked();
  if (g_px_load_failed || g_cfg_load_failed) g_sig_px = 0;
  pthread_mutex_unlock(&g_lock);
  write_state();
}

/* ------------------------------------------------------------------ state -- */

static void exe_name(char *out, size_t cap)
{
  out[0] = 0;
#ifdef __linux__
  char path[64];
  snprintf(path, sizeof(path), "/proc/%d/exe", (int)getpid());
  ssize_t n = readlink(path, out, cap - 1);
  if (n > 0) {
    out[n] = 0;
    char *slash = strrchr(out, '/');
    if (slash) memmove(out, slash + 1, strlen(slash + 1) + 1);
    return;
  }
#endif
  snprintf(out, cap, "-");
}

static void write_state(void)
{
  char sdir[MAX_DIR + 16], dir[MAX_DIR + 32], tmp[MAX_DIR + 48], path[MAX_DIR + 48];
  char exe[256], body[768];

  pthread_mutex_lock(&g_lock);
  int cur = g_cur, npx = g_npx, enabled = g_enabled;
  uint64_t rotated = g_last_rotate;
  pthread_mutex_unlock(&g_lock);

  snprintf(sdir, sizeof(sdir), "%s/state", g_dir);
  mkdir(sdir, 0777);
  exe_name(exe, sizeof(exe));
  snprintf(dir, sizeof(dir), "%s", sdir);
  snprintf(tmp, sizeof(tmp), "%s/%d.tmp", dir, (int)g_pid);
  snprintf(path, sizeof(path), "%s/%d.txt", dir, (int)g_pid);
  snprintf(body, sizeof(body),
           "pid=%d\nexe=%s\ncurrent=%d\nproxies=%d\nenabled=%d\nrotated_ms=%llu\n",
           (int)g_pid, exe, cur, npx, enabled, (unsigned long long)rotated);

  FILE *f = fopen(tmp, "wb");
  if (f) {
    fwrite(body, 1, strlen(body), f);
    fclose(f);
    rename(tmp, path);
  }
}

/* --------------------------------------------------------- hostname map ---- */

static void map_store(int family, const unsigned char *addr, const char *host)
{
  if (!addr || !host || !host[0]) return;
  size_t len = strlen(host);
  if (len >= MAX_HOST) return;
  int slot = family == AF_INET ? 4 : 16;

  pthread_mutex_lock(&g_lock);
  for (int i = 0; i < MAP_SLOTS; i++) {
    if (g_map[i].used && g_map[i].family == family && !memcmp(g_map[i].addr, addr, slot)) {
      snprintf(g_map[i].host, sizeof(g_map[i].host), "%s", host);
      g_map[i].stamp = now_ms();
      pthread_mutex_unlock(&g_lock);
      return;
    }
  }
  HostMap *e = &g_map[g_map_slot];
  g_map_slot = (g_map_slot + 1) % MAP_SLOTS;
  e->used = 1;
  e->family = family;
  memcpy(e->addr, addr, slot);
  snprintf(e->host, sizeof(e->host), "%s", host);
  e->stamp = now_ms();
  pthread_mutex_unlock(&g_lock);
}

static int map_lookup(int family, const unsigned char *addr, char *out, size_t cap)
{
  int slot = family == AF_INET ? 4 : 16;
  int found = 0;
  pthread_mutex_lock(&g_lock);
  for (int i = 0; i < MAP_SLOTS; i++) {
    if (g_map[i].used && g_map[i].family == family && !memcmp(g_map[i].addr, addr, slot)) {
      snprintf(out, cap, "%s", g_map[i].host);
      found = 1;
      break;
    }
  }
  pthread_mutex_unlock(&g_lock);
  return found;
}

/* ---------------------------------------------------------------- sockets -- */

static int set_nonblock(int fd, int on)
{
  int fl = fcntl(fd, F_GETFL, 0);
  if (fl < 0) return -1;
  return fcntl(fd, F_SETFL, on ? (fl | O_NONBLOCK) : (fl & ~O_NONBLOCK));
}

static int wait_fd(int fd, int for_write, uint64_t deadline)
{
  for (;;) {
    uint64_t t = now_ms();
    if (t >= deadline) return -1;
    struct pollfd pfd;
    pfd.fd = fd;
    pfd.events = for_write ? POLLOUT : POLLIN;
    pfd.revents = 0;
    int r = poll(&pfd, 1, (int)(deadline - t));
    if (r > 0) return 0;
    if (r == 0) return -1;
    if (errno != EINTR) return -1;
  }
}

static int send_all(int fd, const void *buf, size_t len, uint64_t deadline)
{
  const char *p = (const char *)buf;
  while (len > 0) {
    ssize_t n = send(fd, p, len, MSG_NOSIGNAL);
    if (n > 0) {
      p += n;
      len -= (size_t)n;
      continue;
    }
    if (n < 0 && (errno == EAGAIN || errno == EWOULDBLOCK)) {
      if (wait_fd(fd, 1, deadline) != 0) return -1;
      continue;
    }
    if (n < 0 && errno == EINTR) continue;
    return -1;
  }
  return 0;
}

static int recv_all(int fd, void *buf, size_t len, uint64_t deadline)
{
  char *p = (char *)buf;
  size_t got = 0;
  while (got < len) {
    ssize_t n = recv(fd, p + got, len - got, 0);
    if (n > 0) {
      got += (size_t)n;
      continue;
    }
    if (n == 0) return -1;
    if (errno == EAGAIN || errno == EWOULDBLOCK) {
      if (wait_fd(fd, 0, deadline) != 0) return -1;
      continue;
    }
    if (errno == EINTR) continue;
    return -1;
  }
  return 0;
}

static int connect_real(int fd, const struct sockaddr *addr, socklen_t len, uint64_t deadline)
{
  if (real_connect(fd, addr, len) == 0) return 0;
  if (errno != EINPROGRESS && errno != EALREADY) return -1;
  if (wait_fd(fd, 1, deadline) != 0) return -1;
  int err = 0;
  socklen_t elen = sizeof(err);
  if (getsockopt(fd, SOL_SOCKET, SO_ERROR, &err, &elen) != 0) return -1;
  if (err != 0) {
    errno = err;
    return -1;
  }
  return 0;
}

static int connect_proxy(const Proxy *p, uint64_t deadline)
{
  struct addrinfo hints, *res = NULL;
  memset(&hints, 0, sizeof(hints));
  hints.ai_family = AF_UNSPEC;
  hints.ai_socktype = SOCK_STREAM;
  char port[16];
  snprintf(port, sizeof(port), "%d", p->port);
  if (real_getaddrinfo(p->host, port, &hints, &res) != 0 || !res) return -1;

  int fd = -1;
  for (struct addrinfo *ai = res; ai; ai = ai->ai_next) {
    fd = socket(ai->ai_family, ai->ai_socktype, ai->ai_protocol);
    if (fd < 0) continue;
    set_nonblock(fd, 1);
    if (connect_real(fd, ai->ai_addr, ai->ai_addrlen, deadline) == 0) break;
    real_close(fd);
    fd = -1;
  }
  freeaddrinfo(res);
  return fd;
}

/* -------------------------------------------------------------- handshake -- */

static int base64_encode(const char *in, char *out, size_t cap)
{
  static const char *tbl = "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
  size_t len = strlen(in), o = 0;
  for (size_t i = 0; i < len; i += 3) {
    unsigned v = (unsigned char)in[i] << 16;
    if (i + 1 < len) v |= (unsigned char)in[i + 1] << 8;
    if (i + 2 < len) v |= (unsigned char)in[i + 2];
    if (o + 4 >= cap) return -1;
    out[o++] = tbl[(v >> 18) & 63];
    out[o++] = tbl[(v >> 12) & 63];
    out[o++] = i + 1 < len ? tbl[(v >> 6) & 63] : '=';
    out[o++] = i + 2 < len ? tbl[v & 63] : '=';
  }
  out[o] = 0;
  return (int)o;
}

static int handshake_socks5(int fd, const Proxy *p, const char *host, int port, uint64_t deadline)
{
  unsigned char buf[300];
  int nm = p->user[0] ? 2 : 1;
  size_t n = 0;
  buf[n++] = 5;
  buf[n++] = (unsigned char)nm;
  buf[n++] = 0;
  if (p->user[0]) buf[n++] = 2;
  if (send_all(fd, buf, n, deadline) != 0) return -1;

  unsigned char rep[2];
  if (recv_all(fd, rep, 2, deadline) != 0) return -1;
  if (rep[0] != 5 || rep[1] == 0xFF) return -1;
  if (rep[1] == 2) {
    if (!p->user[0]) return -1;
    size_t ul = strlen(p->user), pl = strlen(p->pass);
    if (ul > 255) ul = 255;
    if (pl > 255) pl = 255;
    n = 0;
    buf[n++] = 1;
    buf[n++] = (unsigned char)ul;
    memcpy(buf + n, p->user, ul);
    n += ul;
    buf[n++] = (unsigned char)pl;
    memcpy(buf + n, p->pass, pl);
    n += pl;
    if (send_all(fd, buf, n, deadline) != 0) return -1;
    unsigned char ar[2];
    if (recv_all(fd, ar, 2, deadline) != 0) return -1;
    if (ar[1] != 0) return -1;
  } else if (rep[1] != 0) {
    return -1;
  }

  unsigned char ip4[4], ip6[16];
  n = 0;
  buf[n++] = 5;
  buf[n++] = 1;
  buf[n++] = 0;
  if (inet_pton(AF_INET, host, ip4) == 1) {
    buf[n++] = 1;
    memcpy(buf + n, ip4, 4);
    n += 4;
  } else if (inet_pton(AF_INET6, host, ip6) == 1) {
    buf[n++] = 4;
    memcpy(buf + n, ip6, 16);
    n += 16;
  } else {
    size_t hl = strlen(host);
    if (hl > 255) hl = 255;
    buf[n++] = 3;
    buf[n++] = (unsigned char)hl;
    memcpy(buf + n, host, hl);
    n += hl;
  }
  buf[n++] = (unsigned char)(port >> 8);
  buf[n++] = (unsigned char)(port & 0xFF);
  if (send_all(fd, buf, n, deadline) != 0) return -1;

  unsigned char head[4];
  if (recv_all(fd, head, 4, deadline) != 0) return -1;
  if (head[0] != 5 || head[1] != 0) return -1;
  if (head[3] == 1) {
    if (recv_all(fd, buf, 4 + 2, deadline) != 0) return -1;
  } else if (head[3] == 3) {
    unsigned char hl = 0;
    if (recv_all(fd, &hl, 1, deadline) != 0) return -1;
    if (recv_all(fd, buf, hl + 2, deadline) != 0) return -1;
  } else if (head[3] == 4) {
    if (recv_all(fd, buf, 16 + 2, deadline) != 0) return -1;
  } else {
    return -1;
  }
  return 0;
}

static int handshake_socks4(int fd, const Proxy *p, const char *host, int port, uint64_t deadline)
{
  (void)p;
  unsigned char req[300];
  unsigned char ip4[4];
  if (inet_pton(AF_INET, host, ip4) != 1) return -1;
  size_t n = 0;
  req[n++] = 4;
  req[n++] = 1;
  req[n++] = (unsigned char)(port >> 8);
  req[n++] = (unsigned char)(port & 0xFF);
  memcpy(req + n, ip4, 4);
  n += 4;
  req[n++] = 0;
  if (send_all(fd, req, n, deadline) != 0) return -1;
  unsigned char rep[8];
  if (recv_all(fd, rep, 8, deadline) != 0) return -1;
  return rep[1] == 0x5A ? 0 : -1;
}

static int handshake_http(int fd, const Proxy *p, const char *host, int port, uint64_t deadline)
{
  char req[768], head[512], cred[400], token[560];
  int is_v6 = strchr(host, ':') != NULL;
  char target[340];
  if (is_v6) snprintf(target, sizeof(target), "[%s]:%d", host, port);
  else snprintf(target, sizeof(target), "%s:%d", host, port);

  cred[0] = 0;
  if (p->user[0]) {
    snprintf(cred, sizeof(cred), "%s:%s", p->user, p->pass);
    if (base64_encode(cred, token, sizeof(token)) < 0) return -1;
    snprintf(req, sizeof(req),
             "CONNECT %s HTTP/1.1\r\nHost: %s\r\nProxy-Authorization: Basic %s\r\n\r\n",
             target, target, token);
  } else {
    snprintf(req, sizeof(req), "CONNECT %s HTTP/1.1\r\nHost: %s\r\n\r\n", target, target);
  }
  if (send_all(fd, req, strlen(req), deadline) != 0) return -1;

  size_t got = 0;
  while (got + 1 < sizeof(head)) {
    if (recv_all(fd, head + got, 1, deadline) != 0) return -1;
    got++;
    if (got >= 4 && !memcmp(head + got - 4, "\r\n\r\n", 4)) break;
  }
  head[got] = 0;
  return strstr(head, " 200") ? 0 : -1;
}

/* ------------------------------------------------------------- interception- */

static int is_local_v4(const unsigned char *b)
{
  return b[0] == 127 || b[0] == 10 || b[0] == 0 || b[0] == 255 ||
         (b[0] == 172 && b[1] >= 16 && b[1] <= 31) ||
         (b[0] == 192 && b[1] == 168) ||
         (b[0] == 169 && b[1] == 254);
}

static int is_local_v6(const unsigned char *b)
{
  if (!memcmp(b, "\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\1", 16)) return 1;
  if ((b[0] & 0xFE) == 0xFC) return 1;
  if (b[0] == 0xFE && (b[1] & 0xC0) == 0x80) return 1;
  return 0;
}

static int is_fake_ip(const unsigned char *b)
{
  return b[0] == 198 && (b[1] == 18 || b[1] == 19);
}

static int is_socket_stream(int fd, int *is_udp)
{
  int type = 0;
  socklen_t len = sizeof(type);
  if (getsockopt(fd, SOL_SOCKET, SO_TYPE, &type, &len) != 0) return 0;
  *is_udp = type == SOCK_DGRAM;
  return type == SOCK_STREAM || type == SOCK_DGRAM;
}

static int proxy_attempt(const Proxy *p, const char *host, int port,
                         const char *numeric_host, uint64_t deadline)
{
  int fd = connect_proxy(p, deadline);
  if (fd < 0) return -1;
  int rc = -1;
  if (p->proto == PROTO_SOCKS5) rc = handshake_socks5(fd, p, host, port, deadline);
  else if (p->proto == PROTO_SOCKS4) rc = handshake_socks4(fd, p, numeric_host, port, deadline);
  else rc = handshake_http(fd, p, host, port, deadline);
  if (rc != 0) {
    real_close(fd);
    return -1;
  }
  return fd;
}

int connect(int fd, const struct sockaddr *addr, socklen_t addrlen)
{
  if (!real_connect) return -1;

  int is_udp = 0;
  if (!addr || (addr->sa_family != AF_INET && addr->sa_family != AF_INET6) ||
      !is_socket_stream(fd, &is_udp)) {
    return real_connect(fd, addr, addrlen);
  }

  int port = 0;
  unsigned char ip[16];
  int family = addr->sa_family;
  if (family == AF_INET) {
    const struct sockaddr_in *in4 = (const struct sockaddr_in *)addr;
    port = ntohs(in4->sin_port);
    memcpy(ip, &in4->sin_addr, 4);
  } else {
    const struct sockaddr_in6 *in6 = (const struct sockaddr_in6 *)addr;
    port = ntohs(in6->sin6_port);
    memcpy(ip, &in6->sin6_addr, 16);
  }

  pthread_mutex_lock(&g_lock);
  int enabled = g_enabled;
  int block_udp = g_block_udp;
  int allow_dns = g_allow_dns_udp;
  int npx = g_npx;
  int cur = g_cur;
  int retry = g_retry_next;
  int fail_closed = g_fail_closed;
  int rotate = g_rotate_sec;
  int timeout_ms = g_timeout_ms;
  pthread_mutex_unlock(&g_lock);

  int local = family == AF_INET ? is_local_v4(ip) : is_local_v6(ip);
  if (is_udp) {
    if (!enabled) return real_connect(fd, addr, addrlen);
    if (local) return real_connect(fd, addr, addrlen);
    if (port == 53 && allow_dns) return real_connect(fd, addr, addrlen);
    if (block_udp && !local) {
      errno = ENETUNREACH;
      return -1;
    }
    return real_connect(fd, addr, addrlen);
  }

  if (!enabled || npx <= 0 || local) return real_connect(fd, addr, addrlen);

  char numeric[64] = "", host[MAX_HOST] = "";
  if (family == AF_INET) inet_ntop(AF_INET, ip, numeric, sizeof(numeric));
  else inet_ntop(AF_INET6, ip, numeric, sizeof(numeric));

  int have_name = map_lookup(family, ip, host, sizeof(host));
  if (!have_name && family == AF_INET && is_fake_ip(ip)) {
    errno = ENETUNREACH;
    return -1;
  }
  if (!have_name) snprintf(host, sizeof(host), "%s", numeric);

  if (rotate > 0) {
    pthread_mutex_lock(&g_lock);
    if (now_ms() - g_last_rotate >= (uint64_t)rotate * 1000u) {
      if (g_npx > 0) g_cur = (g_cur + 1) % g_npx;
      g_last_rotate = now_ms();
      pthread_mutex_unlock(&g_lock);
      write_state();
    } else {
      pthread_mutex_unlock(&g_lock);
    }
    pthread_mutex_lock(&g_lock);
    cur = g_cur;
    npx = g_npx;
    pthread_mutex_unlock(&g_lock);
  }

  int attempts = retry ? (npx < 3 ? npx : 3) : 1;
  uint64_t deadline = now_ms() + (uint64_t)timeout_ms;
  int saved_flags = fcntl(fd, F_GETFL, 0);

  for (int a = 0; a < attempts; a++) {
    int idx = (cur + a) % npx;
    Proxy p;
    pthread_mutex_lock(&g_lock);
    if (g_npx > 0) p = g_px[idx];
    pthread_mutex_unlock(&g_lock);

    int upstream = proxy_attempt(&p, host, port, numeric, deadline);
    if (upstream < 0) {
      logf_("proxy %d %s:%d failed for %s:%d", idx, p.host, p.port, host, port);
      continue;
    }
    if (dup2(upstream, fd) < 0) {
      real_close(upstream);
      continue;
    }
    real_close(upstream);
    if (saved_flags >= 0) fcntl(fd, F_SETFL, saved_flags);
    logf_("proxy %d %s via %s:%d", idx, host, p.host, p.port);
    return 0;
  }

  logf_("all proxies failed for %s:%d (fail_closed=%d)", host, port, fail_closed);
  if (fail_closed) {
    errno = ECONNREFUSED;
    return -1;
  }
  return real_connect(fd, addr, addrlen);
}

int getaddrinfo(const char *node, const char *service, const struct addrinfo *hints,
                struct addrinfo **res)
{
  if (!real_getaddrinfo) return EAI_FAIL;
  int rc = real_getaddrinfo(node, service, hints, res);
  if (rc != 0 || !node || !*res) return rc;
  if (!strcmp(node, "localhost")) return rc;

  for (struct addrinfo *ai = *res; ai; ai = ai->ai_next) {
    if (ai->ai_family == AF_INET)
      map_store(AF_INET, (const unsigned char *)&((struct sockaddr_in *)ai->ai_addr)->sin_addr, node);
    else if (ai->ai_family == AF_INET6)
      map_store(AF_INET6, (const unsigned char *)&((struct sockaddr_in6 *)ai->ai_addr)->sin6_addr, node);
  }
  return rc;
}

/* --------------------------------------------------------------- bootstrap - */

static int has_config(const char *dir)
{
  char path[MAX_DIR + 32];
  struct stat st;
  snprintf(path, sizeof(path), "%s/proxies.db", dir);
  return stat(path, &st) == 0 && !S_ISDIR(st.st_mode);
}

static void resolve_dir(void)
{
  const char *env = getenv("PROXYINJECT_DIR");
  if (env && env[0]) {
    snprintf(g_dir, sizeof(g_dir), "%s", env);
    return;
  }

  char base[MAX_DIR];
  base[0] = 0;
  Dl_info info;
  if (dladdr((void *)&write_state, &info) && info.dli_fname) {
    snprintf(base, sizeof(base), "%s", info.dli_fname);
    char *slash = strrchr(base, '/');
    if (slash) *slash = 0;
  }
  if (!base[0]) return;

  snprintf(g_dir, sizeof(g_dir), "%s", base);
  if (has_config(base)) return;

  char parent[MAX_DIR];
  snprintf(parent, sizeof(parent), "%s", base);
  char *slash = strrchr(parent, '/');
  if (slash && slash != parent) {
    *slash = 0;
    if (has_config(parent)) snprintf(g_dir, sizeof(g_dir), "%s", parent);
  }
}

__attribute__((constructor)) static void bootstrap(void)
{
  if (g_initialized) return;
  g_initialized = 1;
  g_pid = getpid();

  real_connect = (int (*)(int, const struct sockaddr *, socklen_t))dlsym(RTLD_NEXT, "connect");
  real_getaddrinfo = (int (*)(const char *, const char *, const struct addrinfo *,
                              struct addrinfo **))dlsym(RTLD_NEXT, "getaddrinfo");
  real_close = (int (*)(int))dlsym(RTLD_NEXT, "close");
  if (!real_connect || !real_getaddrinfo) return;
  if (!real_close) real_close = close;

  resolve_dir();
  char logs[MAX_DIR + 16];
  snprintf(logs, sizeof(logs), "%s/logs", g_dir);
  mkdir(logs, 0777);
  snprintf(g_logpath, sizeof(g_logpath), "%s/%d.log", logs, (int)g_pid);

  load_config_locked();
  load_proxies_locked();

  const char *forced = getenv("PROXYINJECT_FORCE");
  if (forced && forced[0]) {
    Proxy p;
    if (parse_proxy_spec(forced, &p)) {
      g_px[0] = p;
      g_npx = 1;
      g_cur = 0;
      g_enabled = 1;
      g_forced = 1;
    }
  }
  const char *logenv = getenv("PROXYINJECT_LOG");
  if (logenv && logenv[0]) g_log_on = atoi(logenv) != 0;

  char pxf[MAX_DIR + 32];
  snprintf(pxf, sizeof(pxf), "%s/proxies.db", g_dir);
  g_sig_px = file_sig(pxf);
  g_last_check = now_ms();
  g_last_rotate = now_ms();

  logf_("INIT dir=%s proxies=%d current=%d enabled=%d", g_dir, g_npx, g_cur, g_enabled);
  write_state();
}

__attribute__((destructor)) static void shutdown_hook(void)
{
  write_state();
}
