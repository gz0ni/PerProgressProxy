# Project Decisions

Non-trivial choices and why. Newest first.

## Format

```
### <Decision title> (YYYY-MM-DD)

- Decision: <what was chosen, one sentence>
- Reason: <why, compared with the alternatives>
- Alternatives considered: <rejected options and why>
- Consequences: <what this affects>
```

---

### macOS binaries built natively, universal (2026-11-05)

- Decision: `bin/proxyinject.dylib` and `bin/rot-macos` are built on a Mac with
  Apple clang / cargo as arm64 + x86_64 fat binaries, minimum macOS 11, ad-hoc signed.
- Reason: the Rust panel needs the Apple SDK to link; one fat file covers Intel
  Macs and Rosetta targets (an x86_64 app needs an x86_64 slice in the dylib).
- Alternatives considered: zig `aarch64-macos` only (no x86_64 slice, no signature
  step, and the panel still needed a Mac); separate per-arch files (more cases in
  `run.sh` and `backend.rs`).
- Consequences: the dylib grew to about 2.4 MB, `rot-macos` to about 9 MB; rebuild
  both on a Mac after touching `src/proxyinject_posix.c` or `rust/`.


### Settings moved from config.ini into the database (2026-11-05)

- Decision: table `settings(key, value)` in `proxies.db` replaces `config.ini`; `rot config` / `rot set` edit it. The DLL and shim render the table to `key=value` lines and reuse their old parsers.
- Reason: requested; `config.ini` was tracked in git and showed runtime state (`current`) as a permanent diff, and a second source of truth next to the DB.
- Alternatives considered: untracking `config.ini` and generating it on first run (smaller, but keeps the file); a `rot.toml` for the panel only (C side still needs the values).
- Consequences: no hand editing of settings (use `rot set`, only the 11 known keys are accepted); an old `config.ini` is imported once and removed; the data-folder marker for the DLL/shim and for `rot` is now `proxies.db`; one file signature (`proxies.db`) triggers reloads of both settings and proxies.

### Proxy list lives only in SQLite, read by the DLL (2026-11-05)

- Decision: `proxies.db` (SQLite) is the only store of the list; `proxies.txt` is gone. The DLL and the POSIX shim embed SQLite (`src/sqlite/`, amalgamation 3.54.1) and read the table read-only.
- Reason: requested ("txt is a disgrace"). MySQL/Postgres need a server and cannot be created on first run; SQLite is one file with no install.
- Alternatives considered: keeping a generated `proxies.txt` for the C side (tried first, rejected by the user); a custom binary file (still a file, no gain); named pipe / IPC (new moving parts).
- Consequences: reverses the old "no database" rule; C binaries grew to about 1.2 MB each; shim builds hide SQLite symbols (`SQLITE_API` hidden) so they never interpose the target's own libsqlite3; journal mode must stay DELETE so file mtime/size changes on every write; a busy database keeps the previous list and retries on the next 400 ms tick; deleting the DB loses the list (it is recreated empty); an existing `proxies.txt` is imported once and deleted.

### Control panel ported from Python to Rust (2026-11-05)

- Decision: `rust/` crate `rot` replaces `rot.py` + `ppp/`; behaviour and file contract unchanged.
- Reason: single native binary, no interpreter probing in `run.cmd`, typed code.
- Alternatives considered: keeping Python (needs a working interpreter); a Rust-only std build (hand-rolled JSON/HTTPS/Win32 FFI is more risk than clap/serde_json/ureq/windows-sys).
- Consequences: Python panel deleted after acceptance; `run.cmd` / `run.sh` start the Rust binary; `tests/testsocks.py` and `tests/poller.py` stay as dev helpers. Known carry-overs from Python: TUI help/footer mention `j/k` but no handler exists; `proxies()` index counts invalid lines while `list` does not.

### Hotkey TUI replaces the command shell as the default console (2026-10-29)

- Decision: `python rot.py` / `run.cmd` / `run.sh` open `ppp/tui` - a raw-key
  console with modals and a scrollable proxy viewport; `ppp/ui.py` stays as
  `python rot.py console`.
- Reason: typing commands for daily use was the main complaint, and a long proxy
  list flooded the screen and blocked work. Hotkeys plus a fixed-height viewport
  solve both without touching the file contract.
- Alternatives considered: keeping the command shell and adding shortcuts
  (rejected: still needs typed commands, still redraws every proxy); curses/rich/
  textual (rejected: third-party deps against the project rules, curses is absent
  from the Windows standard library).
- Consequences: raw input decoding is now platform code (msvcrt scan codes vs
  ANSI CSI); `ppp/tui/view.py` must stay a pure function so layout is testable
  without a TTY; pasting a list URL adds a stdlib `urllib` download path (5 MB cap).

### Console is a command shell with Enter, not a raw-key TUI (2026-10-28, superseded)

- Decision: `python rot.py` starts a redraw-on-command shell; commands are typed
  and confirmed with Enter.
- Reason: works in any terminal on Windows/Linux/macOS with zero dependencies and
  no VT/raw-mode edge cases; the user still sees process liveness on every redraw.
- Alternatives considered: raw-key TUI (msvcrt/termios, fragile), Textual/rich
  (third-party deps, heavier than the tool itself).
- Consequences: no live clock; staleness is shown as "N s ago" per process.

### Windows first, POSIX shim as a separate stage (2026-10-28)

- Decision: finish the Windows console, `proxy/` import, and `state/` folder
  before writing the Linux/macOS shim.
- Reason: the Windows path is testable on the development machine; the POSIX shim
  needs a real Linux/macOS host for verification.
- Alternatives considered: all platforms at once (unverifiable code shipped blind).
- Consequences: `ppp.platform` raises `UnsupportedPlatform` outside Windows until
  the shim lands.

### Per-process state stays a flat file per PID, only moved to `state/` (2026-10-28)

- Decision: the DLL keeps writing key=value text, but into `state/<pid>.txt`.
- Reason: the DLL must publish state without the CLI being alive; a directory plus
  a tiny file is the smallest thing that works, and the UI derives the rest.
- Alternatives considered: SQLite registry (write contention with injected
  processes, migration cost), JSON per PID (no benefit over key=value here).
- Consequences: `state/` is disposable; stale files are pruned by the UI.

### Binaries live in `bin/`, data stays in the repository root (2026-10-28)

- Decision: `inject.exe`, `proxyinject.dll`, `proxyinject.so`, `proxyinject.dylib`
  live in `bin/`; `config.ini`, `proxies.txt`, `state/`, `logs/` stay in the root.
- Reason: the root was cluttered, but moving the data would break manual editing
  and every documented path.
- Alternatives considered: keep binaries in the root (clutter), move data into
  `data/` (rewrites every path and the documented workflow).
- Consequences: the DLL and the POSIX shim resolve the data folder as
  `PROXYINJECT_DIR` -> own folder -> parent; `inject.exe` finds the DLL next to
  itself, so the pair must stay together.

### POSIX shim: launch-only, socket replacement with dup2 (2026-10-28)

- Decision: the POSIX shim is loaded with `LD_PRELOAD` / `DYLD_INSERT_LIBRARIES`
  at launch and replaces the app socket by `dup2`; attaching to a running process
  is not supported.
- Reason: `connect()` is the one portable choke point; `ptrace` injection is
  unportable and blocked by macOS SIP, and a relay thread would double the
  syscalls.
- Alternatives considered: `gdb -p` injection (fragile, no macOS), a userspace
  relay proxy (needs an extra port and per-app configuration).
- Consequences: hand-written DNS and `sendto()` datagrams bypass the shim; UDP is
  refused so QUIC falls back to TCP.
