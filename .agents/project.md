# Project Context

Per-process proxy rotator. A DLL (`proxyinject.dll`) is injected into one
chosen executable only; it hooks that process's Winsock calls and sends its TCP
connections through a SOCKS5 / SOCKS4 / HTTP proxy from a flat text list. Every
other process on the machine keeps its normal route. The list and the settings
are plain files, re-read by the injected process within about a second, so the
exit IP can be switched without restarting the target.

Implementation: a C shim (`src/proxyinject.c`) plus a small C injector
(`src/inject.c`), driven by a Rust control panel (`rust/`, binary `rot`).

## Platforms

- Windows x86_64 - supported (DLL injection via `inject.exe`).
- Linux x86_64 / arm64 - supported via `proxyinject.so` and `LD_PRELOAD`
  (launch-only, no attaching to running processes).
- macOS arm64 / x64 - `proxyinject.dylib` built and wired up, not yet verified on
  real hardware; SIP and hardened runtime limit what can be injected.

## Toolchain

- Rust (stable, edition 2024) for the control panel; crates: clap, serde_json,
  ureq, windows-sys / libc. Python is needed only for the dev helpers `tests/testsocks.py` and
  `tests/poller.py`.
- C11, built with the Zig toolchain bundled at `.tools/ziglang/zig.exe` (0.16.0).
- Target triple for Windows builds: `x86_64-windows-gnu`.

## Package / Repository

- `name:` PerProcessProxy
- `version:` unversioned, file-based
- `org:` personal project, not a git repository

## Dependencies

- Runtime: none beyond the built `rot` binary and the prebuilt binaries in `bin/`.
- Proxy lists come from outside (`proxy/`), usually public or paid proxy dumps.

## Design Basis

The proxy list lives in an embedded SQLite file `proxies.db` (created on first
run, recreated empty if deleted). The panel writes it; the injected DLL/shim
open it read-only (SQLite is compiled into them, symbols hidden). Everything
else is plain files: `state/<pid>.txt`, `logs/<pid>.log`. Settings are the
`settings` table of the same database (no `config.ini`).
No IPC, no daemon, no database server.

## Status

- Working: injection, SOCKS4/5 + HTTP CONNECT, hostname pass-through for
  fake-ip DNS, live reload, rotation, hotkey console (`rust/src/tui`), legacy
  command console, `proxy/` auto-import, `state/` folder, POSIX shim (verified
  in WSL against a fake SOCKS5 exit and a real HTTP proxy).
- Not started: UDP proxying, attaching to existing POSIX processes, macOS field
  testing.
