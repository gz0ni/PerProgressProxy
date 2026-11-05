# PerProcessProxy

Make **one chosen `.exe`** send its TCP connections through a proxy you can
change at any moment, while everything else on the machine (browser, Clash,
games, Windows) keeps working exactly as before.

It works by injecting `proxyinject.dll` into the target process only. The DLL
hooks the process's own Winsock calls, so the app's socket *is* the connection
to the proxy - no extra relay thread, no TUN/VPN changes, no admin rights
required for normal apps.

```
app.exe --connect(example.com:443)--> [ proxyinject.dll ] --SOCKS5/HTTP--> 1.2.3.4:1080 --> example.com
```

Every change to the database (`proxies.db`: the proxy list and the settings) is picked up by running processes
in under a second, and live sockets are dropped on rotation, so the app shows a
new exit IP almost immediately - without restarting it.

On Windows the DLL is injected into a running process or a process started by
the injector. On Linux and macOS an equivalent shim is loaded at launch time
(`LD_PRELOAD` / `DYLD_INSERT_LIBRARIES`); see [Platform support](#platform-support).

---

## Quick start

```powershell
cd "C:\path\to\PerProcessProxy"

# 0. console: hotkeys, scrollable proxy list, live process state
.\run.cmd

# ...or one-shot commands
.\run.cmd doctor
.\run.cmd status
```

Linux / macOS launcher: `./run.sh` (prebuilt `bin/rot`, `bin/rot-aarch64`,
universal `bin/rot-macos`); see [Platform support](#platform-support).

Drop proxy lists into `proxy/` and they are merged into the database
automatically on every start:

```powershell
# any *.txt there: host:port, host:port:user:pass, socks5://..., http://...
copy my_list.txt proxy\
.\run.cmd import          # merge now, without restarting the console
.\run.cmd test --prune    # check them, keep the working ones
```

Then start or attach a target:

```powershell
.\run.cmd launch "C:\Program Files\App\app.exe"
.\run.cmd attach 1234
.\run.cmd attach Telegram.exe
```

Switch proxies from the console (`3`, `next`), or non-interactively:

```powershell
.\run.cmd use 7
.\run.cmd next
.\run.cmd auto 120      # rotate on its own every 2 minutes
.\run.cmd off / on      # instant bypass / re-enable
```

`.\run.cmd status` shows what is currently applied and which processes are still
alive (`state/<pid>.txt`).

---

## Console

`run.cmd` / `run.sh` / `rot` open the interactive console. It uses a
fixed layout and hotkeys instead of typed commands, and the proxy list lives in
a scrollable viewport, so a list of hundreds of proxies never floods the screen
or blocks you from switching, testing or attaching.

`run.cmd` starts `bin\rot.exe` (or `rust\target\release\rot.exe`), built with
`cd rust && cargo build --release`, and appends every launch to
`logs\launcher.log`; panics are written to `logs\ui-crash.log`.

```
 PerProcessProxy   PROXY: ON   #12 1.2.3.4:1080   rotate manual
 --------------------------------------------------------------------
 PROXIES   248 total, 37 shown   filter: working
 > *  12  1.2.3.4:1080                            ok 240ms 91.2.3.4
      13  5.6.7.8:8080                            FAIL timeout
 --------------------------------------------------------------------
 PROCESSES   1 alive / 1 recorded
       6672  Telegram.exe              alive  #12  seen 3s ago
 --------------------------------------------------------------------
 [v]paste [a]attach/kill [l]launch [t]test [n]next [p]prev [o]on/off [r]rotate
 [/]search [f]filter [d]delete [c]clean [?]help [q]quit
```

| key | effect |
|---|---|
| `j` / `k`, arrows | move in the list |
| `PgUp` / `PgDn`, `Home` / `End` | page and jump |
| `Enter` | use the selected proxy |
| `n` / `p` | next / previous proxy |
| `v` | paste proxy lines (any format below) or an `http(s)` link to a list; `F2` applies, `Esc` cancels |
| `a` | attach to a running process; type a pid / exe name, `x` kills the highlighted one |
| `l` | launch a program already injected |
| `t` | test every proxy in the background - the screen stays usable |
| `o` / `r` | proxying on/off, automatic rotation interval |
| `/` / `f` | search, cycle filter (all / working / untested / failed / same-ip) |
| `d` / `c` | delete the selected proxy, drop state files of dead processes |
| `?` / `q` | help, quit |

Hotkeys work in both layouts (`v` and `м`, `a` and `ф`, ...), so the console
never needs a keyboard layout switch.

The command-driven console from earlier versions is still available:
`rot console`.

---

## Commands

| command | what it does |
|---|---|
| `status` | settings, current proxy, injected processes and liveness |
| `list` | proxies from the database, `*` marks the active one |
| `sync` / `import` | merge every `proxy/*.txt` into the database |
| `add <spec...>` | append proxies (deduplicated) |
| `import <file>` | import one specific file |
| `test [--prune] [--concurrency N]` | connect through every proxy, print exit IP + latency, flag ones whose IP equals your own, optionally delete the failures |
| `use N` / `next` | switch now |
| `auto N` / `auto 0` | rotate every N seconds / stop |
| `on` / `off` | proxy / pass everything straight through |
| `rm N...` | delete entries by index |
| `inject --name X.exe` | inject all running instances |
| `inject --pid 1234` | inject one process |
| `inject --launch "cmd"` / `launch "cmd"` | start a program already injected |
| `watch X.exe` | keep injecting new instances as they appear |
| `kill <pid>` | terminate an injected process |
| `clean` | delete state files of processes that are gone |
| `doctor` | environment / DNS / Clash checks |
| `ui` | interactive console with hotkeys (also the default with no arguments) |
| `console` | plain-text console driven by typed commands |

Accepted proxy formats:

```
socks5://user:pass@1.2.3.4:1080
socks4://1.2.3.4:1080
http://user:pass@1.2.3.4:8080
1.2.3.4:8080
1.2.3.4:8080:user:pass       <- WebShare-style list, 3 colons
1.2.3.4:1080                 <- 1080/1081/9050/9150 guessed as socks5, others http
```

---

## Files and folders

| path | owner | purpose |
|---|---|---|
| `bin/` | build | `inject.exe`, `proxyinject.dll`, `proxyinject.so`, `proxyinject.dylib` |
| `proxies.db` | the panel | SQLite database with the proxy list (table `proxies`); created on first run, recreated empty if deleted. The DLL and the POSIX shim open it read-only |
| `proxy/*.txt` | you | intake folder, merged on start and by `import` |
| `state/<pid>.txt` | DLL / shim | pid, exe, current proxy, enabled, last rotation |
| `state/health.json` | CLI | last test result per proxy |
| `logs/<pid>.log` | DLL / shim | per-process event log |
| `proxyinject.so` / `.dylib` | build | POSIX shim loaded by the launcher |

`state/` and `logs/` are disposable: deleting them never loses settings. State
files of dead processes are removed by `clean`, and the old
`state_<pid>.txt` files in the repository root are migrated into `state/` on
the first run.

Binaries live in `bin/`. The DLL finds `proxies.db` through `PROXYINJECT_DIR`,
then its own folder, then the parent folder - so the repository root stays the
working directory even though the DLL is one level down.

---

## Settings (table `settings` in `proxies.db`, live-reloaded)

`rot config` shows them, `rot set <key> <value>` changes one (for example
`rot set fail_closed 0`); running processes follow within about a second.
Defaults are written on the first run. An old `config.ini` is imported once and removed.

| key | default | meaning |
|---|---|---|
| `enabled` | 1 | 1 = proxy, 0 = transparent pass-through |
| `fail_closed` | 1 | if the proxy fails, refuse the connection instead of leaking your real IP |
| `block_udp` | 1 | block UDP so QUIC falls back to TCP (which we can proxy). DNS/loopback/private are still allowed |
| `allow_dns_udp` | 1 | keep UDP/53 working |
| `rotate_sec` | 0 | automatic rotation interval, 0 = manual only |
| `retry_next` | 1 | try the next proxy when one fails |
| `timeout_ms` | 10000 | connect + handshake timeout |
| `hook_connectex` | 1 | also cover apps using async `ConnectEx` (Node/Electron/.NET) |
| `default_scheme` | auto | scheme for bare `host:port` lines |
| `current` | 0 | index into the proxy list (managed by `rot`) |
| `log` | 1 | write `logs\<pid>.log` |

Each injected process also writes `state/<pid>.txt`, which the console uses to
show whether the process is still alive.

---

## What it does about Clash / fake-ip

Clash Verge with TUN + `enhanced-mode: fake-ip` makes every hostname resolve to
`198.18.x.x`. Sending such an address to a public proxy is useless - the proxy
would try to connect to `198.18.x.x` itself.

So the DLL also wraps `getaddrinfo` / `GetAddrInfoW` / `GetAddrInfoExW` /
`gethostbyname`, remembers which hostname produced which address, and sends the
**hostname** to the proxy (`SOCKS5 ATYP=3` / `CONNECT host:port`). The exit node
resolves it. That also means DNS is resolved at the exit, so no local DNS leak
and no fake-ip breakage.

Your Clash TUN stays untouched for all other processes - this tool never changes
routing, DNS or adapters.

---

## Platform support

| platform | how it hooks | attach to a running app | status |
|---|---|---|---|
| Windows x64 | `proxyinject.dll` injected by `inject.exe` | yes (`--pid`, `--name`) | ready |
| Linux x64 / arm64 | `proxyinject.so` via `LD_PRELOAD` | no (launch only) | ready |
| macOS arm64 / x64 | `proxyinject.dylib` via `DYLD_INSERT_LIBRARIES` | no (launch only) | ready (arm64 and x64 under Rosetta) |

The POSIX shim uses the same `proxies.db`, `state/` and `logs/`,
so the console and the proxy list are shared. Start a target with:

```sh
./run.sh launch "curl -sS http://checkip.amazonaws.com"
./run.sh launch "/path/to/app --flag"
```

`run.sh` sets the environment itself:

```sh
LD_PRELOAD=/path/to/PerProcessProxy/bin/proxyinject.so PROXYINJECT_DIR=/path/to/PerProcessProxy app
DYLD_INSERT_LIBRARIES=/path/to/PerProcessProxy/bin/proxyinject.dylib PROXYINJECT_DIR=/path/to/PerProcessProxy app
```

Known limits on POSIX:

- Only `connect()` and `getaddrinfo()` are hooked: hand-written DNS (`raw` UDP),
  `sendto()`-based UDP traffic and statically linked binaries are out of reach.
- UDP is refused except DNS and loopback, so QUIC falls back to TCP.
- The socket the app holds becomes the socket to the proxy: `getpeername()`
  reports the proxy address.
- Attaching to an already running process needs `ptrace` and is not implemented.
- macOS: binaries signed with hardened runtime, notarised apps, and system
  binaries protected by SIP (`/usr/bin/curl`, `/usr/bin/python3`, `/usr/bin/arch`)
  ignore `DYLD_INSERT_LIBRARIES`. Use Homebrew or self-built tools as targets
  (`brew install curl` puts one in `/opt/homebrew/opt/curl/bin`). The dylib is
  ad-hoc signed; Apple Silicon requires that at minimum.
- macOS: hooks are installed through the `__DATA,__interpose` section, so only
  `connect()` / `getaddrinfo()` callers are covered; Network.framework and
  `NSURLSession` traffic does not go through them.
- macOS: an app that honours the system proxy (for example `127.0.0.1:7890` from
  Clash) connects to loopback, which the shim passes through untouched.
- Applications started before the shim are not affected; that is the point.

---

## Reality check

* **Public proxies are mostly dead.** `rot test --prune` before you
  rely on a list. In Russia many proxy IPs/ports are throttled or blocked by
  TSPU, which looks like a handshake timeout (`proxy failed rc=-9/-10` in the
  log) - the tool retries the next proxy, and with `fail_closed=1` it never
  falls back to your real IP.
* **UDP is not proxied.** It is blocked (except DNS) so apps fall back to TCP.
  UDP-only protocols will not work.
* **TLS SNI is visible to the proxy operator**, and the size/timing of the
  traffic is visible to your ISP. Only the destination IP and DNS are hidden
  from the network.
* **Apps that fetched `ConnectEx` before injection** can bypass the hook; set
  `hook_connectex=1` (default) and inject as early as possible
  (`rot inject --launch` or `rot watch`).
* **Antivirus.** DLL injection is the same technique malware uses; Defender may
  quarantine the DLL. Add the folder to exclusions. Only inject into software
  you trust.
* **Elevated targets** need an elevated injector. Run PowerShell as admin in
  that case.
* The DLL is never unloaded. `off` turns it into a pass-through, which is
  equivalent for practical purposes.
* 32-bit targets are detected and refused - build `inject32.exe` from the same
  sources with `-target i386-windows-gnu` if you need them.

---

## Local self-test (no internet, no real proxy needed)

```powershell
# terminal 1: a fake SOCKS5 exit that claims a chosen IP
python tests\testsocks.py --port 11080 --fake-ip 10.9.9.9
python tests\testsocks.py --port 11081 --fake-ip 10.9.9.10

# terminal 2: end-to-end check inside one process
.\bin\inject.exe --selftest --proxy socks5://127.0.0.1:11080 --url http://checkip.amazonaws.com/
#   proxy=socks5://127.0.0.1:11080
#   exit_ip=10.9.9.9
#   result=OK

# live rotation demo: a poller that prints the IP every second
python tests\poller.py checkip.amazonaws.com / 1 out.txt
.\bin\inject.exe --pid <that pid>
rot use 1      # out.txt flips from 10.9.9.9 to 10.9.9.10 within ~1s
```

---

## How it works (for the curious)

* 5-byte `E9 rel32` inline patches on `ws2_32` prologues (`connect`,
  `WSAConnect`, `sendto`, `WSASendTo`, and the four resolvers), with executable
  stubs placed in a page allocated within ±2 GB of `ws2_32`.
* Each hooked function is fully reimplemented, so no instruction-length
  disassembly or trampolines are needed.
* `ConnectEx` is reached by rewriting the `WSAIoctl` slot in every loaded
  module's import table (re-swept every 3 s), because `mswsock!ConnectEx` is not
  exported by name.
* A second, private copy of `ws2_32.dll` (in `%TEMP%\proxyinject`) keeps the
  original resolvers callable, which is what makes the fake-ip mapping possible.
* Upstream sockets are connected with the real `ConnectEx` captured before any
  patching.
* Proxy pools rotate inside the process; the files are the only interface, so
  `rot` (or Notepad) works the same.

## Rebuilding

Windows:

```powershell
.tools\ziglang\zig.exe cc -target x86_64-windows-gnu -O2 -shared `
  -o bin\proxyinject.dll src\proxyinject.c -lws2_32

.tools\ziglang\zig.exe cc -target x86_64-windows-gnu -O2 `
  -o bin\inject.exe src\inject.c -lws2_32 -lpsapi
```

POSIX shim (same source builds for every target, pick what you need):

```powershell
# Linux x86_64
.tools\ziglang\zig.exe cc -target x86_64-linux-gnu -O2 -shared -fPIC `
  -o bin\proxyinject.so src\proxyinject_posix.c -ldl

# Linux arm64
.tools\ziglang\zig.exe cc -target aarch64-linux-gnu -O2 -shared -fPIC `
  -o bin\proxyinject-aarch64.so src\proxyinject_posix.c -ldl

```

macOS (universal arm64 + x86_64, on a Mac with the Xcode command line tools;
see `.agents/commands.md` for the SQLite flags):

```sh
cc -arch arm64 -arch x86_64 -mmacosx-version-min=11.0 -O2 -dynamiclib "$HID" $SQ \
  -o bin/proxyinject.dylib src/proxyinject_posix.c src/sqlite/sqlite3.c
codesign -s - -f bin/proxyinject.dylib
```

Verify the POSIX shim without internet, then with a real proxy:

```sh
sh tests/posix_selftest.sh     # fake SOCKS5 exit, expects exit_ip=10.9.9.9
sh tests/posix_live.sh         # first credentialed proxy from proxies.db
```

No compiler installed? `pip install --target .tools ziglang` (it ships a
complete Zig toolchain, about 200 MB, no admin needed).
