# Usage guide

**English** · [Русский](USAGE.ru.md) · [← README](../README.md)

A step-by-step guide: from an empty folder to an app whose exit IP you switch
with one key.

- [1. First run](#1-first-run)
- [2. Add proxies](#2-add-proxies)
- [3. Check proxies](#3-check-proxies)
- [4. Turn proxying on](#4-turn-proxying-on)
- [5. Put an app behind the proxy](#5-put-an-app-behind-the-proxy)
- [6. Switch and rotate](#6-switch-and-rotate)
- [7. Make sure it works](#7-make-sure-it-works)
- [8. Stop](#8-stop)
- [Console hotkeys](#console-hotkeys)
- [Command reference](#command-reference)
- [Settings](#settings)
- [macOS](#macos)
- [Troubleshooting](#troubleshooting)

> In this guide `rot` means the launcher for your system:
> `.\run.cmd` on Windows, `./run.sh` on Linux and macOS.

---

## 1. First run

Open a terminal in the project folder and start the console:

```powershell
.\run.cmd          # Windows
```

```sh
./run.sh           # Linux / macOS
```

The first run creates `proxies.db` (the proxy list and settings) next to the
launcher. Run `rot doctor` to see what was found: binaries, database, your
direct IP, and whether Clash is listening.

> [!TIP]
> Any command can also be run once without opening the console:
> `rot status`, `rot list`, `rot next`, and so on.

## 2. Add proxies

Pick whichever is handier.

**A file.** Put any `*.txt` into `proxy/`. Everything there is merged into the
database on every start, or right away with:

```sh
rot import                     # the whole proxy/ folder
rot import my_list.txt         # one file
```

**The command line.**

```sh
rot add socks5://user:pass@1.2.3.4:1080 http://5.6.7.8:8080
```

**The console.** Press `v`, paste lines or an `http(s)://` link to a list,
then press `F2`.

Accepted line formats:

```
socks5://user:pass@1.2.3.4:1080
socks4://1.2.3.4:1080
http://user:pass@1.2.3.4:8080
1.2.3.4:8080
1.2.3.4:8080:user:pass        WebShare style
```

A bare `host:port` on 1080, 1081, 9050 or 9150 is treated as SOCKS5, any other
port as HTTP. Change that with `rot set default_scheme socks5`. Duplicates are
skipped.

## 3. Check proxies

```sh
rot test                       # check all, print latency and exit IP
rot test --prune               # also delete the ones that failed
rot test --concurrency 64 --timeout 8
```

In the console press `t`: the test runs in the background and the list fills
in as results arrive. Press `f` to show only working ones.

> [!WARNING]
> A proxy marked **same** returns your own IP. It is transparent and hides
> nothing. Delete it.

## 4. Turn proxying on

Proxying is **off** by default: injected apps pass traffic straight through
until you enable it.

```sh
rot on
```

In the console press `o`. The header changes to `PROXY: ON`.

## 5. Put an app behind the proxy

### Windows

```powershell
rot launch "C:\Program Files\App\app.exe"   # start it already injected
rot attach 1234                             # a running process, by pid
rot attach Telegram.exe                     # every running instance, by name
rot watch Telegram.exe                      # keep injecting new instances
```

In the console: `l` to launch, `a` to attach (type a pid or exe name).

> [!TIP]
> Starting with `launch` or `watch` is more reliable than attaching later:
> some apps (Node, Electron, .NET) set up their network stack at start-up.

### Linux and macOS

Only launching works there: the library is loaded when the program starts.

```sh
rot launch "/usr/bin/firefox --new-instance"
rot launch "/opt/homebrew/bin/curl -s http://checkip.amazonaws.com"
```

The app keeps running after `rot` exits. Stop it with `kill <pid>` or `rot kill <pid>`.

## 6. Switch and rotate

| Do | Command | Console |
|---|---|---|
| next / previous proxy | `rot next` | `n` / `p` |
| a specific proxy | `rot use 7` | select it, `Enter` |
| rotate every 2 minutes | `rot auto 120` | `r` |
| stop rotating | `rot auto 0` | `r` |
| bypass for a while | `rot off` / `rot on` | `o` |

Every running injected process picks up the change within about a second and
drops its open connections, so the app reconnects through the new exit.

## 7. Make sure it works

- `rot status` lists the injected processes, whether each is alive, and which
  proxy it uses.
- `logs/<pid>.log` records every connection: `proxy 3 example.com via 1.2.3.4:1080`.
- Open an IP check site inside the app, or launch a checker:
  `rot launch "curl -s http://checkip.amazonaws.com"` (Linux).

## 8. Stop

- `rot off` turns every injected process into a pass-through at once.
- Closing the app ends the effect. Nothing stays installed in the system.
- `rot clean` removes the state of processes that are gone. `state/` and
  `logs/` can be deleted at any time.

---

## Console hotkeys

Hotkeys work in both English and Russian layouts (`v` = `м`, `a` = `ф`, …).

| Key | Action |
|---|---|
| `↑` `↓`, `j` `k` | move in the list |
| `PgUp` `PgDn`, `Home` `End` | page, jump to start or end |
| `Enter` | use the selected proxy |
| `n` / `p` | next / previous proxy |
| `v` | paste proxies or a list URL; `F2` applies, `Esc` cancels |
| `a` | attach to a process; `x` kills the highlighted one |
| `l` | launch a program behind the proxy |
| `t` | test every proxy in the background |
| `o` | proxying on / off |
| `r` | auto-rotation interval |
| `/` | search |
| `f` | filter: all / working / untested / failed / same |
| `d` | delete the selected proxy |
| `c` | clean state of dead processes |
| `?` | help |
| `q` | quit |

The old console driven by typed commands is still there: `rot console`.

## Command reference

| Command | What it does |
|---|---|
| `rot` / `rot ui` | interactive console |
| `rot status` | settings, current proxy, injected processes |
| `rot list` | numbered proxy list, `*` marks the active one |
| `rot doctor` | binaries, database, direct IP, Clash |
| `rot add <spec>...` | add proxies |
| `rot import [file]` | import a file, or the whole `proxy/` folder |
| `rot sync` | merge `proxy/*.txt` into the database |
| `rot rm <N>...` | delete proxies by index |
| `rot test [--prune] [--concurrency N] [--timeout S]` | check every proxy |
| `rot use <N>` / `rot next` | switch now |
| `rot auto <N>` | rotate every N seconds, `0` stops |
| `rot on` / `rot off` | enable / bypass |
| `rot launch "<cmd>"` | start a program behind the proxy |
| `rot attach <pid\|exe>` | inject into a running process (Windows) |
| `rot inject --pid N \| --name X \| --launch "cmd" \| --list` | low-level injector access (Windows) |
| `rot watch <exe> [--interval S]` | inject new instances as they appear (Windows) |
| `rot kill <pid>` | terminate an injected process |
| `rot clean` | drop state files of dead processes |
| `rot config` | show all settings |
| `rot set <key> <value>` | change a setting |
| `rot console` | old typed-command console |

## Settings

Stored in `proxies.db`, applied to running processes within a second.
`rot config` shows them, `rot set <key> <value>` changes one.

| Key | Default | Meaning |
|---|---|---|
| `enabled` | `0` | `1` proxy, `0` pass-through |
| `fail_closed` | `1` | refuse the connection if every proxy failed, never fall back to your IP |
| `retry_next` | `1` | try up to 3 proxies in a row before giving up |
| `timeout_ms` | `10000` | connect + handshake timeout |
| `rotate_sec` | `0` | auto-rotation interval in seconds, `0` manual |
| `block_udp` | `1` | block UDP so QUIC falls back to TCP |
| `allow_dns_udp` | `1` | keep UDP port 53 working |
| `hook_connectex` | `1` | also cover `ConnectEx` (Node, Electron, .NET), Windows |
| `default_scheme` | `auto` | scheme for bare `host:port` lines: `auto`, `socks5`, `socks4`, `http` |
| `current` | `0` | active proxy index, managed by `rot use` / `next` |
| `log` | `1` | write `logs/<pid>.log` |

Environment variables:

| Variable | Effect |
|---|---|
| `PPP_ROOT` | data folder for `rot` (default: the project folder) |
| `PROXYINJECT_DIR` | data folder for the injected library (`rot launch` sets it) |
| `PROXYINJECT_FORCE` | ignore the database and use this one proxy, e.g. `socks5://127.0.0.1:1080` |
| `PROXYINJECT_LOG` | `1` / `0` overrides the `log` setting |

## macOS

The launcher uses the universal `bin/rot-macos` and `bin/proxyinject.dylib`
(Apple Silicon and Intel).

**What can be put behind the proxy.** macOS removes `DYLD_INSERT_LIBRARIES` for:

- system programs protected by SIP: everything in `/usr/bin`, `/bin`,
  `/System`, including `curl`, `python3` and `arch`;
- apps signed with hardened runtime without the
  `allow-dyld-environment-variables` entitlement (most App Store and notarised
  apps).

Homebrew tools, apps you build yourself, and many open-source apps work fine:

```sh
brew install curl
rot launch "/opt/homebrew/opt/curl/bin/curl -s http://checkip.amazonaws.com"
```

**What is not covered.** Only `connect()` and `getaddrinfo()` are hooked. Apps
that use Network.framework or `NSURLSession` (Safari and many native Cocoa apps)
go around them.

**System proxy.** If Clash or another tool sets the macOS system proxy
(`scutil --proxy` shows it), many apps connect to `127.0.0.1:7890` instead of
the real host. Loopback is never proxied, so such traffic goes through Clash,
not through PerProcessProxy. Turn off "use system proxy" in the app, or the
system proxy itself.

**Intel apps on Apple Silicon.** They run under Rosetta and work with the same
dylib. `arch -x86_64` is a system binary and drops `DYLD_*`, so pass variables
through it explicitly:
`arch -x86_64 -e DYLD_INSERT_LIBRARIES=... app`.

## Troubleshooting

| Symptom | What to check |
|---|---|
| IP did not change | `rot status`: is `PROXY: ON`? Is the process listed as alive? |
| Process not listed at all | the library was not loaded: on macOS see [above](#macos); on Windows run the console as admin for elevated apps |
| App says "no connection" | `fail_closed=1` and every proxy failed. Run `rot test --prune`, or `rot set fail_closed 0` to fall back to direct (leaks your IP) |
| `proxy failed rc=-9/-10` in the log | handshake timeout: the proxy is dead or blocked by the ISP. The next one is tried |
| Some requests bypass the proxy | the app uses UDP/QUIC (blocked, should fall back to TCP), or Network.framework on macOS, or attached too late on Windows: use `launch` / `watch` |
| Antivirus deletes `proxyinject.dll` | add the project folder to exclusions |

Logs: `logs/<pid>.log` per injected process, `logs/ui-crash.log` for console
crashes, `logs/launcher.log` for every `run.cmd` start on Windows.
