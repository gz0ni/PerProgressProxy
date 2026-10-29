/*
 * inject.exe - inject proxyinject.dll into a chosen process.
 *
 *   inject.exe --list                     list processes (pid, arch, name)
 *   inject.exe --name app.exe             inject into every matching process
 *   inject.exe --pid 1234                 inject into one pid
 *   inject.exe --launch "C:\path\app.exe" [args...]
 *                                         start suspended, inject, resume
 *   inject.exe --check --pid 1234         is the DLL loaded?
 *   inject.exe --selftest --proxy SPEC    load the DLL into this process and
 *                                          fetch an IP-echo URL through it
 *
 * Exit code 0 = success, 1 = failure, 2 = usage error.
 */

#define WIN32_LEAN_AND_MEAN
#define _CRT_SECURE_NO_WARNINGS
#include <winsock2.h>
#include <windows.h>
#include <ws2tcpip.h>
#include <tlhelp32.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <stdint.h>

#define DLL_NAME L"proxyinject.dll"

static void die(const char *msg)
{
    fprintf(stderr, "error: %s (win32=%lu)\n", msg, (unsigned long)GetLastError());
    exit(1);
}

static int my_dir(wchar_t *out, DWORD cap)
{
    if (!GetModuleFileNameW(NULL, out, cap)) return -1;
    wchar_t *slash = wcsrchr(out, L'\\');
    if (slash) *slash = 0;
    return 0;
}

static int is_64bit(HANDLE proc)
{
#ifdef _WIN64
    BOOL wow = FALSE;
    if (IsWow64Process(proc, &wow)) return wow ? 0 : 1;
    return 1;
#else
    (void)proc;
    return 0;
#endif
}

static const char *arch_name(int b64) { return b64 ? "x64" : "x86"; }

/* --------------------------------------------------------------- helpers -- */

typedef struct { DWORD pid; wchar_t name[64]; int is64; } ProcInfo;

static int list_procs(ProcInfo *out, int cap)
{
    HANDLE snap = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0);
    if (snap == INVALID_HANDLE_VALUE) return 0;
    PROCESSENTRY32W pe;
    memset(&pe, 0, sizeof(pe));
    pe.dwSize = sizeof(pe);
    int n = 0;
    if (Process32FirstW(snap, &pe)) {
        do {
            if (n >= cap) break;
            out[n].pid = pe.th32ProcessID;
            wcsncpy(out[n].name, pe.szExeFile, 63);
            out[n].name[63] = 0;
            out[n].is64 = 1;
            HANDLE h = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, FALSE, pe.th32ProcessID);
            if (h) {
                out[n].is64 = is_64bit(h);
                CloseHandle(h);
            }
            n++;
        } while (Process32NextW(snap, &pe));
    }
    CloseHandle(snap);
    return n;
}

static int module_loaded(DWORD pid, const wchar_t *dllName, uint64_t *baseOut)
{
    HANDLE snap = CreateToolhelp32Snapshot(TH32CS_SNAPMODULE | TH32CS_SNAPMODULE32, pid);
    if (snap == INVALID_HANDLE_VALUE) return 0;
    MODULEENTRY32W me;
    memset(&me, 0, sizeof(me));
    me.dwSize = sizeof(me);
    int found = 0;
    if (Module32FirstW(snap, &me)) {
        do {
            const wchar_t *a = me.szModule;
            const wchar_t *b = dllName;
            size_t la = wcslen(a), lb = wcslen(b);
            if (la == lb && _wcsicmp(a, b) == 0) {
                if (baseOut) *baseOut = (uint64_t)(uintptr_t)me.modBaseAddr;
                found = 1;
                break;
            }
            memset(&me, 0, sizeof(me));
            me.dwSize = sizeof(me);
        } while (Module32NextW(snap, &me));
    }
    CloseHandle(snap);
    return found;
}

static HANDLE ready_event_create(DWORD pid)
{
    wchar_t name[64];
    _snwprintf(name, 63, L"Local\\proxyinject-ready-%lu", (unsigned long)pid);
    name[63] = 0;
    return CreateEventW(NULL, TRUE, FALSE, name);
}

static void ready_event_wait(HANDLE ev, DWORD pid, int quiet)
{
    if (!ev) return;
    DWORD w = WaitForSingleObject(ev, 15000);
    if (w != WAIT_OBJECT_0 && !quiet)
        fprintf(stderr, "  pid %lu: warning: DLL did not signal ready in 15s\n",
                (unsigned long)pid);
    CloseHandle(ev);
}

