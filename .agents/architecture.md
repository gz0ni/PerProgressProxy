# Architecture

## Entry Point

- `run.cmd` / `run.sh` - launchers, call `rot.py` with the current directory set
  to the repository root.
- `rot.py` - thin wrapper over `ppp.cli.main`. No arguments starts the
  interactive console (`ppp.tui` on a TTY, `ppp.ui` otherwise).
- `bin/` - build output: `inject.exe`, `proxyinject.dll`, `proxyinject.so`,
  `proxyinject.dylib`.

## Data Folder Resolution

- Windows DLL: `PROXYINJECT_DIR` -> its own folder -> parent of its own folder;
  the first one that contains `config.ini` wins. This keeps `bin/` as the binary
  location and the repository root as the data location.
- POSIX shim: same chain, with `dladdr` used for its own folder.

## Core Layer

### `ppp/specs.py`

- One parser for every proxy line format: URL, `host:port`, `host:port:user:pass`.
- Must stay in sync with `parse_proxy_spec()` in `src/proxyinject.c`; when the C
  side changes, change this file and `tests/test_specs.py` too.

### `ppp/store.py`

- Owns `config.ini` (in-place key updates, comments preserved) and
  `proxies.txt` (canonical list the DLL reads).
- `sync_proxy_dir()` scans `proxy/*.txt`, parses every token, deduplicates by
  canonical label and appends the new ones.

### `ppp/processes.py`

- Reads `state/<pid>.txt` written by the DLL, reports liveness and image name,
  kills a target, removes stale state files.
- Handles the legacy layout: `state_<pid>.txt` in the root is migrated into
  `state/` on startup.
- Windows liveness via `OpenProcess` + `GetExitCodeProcess`; POSIX via
  `os.kill(pid, 0)`.

### `ppp/platform.py`

- Injection backend. Windows shells out to `inject.exe`
  (`--launch` / `--name` / `--pid` / `--list`).
- POSIX starts the target with `LD_PRELOAD` / `DYLD_INSERT_LIBRARIES` plus
  `PROXYINJECT_DIR`; attaching to a running process raises `UnsupportedPlatform`.

### `ppp/tui/`

- Interactive console: `term.py` (raw input/size/ANSI), `view.py` (pure render
  of a `ViewState`: header, scrollable proxy viewport, process list, footer,
  modal overlays), `model.py` (`Screen` snapshot, filters, selection, background
  `TestRun`), `app.py` (main loop, hotkeys, modals).
- Never writes files directly: all mutations go through `ppp.store`,
  `ppp.processes`, `ppp.platform`.
- `v` accepts proxy lines and `http(s)` list URLs (stdlib `urllib`, 5 MB cap);
  downloads run in a thread and report through `Screen.notice`.

### `ppp/ui.py`

- The older command-driven console, kept for scripts and muscle memory
  (`python rot.py console`). Redraws after every typed command.

## Feature Layer

### C shim (`src/proxyinject.c`)

- Inline `E9 rel32` patches on `ws2_32` prologues; a private copy of `ws2_32.dll`
  keeps the original resolvers callable.
- Hooks `connect`, `WSAConnect`, `sendto`, `WSASendTo`, the four resolvers, and
  `ConnectEx` through the `WSAIoctl` import slot.
- Watches `config.ini` / `proxies.txt` signatures every 400 ms and drops live
  sockets on rotation.
- Writes `state/<pid>.txt` with `pid`, `exe`, `current`, `proxies`, `enabled`,
  `rotated_ms`.

### POSIX shim (`src/proxyinject_posix.c`)

- Hooks `connect()` and `getaddrinfo()` with `dlsym(RTLD_NEXT, ...)`; no inline
  patching, so it works on any architecture the loader supports.
- Resolves the repository folder from `PROXYINJECT_DIR` or its own `dladdr` path,
  then reads the same `config.ini` / `proxies.txt`.
- Turns the app's socket into the socket to the proxy with `dup2`, so the app's
  `getpeername()` reports the proxy.
- Same file layouts: `state/<pid>.txt`, `logs/<pid>.log`.
- Built with zig for `x86_64-linux-gnu`, `aarch64-linux-gnu`, `aarch64-macos`.

### Injector (`src/inject.c`)

- `--launch` starts the target suspended, injects, resumes; `--pid` and `--name`
  attach to running processes; `--selftest` loads the DLL into itself.

## Data Flow

```
proxy/*.txt --(sync_proxy_dir)--> proxies.txt --+
config.ini ------------------------------------+--> injected process --> proxy
state/<pid>.txt <------- DLL ------------------+--> ppp.processes --> UI
```

## Known Gaps

- No attaching to running processes on Linux/macOS.
- 32-bit Windows targets need a separately built `inject32.exe`.
- UDP is blocked rather than proxied; on POSIX only `connect()` is hooked, so
  `sendto()` datagrams bypass the shim.
- macOS build is not verified on hardware.

## Roadmap

1. macOS verification and signing notes.
2. Optional latency history per proxy in `state/`.
