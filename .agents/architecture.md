# Architecture

## Entry Point

- `run.cmd` / `run.sh` - launchers, start `bin/rot` or `rust/target/release/rot`
  with the current directory set to the repository root.
- `rust/` - the control panel crate (binary `rot`). No arguments starts the
  interactive console (`tui` on a TTY, a hint and exit code 2 otherwise).
- `bin/` - build output of the C parts: `inject.exe`, `proxyinject.dll`,
  `proxyinject.so`, `proxyinject.dylib`.

## Data Folder Resolution

- Panel: `PPP_ROOT` -> cwd if it holds `proxies.db` (or a legacy `config.ini`) ->
  parent of `bin/` when the exe lives there -> first parent of the exe that does -> cwd.
- Windows DLL: `PROXYINJECT_DIR` -> its own folder -> parent of its own folder;
  the first one that contains `proxies.db` wins. This keeps `bin/` as the binary
  location and the repository root as the data location.
- POSIX shim: same chain, with `dladdr` used for its own folder.

## Core Layer (`rust/src/`)

### `specs.rs`

- One parser for every proxy line format: URL, `host:port`, `host:port:user:pass`.
- Must stay in sync with `parse_proxy_spec()` in `src/proxyinject.c`; when the C
  side changes, change this file and its tests too.

### `store.rs`

- `Store` owns everything persistent in SQLite `proxies.db` (rusqlite, bundled):
  `proxies(id, text)`, `settings(key, value)` and `meta`. The DB is created on
  first use and recreated empty if deleted; defaults are seeded into `settings`.
  A legacy `proxies.txt` / `config.ini` is imported once (`meta.migrated`,
  `meta.settings_seeded`) and then deleted.
  The journal mode stays DELETE (not WAL) so the file signature changes on
  every write; the DLL/shim reload on that signature.
- `sync_proxy_dir()` scans `proxy/*.txt`, parses every token, deduplicates by
  canonical label and appends the new ones.

### `processes.rs`

- Reads `state/<pid>.txt` written by the DLL, reports liveness and image name,
  kills a target, removes stale state files; migrates legacy `state_<pid>.txt`.
- Windows liveness via `OpenProcess` + `GetExitCodeProcess`; POSIX via `kill(pid, 0)`.

### `backend.rs`

- Injection backend. Windows shells out to `inject.exe`
  (`--launch` / `--name` / `--pid` / `--list`).
- POSIX starts the target with `LD_PRELOAD` / `DYLD_INSERT_LIBRARIES` plus
  `PROXYINJECT_DIR`; attaching to a running process returns an error.

### `netcheck.rs`

- SOCKS5 / SOCKS4 / HTTP CONNECT handshakes to the echo host, worker pool,
  `state/health.json` load/save.

### `cli.rs`, `main.rs`

- clap subcommands; `main` catches panics, logs them to `logs/ui-crash.log`.
  Only these handlers and `tui` mutate files, through `Store` / `processes`.

### `tui/`

- `term.rs` (raw input, `ReadConsoleInputW` on Windows, termios on POSIX, key
  decoding as pure functions), `view.rs` (pure render of a `ViewState`),
  `model.rs` (`Screen` snapshot, filters, selection, background `TestRun`),
  `app.rs` (main loop, hotkeys, modals; generic over the `Term` trait so tests
  use a fake terminal).
- `v` accepts proxy lines and `http(s)` list URLs (ureq, 5 MB cap); downloads run
  in a thread and report through `Screen.notice`.

### `ui.rs`

- The older command-driven console (`rot console`). Redraws after every command.

## Feature Layer

### C shim (`src/proxyinject.c`)

- Inline `E9 rel32` patches on `ws2_32` prologues; a private copy of `ws2_32.dll`
  keeps the original resolvers callable.
- Hooks `connect`, `WSAConnect`, `sendto`, `WSASendTo`, the four resolvers, and
  `ConnectEx` through the `WSAIoctl` import slot.
- Watches the `proxies.db` signature every 400 ms and drops live
  sockets on rotation.
- Writes `state/<pid>.txt` with `pid`, `exe`, `current`, `proxies`, `enabled`,
  `rotated_ms`.

### POSIX shim (`src/proxyinject_posix.c`)

- Hooks `connect()` and `getaddrinfo()` with `dlsym(RTLD_NEXT, ...)`; no inline
  patching, so it works on any architecture the loader supports.
- Resolves the repository folder from `PROXYINJECT_DIR` or its own `dladdr` path,
  then reads the same `proxies.db`.
- Turns the app's socket into the socket to the proxy with `dup2`, so the app's
  `getpeername()` reports the proxy.
- Same file layouts: `state/<pid>.txt`, `logs/<pid>.log`.
- Built with zig for `x86_64-linux-gnu`, `aarch64-linux-gnu`, `aarch64-macos`.

### Injector (`src/inject.c`)

- `--launch` starts the target suspended, injects, resumes; `--pid` and `--name`
  attach to running processes; `--selftest` loads the DLL into itself.

## Data Flow

```
proxy/*.txt --(sync_proxy_dir)--> proxies.db --+
rot set / config (settings table) -------------+--> injected process --> proxy
state/<pid>.txt <------- DLL ------------------+--> processes --> UI
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