static int do_inject(DWORD pid, const wchar_t *dllPath, int quiet)
{
    if (module_loaded(pid, DLL_NAME, NULL)) {
        if (!quiet)
            fprintf(stderr, "  pid %lu: already injected, skipping\n",
                    (unsigned long)pid);
        return 0;
    }
    HANDLE proc = OpenProcess(PROCESS_CREATE_THREAD | PROCESS_QUERY_INFORMATION |
                                  PROCESS_VM_OPERATION | PROCESS_VM_WRITE,
                              FALSE, pid);
    if (!proc) {
        if (!quiet)
            fprintf(stderr, "  pid %lu: OpenProcess failed (%lu)%s\n",
                    (unsigned long)pid, (unsigned long)GetLastError(),
                    GetLastError() == 5 ? " - try running as admin" : "");
        return -1;
    }
    if (!is_64bit(proc)) {
#ifdef _WIN64
        fprintf(stderr, "  pid %lu: target is 32-bit, use inject32.exe\n",
                (unsigned long)pid);
        CloseHandle(proc);
        return -1;
#endif
    }

    size_t bytes = (wcslen(dllPath) + 1) * sizeof(wchar_t);
    void *remote = VirtualAllocEx(proc, NULL, bytes, MEM_COMMIT | MEM_RESERVE,
                                  PAGE_READWRITE);
    if (!remote) { CloseHandle(proc); return -1; }
    if (!WriteProcessMemory(proc, remote, dllPath, bytes, NULL)) {
        VirtualFreeEx(proc, remote, 0, MEM_RELEASE);
        CloseHandle(proc);
        return -1;
    }

    HMODULE k32 = GetModuleHandleW(L"kernel32.dll");
    FARPROC loadlib = GetProcAddress(k32, "LoadLibraryW");
    HANDLE rev = ready_event_create(pid);
    HANDLE th = CreateRemoteThread(proc, NULL, 0, (LPTHREAD_START_ROUTINE)loadlib,
                                  remote, 0, NULL);
    if (!th) {
        fprintf(stderr, "  pid %lu: CreateRemoteThread failed (%lu)\n",
                (unsigned long)pid, (unsigned long)GetLastError());
        VirtualFreeEx(proc, remote, 0, MEM_RELEASE);
        if (rev) CloseHandle(rev);
        CloseHandle(proc);
        return -1;
    }
    WaitForSingleObject(th, 20000);
    DWORD ret = 0;
    GetExitCodeThread(th, &ret);
    CloseHandle(th);
    Sleep(120);
    VirtualFreeEx(proc, remote, 0, MEM_RELEASE);
    CloseHandle(proc);

    if (ret == 0 && !module_loaded(pid, DLL_NAME, NULL)) {
        fprintf(stderr, "  pid %lu: LoadLibrary returned 0 (blocked by AV? bitness?)\n",
                (unsigned long)pid);
        if (rev) CloseHandle(rev);
        return -1;
    }
    ready_event_wait(rev, pid, quiet);
    if (!quiet) {
        uint64_t base = 0;
        module_loaded(pid, DLL_NAME, &base);
        printf("  pid %lu: injected, module base 0x%llx\n", (unsigned long)pid,
               (unsigned long long)base);
    }
    return 0;
}

/* ---------------------------------------------------------------- selftest */

