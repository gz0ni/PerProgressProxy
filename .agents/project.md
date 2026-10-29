# Project Context

Per-process proxy rotator. A DLL (`proxyinject.dll`) is injected into one
chosen executable only; it hooks that process's Winsock calls and sends its TCP
connections through a SOCKS5 / SOCKS4 / HTTP proxy from a flat text list. Every
other process on the machine keeps its normal route. The list and the settings
are plain files, re-read by the injected process within about a second, so the
exit IP can be switched without restarting the target.

Implementation: a C shim (`src/proxyinject.c`) plus a small C injector
(`src/inject.c`), driven by a standard-library-only Python control panel
(`rot.py` + `ppp/`).

## Platforms

- Windows x86_64 - supported (DLL injection via `inject.exe`).
- Linux x86_64 / arm64 - supported via `proxyinject.so` and `LD_PRELOAD`
  (launch-only, no attaching to running processes).
- macOS arm64 / x64 - `proxyinject.dylib` built and wired up, not yet verified on
  real hardware; SIP and hardened runtime limit what can be injected.

## Toolchain

- Python 3.8+, standard library only, no virtualenv needed.
- C11, built with the Zig toolchain bundled at `.tools/ziglang/zig.exe` (0.16.0).
- Target triple for Windows builds: `x86_64-windows-gnu`.

## Package / Repository

- `name:` PerProcessProxy
- `version:` unversioned, file-based
- `org:` personal project, not a git repository

## Dependencies

- Runtime: none beyond Python and the prebuilt binaries in `bin/`.
- Proxy lists come from outside (`proxy/`), usually public or paid proxy dumps.

## Design Basis

File-based contract only: `config.ini`, `proxies.txt`, `state/<pid>.txt`,
`logs/<pid>.log`. Any editor can change the settings while processes run, and
the console panel writes the same files. No IPC, no daemon, no database.

## Status

- Working: injection, SOCKS4/5 + HTTP CONNECT, hostname pass-through for
  fake-ip DNS, live reload, rotation, hotkey console (`ppp/tui`), legacy
  command console, `proxy/` auto-import, `state/` folder, POSIX shim (verified
  in WSL against a fake SOCKS5 exit and a real HTTP proxy).
- Not started: UDP proxying, attaching to existing POSIX processes, macOS field
  testing.
