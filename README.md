<div align="center">

# PerProcessProxy

**Route one app through a proxy. Leave the rest of the machine alone.**

![Windows](https://img.shields.io/badge/Windows-x64-0078D6?logo=windows&logoColor=white)
![Linux](https://img.shields.io/badge/Linux-x64%20%7C%20arm64-FCC624?logo=linux&logoColor=black)
![macOS](https://img.shields.io/badge/macOS-arm64%20%7C%20x64-000000?logo=apple&logoColor=white)
![Rust](https://img.shields.io/badge/panel-Rust-B7410E?logo=rust&logoColor=white)
![C](https://img.shields.io/badge/shim-C11-A8B9CC?logo=c&logoColor=black)

**English** · [Русский](README.ru.md) · [Usage guide](docs/USAGE.md) · [Инструкция](docs/USAGE.ru.md)

</div>

---

PerProcessProxy sends the TCP traffic of **one chosen program** through a
SOCKS5 / SOCKS4 / HTTP proxy. The browser, games, Clash and the OS keep their
normal route. The proxy can be switched while the program runs, with no restart,
and the new exit IP shows up within about a second.

```
 app ──connect(example.com:443)──▶ [ proxyinject ] ──SOCKS5 / HTTP──▶ 1.2.3.4:1080 ──▶ example.com
 everything else ─────────────────────────────────────────────────────────────────▶ normal route
```

## Features

- **Per-process.** Only the injected process is affected. No TUN, VPN, routing
  or DNS changes.
- **Live switching.** Running processes re-read the proxy list and settings in
  under a second and drop old sockets on rotation.
- **Auto-rotation.** Change the exit every N seconds, or step through the list
  by hand.
- **Fail-closed.** If the proxy fails, the connection is refused instead of
  leaking your real IP.
- **Works with Clash fake-ip.** The hostname, not the `198.18.x.x` address, goes
  to the proxy, so DNS is resolved at the exit.
- **Built-in checker.** `rot test` connects through every proxy, shows latency
  and exit IP, flags proxies that leak your own IP, and can delete dead ones.
- **Any list format.** `host:port`, `host:port:user:pass`, `socks5://…`,
  `http://…`. Drop files into `proxy/` and they get imported.
- **Keyboard console.** Scrollable list, background tests, attach / launch /
  kill, filters and search. Hotkeys work in both English and Russian layouts.
- **No services.** One SQLite file (`proxies.db`), no daemon, no IPC, no admin
  rights for normal apps.

## Quick start

Prebuilt binaries are in `bin/`. Nothing needs to be installed.

<table>
<tr><th>Windows</th><th>Linux / macOS</th></tr>
<tr><td>

```powershell
cd C:\path\to\PerProcessProxy
.\run.cmd                      # console
.\run.cmd add socks5://1.2.3.4:1080
.\run.cmd test --prune
.\run.cmd on
.\run.cmd attach Telegram.exe
```

</td><td>

```sh
cd /path/to/PerProcessProxy
./run.sh                       # console
./run.sh add socks5://1.2.3.4:1080
./run.sh test --prune
./run.sh on
./run.sh run /path/to/app
```

</td></tr>
</table>

Then switch the exit at any moment:

```sh
rot next          # next proxy
rot use 7         # proxy #7
rot auto 120      # rotate every 2 minutes
rot off           # pass everything straight through
rot status        # what is applied, which processes are alive
```

`rot` here means `run.cmd` / `run.sh`. The full walkthrough, with every command,
hotkey and setting, is in the **[usage guide](docs/USAGE.md)**.

## Console

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

## Platforms

| Platform | How it hooks | Attach to running app | Status |
|---|---|---|---|
| Windows x64 | `proxyinject.dll` injected by `inject.exe` | ✅ by pid or exe name | ready |
| Linux x64 / arm64 | `proxyinject.so` via `LD_PRELOAD` | ❌ launch only | ready |
| macOS arm64 / x64 | `proxyinject.dylib` via `DYLD_INSERT_LIBRARIES` | ❌ launch only | ready |

All platforms share the same `proxies.db`, `state/` and `logs/`, so one console
controls everything.

> [!NOTE]
> On macOS, SIP-protected system binaries (`/usr/bin/curl`, `/usr/bin/python3`, …)
> and hardened-runtime apps ignore `DYLD_INSERT_LIBRARIES`. Use Homebrew or
> self-built programs as targets, minimum macOS 13. Details are in the
> [usage guide](docs/USAGE.md#macos).

## Limits

- **UDP is not proxied.** It is blocked (DNS excepted), so QUIC falls back to
  TCP. UDP-only protocols do not work.
- **Public proxies are mostly dead.** Run `rot test --prune` before you rely on
  a list.
- **The proxy operator sees TLS SNI**, and your ISP sees traffic size and
  timing. Only the destination IP and DNS are hidden.
- **Antivirus may flag the DLL.** Injection is the same technique malware uses.
  Add the folder to exclusions and inject only into software you trust.
- **Elevated targets** on Windows need an elevated console.
- **32-bit Windows targets** are refused. Build `inject32.exe` with
  `-target i386-windows-gnu` if you need them.

## How it works

<details>
<summary>Windows DLL</summary>

- 5-byte `E9 rel32` inline patches on `ws2_32` prologues (`connect`,
  `WSAConnect`, `sendto`, `WSASendTo` and four resolvers). Stubs sit in a page
  within ±2 GB of `ws2_32`.
- Each hooked function is fully reimplemented, so no disassembler or
  trampolines are needed.
- `ConnectEx` is caught by rewriting the `WSAIoctl` import slot in every loaded
  module, re-swept every 3 s.
- A private copy of `ws2_32.dll` keeps the original resolvers callable. That is
  what makes the hostname mapping for Clash fake-ip possible.
- The proxy database signature is checked every 400 ms. Live sockets are
  dropped on rotation.

</details>

<details>
<summary>Linux / macOS shim</summary>

- Hooks `connect()` and `getaddrinfo()`. Linux overrides the symbols and uses
  `dlsym(RTLD_NEXT)`. macOS registers the hooks in `__DATA,__interpose`.
- The app's socket becomes the socket to the proxy (`dup2`), so there is no
  relay thread.
- SQLite is compiled in with hidden symbols, so it never clashes with the
  target's own libsqlite3.

</details>

## Project layout

| Path | Purpose |
|---|---|
| `bin/` | prebuilt binaries: `rot*`, `inject.exe`, `proxyinject.{dll,so,dylib}` |
| `rust/` | control panel source (`rot`) |
| `src/` | C sources of the DLL, injector and POSIX shim |
| `proxy/` | drop proxy lists here (`*.txt`) |
| `proxies.db` | proxy list and settings (SQLite, created on first run) |
| `state/`, `logs/` | per-process state and logs, safe to delete |
| `tests/` | self-tests and dev helpers |

## Building from source

<details>
<summary>Control panel (Rust)</summary>

```sh
cd rust
cargo test
cargo build --release          # rust/target/release/rot
```

macOS universal binary:

```sh
rustup target add x86_64-apple-darwin
for t in aarch64-apple-darwin x86_64-apple-darwin; do
  MACOSX_DEPLOYMENT_TARGET=11.0 cargo build --release --target $t
done
lipo -create -output ../bin/rot-macos \
  target/aarch64-apple-darwin/release/rot target/x86_64-apple-darwin/release/rot
```

</details>

<details>
<summary>DLL, injector and shims (C)</summary>

SQLite flags used by every build:

```sh
SQ="-DSQLITE_THREADSAFE=0 -DSQLITE_OMIT_LOAD_EXTENSION -DSQLITE_OMIT_DEPRECATED -DSQLITE_DEFAULT_MEMSTATUS=0 -DSQLITE_OMIT_SHARED_CACHE -DSQLITE_DQS=0 -DSQLITE_TEMP_STORE=3"
HID='-DSQLITE_API=__attribute__((visibility("hidden")))'
```

Windows (vendored Zig at `.tools/ziglang`):

```powershell
.tools\ziglang\zig.exe cc -target x86_64-windows-gnu -O2 -shared $sq.Split(' ') `
  -o bin\proxyinject.dll src\proxyinject.c src\sqlite\sqlite3.c -lws2_32
.tools\ziglang\zig.exe cc -target x86_64-windows-gnu -O2 `
  -o bin\inject.exe src\inject.c -lws2_32 -lpsapi
```

Linux:

```sh
zig cc -target x86_64-linux-gnu.2.17 -O2 -s -shared -fPIC "$HID" $SQ \
  -o bin/proxyinject.so src/proxyinject_posix.c src/sqlite/sqlite3.c -ldl
zig cc -target aarch64-linux-gnu.2.17 -O2 -s -shared -fPIC "$HID" $SQ \
  -o bin/proxyinject-aarch64.so src/proxyinject_posix.c src/sqlite/sqlite3.c -ldl
```

macOS (Xcode command line tools):

```sh
cc -arch arm64 -arch x86_64 -mmacosx-version-min=13.0 -O2 -dynamiclib "$HID" $SQ \
  -o bin/proxyinject.dylib src/proxyinject_posix.c src/sqlite/sqlite3.c
codesign -s - -f bin/proxyinject.dylib
```

No compiler? `pip install --target .tools ziglang` ships a complete Zig
toolchain (about 200 MB, no admin needed).

</details>

<details>
<summary>Self-tests (no internet, no real proxy)</summary>

```sh
sh tests/posix_selftest.sh     # Linux / macOS, expects exit_ip=10.9.9.9
```

```powershell
python tests\testsocks.py --port 11080 --fake-ip 10.9.9.9
.\bin\inject.exe --selftest --proxy socks5://127.0.0.1:11080 --url http://checkip.amazonaws.com/
# exit_ip=10.9.9.9, result=OK
```

</details>