static int http_get_via_dll(const char *url, char *out, int cap)
{
    /* parse http://host[/path] */
    if (strncmp(url, "http://", 7) != 0) {
        fprintf(stderr, "selftest only supports http:// urls\n");
        return -1;
    }
    char host[256] = "", path[512] = "/";
    const char *p = url + 7;
    const char *slash = strchr(p, '/');
    if (slash) {
        size_t hl = (size_t)(slash - p);
        if (hl >= sizeof(host)) hl = sizeof(host) - 1;
        memcpy(host, p, hl);
        host[hl] = 0;
        _snprintf(path, sizeof(path) - 1, "%s", slash);
    } else {
        _snprintf(host, sizeof(host) - 1, "%s", p);
    }
    char hostonly[200];
    _snprintf(hostonly, sizeof(hostonly) - 1, "%s", host);
    char *colon = strchr(hostonly, ':');
    if (colon) *colon = 0;

    struct addrinfo hints, *res = NULL;
    memset(&hints, 0, sizeof(hints));
    hints.ai_family = AF_INET;
    hints.ai_socktype = SOCK_STREAM;
    if (getaddrinfo(hostonly, colon ? colon + 1 : "80", &hints, &res) != 0 || !res) {
        fprintf(stderr, "selftest: cannot resolve %s\n", hostonly);
        return -1;
    }
    SOCKET s = socket(AF_INET, SOCK_STREAM, IPPROTO_TCP);
    if (s == INVALID_SOCKET) return -1;
    if (connect(s, res->ai_addr, (int)res->ai_addrlen) != 0) {
        fprintf(stderr, "selftest: connect failed err=%d\n", WSAGetLastError());
        freeaddrinfo(res);
        closesocket(s);
        return -1;
    }
    freeaddrinfo(res);

    char req[1024];
    _snprintf(req, sizeof(req) - 1,
              "GET %s HTTP/1.0\r\nHost: %s\r\nUser-Agent: proxyinject\r\n"
              "Connection: close\r\n\r\n",
              path, host);
    send(s, req, (int)strlen(req), 0);

    DWORD t0 = GetTickCount();
    int total = 0;
    while (total < cap - 1) {
        if (GetTickCount() - t0 > 20000) break;
        int r = recv(s, out + total, cap - 1 - total, 0);
        if (r <= 0) break;
        total += r;
    }
    out[total] = 0;
    closesocket(s);
    if (total <= 0) return -1;
    char *body = strstr(out, "\r\n\r\n");
    if (body) memmove(out, body + 4, strlen(body + 4) + 1);
    /* keep first line only */
    char *nl = strpbrk(out, "\r\n");
    if (nl) *nl = 0;
    return 0;
}

static int selftest(const char *proxy, const char *url)
{
    if (proxy && proxy[0]) SetEnvironmentVariableA("PROXYINJECT_FORCE", proxy);
    SetEnvironmentVariableA("PROXYINJECT_LOG", "1");

    wchar_t dir[MAX_PATH], dll[MAX_PATH];
    if (my_dir(dir, MAX_PATH) != 0) die("cannot determine own path");
    _snwprintf(dll, MAX_PATH - 1, L"%ls\\%ls", dir, DLL_NAME);
    dll[MAX_PATH - 1] = 0;

    DWORD before = GetTickCount();
    HMODULE m = LoadLibraryW(dll);
    if (!m) {
        fprintf(stderr, "selftest: LoadLibrary(%ls) failed %lu\n", dll,
                (unsigned long)GetLastError());
        return 1;
    }
    Sleep(700); /* let the worker install hooks */

    char body[4096];
    int rc = http_get_via_dll(url, body, sizeof(body));
    printf("proxy=%s\n", proxy && proxy[0] ? proxy : "(config.ini)");
    printf("elapsed=%lums\n", (unsigned long)(GetTickCount() - before));
    if (rc == 0) {
        printf("exit_ip=%s\n", body);
        printf("result=OK\n");
        return 0;
    }
    printf("result=FAIL\n");
    return 1;
}

/* ------------------------------------------------------------------ main -- */

static void usage(void)
{
    printf(
        "inject.exe - per-process proxy injector\n\n"
        "  --list\n"
        "  --pid <n> [--pid <n> ...]\n"
        "  --name <exe>            (all matching processes)\n"
        "  --launch \"<exe> [args]\" start suspended, inject, resume\n"
        "  --check --pid <n>\n"
        "  --selftest [--proxy <spec>] [--url <http url>]\n\n"
        "Exit: 0 ok, 1 failed, 2 usage.\n");
}

