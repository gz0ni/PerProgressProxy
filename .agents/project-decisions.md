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