int main(int argc, char **argv)
{
    SetConsoleOutputCP(CP_UTF8);

    wchar_t dir[MAX_PATH], dllPath[MAX_PATH];
    if (my_dir(dir, MAX_PATH) != 0) die("cannot determine own path");
    _snwprintf(dllPath, MAX_PATH - 1, L"%ls\\%ls", dir, DLL_NAME);
    dllPath[MAX_PATH - 1] = 0;

    if (argc < 2) { usage(); return 2; }

    int want_list = 0, want_check = 0, want_selftest = 0;
    DWORD pids[256];
    int npids = 0;
    const char *name = NULL;
    const char *launch = NULL;
    const char *proxy = NULL;
    const char *url = "http://checkip.amazonaws.com/";

    for (int i = 1; i < argc; i++) {
        if (!strcmp(argv[i], "--list")) want_list = 1;
        else if (!strcmp(argv[i], "--check")) want_check = 1;
        else if (!strcmp(argv[i], "--selftest")) want_selftest = 1;
        else if (!strcmp(argv[i], "--pid") && i + 1 < argc) {
            if (npids < 256) pids[npids++] = (DWORD)strtoul(argv[++i], NULL, 10);
        } else if (!strcmp(argv[i], "--name") && i + 1 < argc) name = argv[++i];
        else if (!strcmp(argv[i], "--launch") && i + 1 < argc) launch = argv[++i];
        else if (!strcmp(argv[i], "--proxy") && i + 1 < argc) proxy = argv[++i];
        else if (!strcmp(argv[i], "--url") && i + 1 < argc) url = argv[++i];
        else if (!strcmp(argv[i], "--help") || !strcmp(argv[i], "-h")) {
            usage();
            return 0;
        }
    }

    if (want_selftest) return selftest(proxy, url);

    if (want_list) {
        ProcInfo *pi = (ProcInfo *)calloc(4096, sizeof(ProcInfo));
        int n = list_procs(pi, 4096);
        printf("%-8s %-4s %s\n", "PID", "ARCH", "NAME");
        for (int i = 0; i < n; i++) {
            if (!pi[i].pid) continue;
            printf("%-8lu %-4s %ls\n", (unsigned long)pi[i].pid,
                   arch_name(pi[i].is64), pi[i].name);
        }
        free(pi);
        return 0;
    }

    if (want_check && npids > 0) {
        for (int i = 0; i < npids; i++) {
            uint64_t base = 0;
            int ok = module_loaded(pids[i], DLL_NAME, &base);
            printf("pid %lu: %s", (unsigned long)pids[i],
                   ok ? "INJECTED" : "not injected");
            if (ok) printf(" base=0x%llx", (unsigned long long)base);
            printf("\n");
        }
        return 0;
    }

    if (name) {
        ProcInfo *pi = (ProcInfo *)calloc(4096, sizeof(ProcInfo));
        int n = list_procs(pi, 4096);
        int hits = 0, fails = 0;
        for (int i = 0; i < n; i++) {
            if (!pi[i].pid) continue;
            char nb[128];
            WideCharToMultiByte(CP_UTF8, 0, pi[i].name, -1, nb, sizeof(nb), NULL, NULL);
            if (_stricmp(nb, name) == 0) {
                hits++;
                if (do_inject(pi[i].pid, dllPath, 0) != 0) fails++;
            }
        }
        free(pi);
        if (!hits) {
            fprintf(stderr, "no process named %s\n", name);
            return 1;
        }
        return fails ? 1 : 0;
    }

    if (launch) {
        /* build a mutable command line for CreateProcessW */
        wchar_t cmd[4096];
        int wn = MultiByteToWideChar(CP_UTF8, 0, launch, -1, cmd, 4095);
        if (wn <= 0) { fprintf(stderr, "bad command line\n"); return 1; }
        cmd[4095] = 0;

        STARTUPINFOW si;
        PROCESS_INFORMATION pi;
        memset(&si, 0, sizeof(si));
        si.cb = sizeof(si);
        memset(&pi, 0, sizeof(pi));
        if (!CreateProcessW(NULL, cmd, NULL, NULL, FALSE, CREATE_SUSPENDED, NULL, NULL,
                            &si, &pi)) {
            fprintf(stderr, "CreateProcess failed %lu\n", (unsigned long)GetLastError());
            return 1;
        }
        printf("started pid %lu suspended\n", (unsigned long)pi.dwProcessId);
        int rc = do_inject(pi.dwProcessId, dllPath, 0);
        if (rc != 0) {
            fprintf(stderr, "injection failed, terminating\n");
            TerminateProcess(pi.hProcess, 1);
            return 1;
        }
        ResumeThread(pi.hThread);
        printf("resumed\n");
        CloseHandle(pi.hThread);
        CloseHandle(pi.hProcess);
        return 0;
    }

    if (npids > 0) {
        int fails = 0;
        for (int i = 0; i < npids; i++)
            if (do_inject(pids[i], dllPath, 0) != 0) fails++;
        return fails ? 1 : 0;
    }

    usage();
    return 2;
}
