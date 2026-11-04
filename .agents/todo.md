# TODO

Task checklist. Keep it honest: mark `[x]` only after verification.

## Open

- [ ] Manual TUI check on Windows Terminal: scroll a long `proxies.txt`, `v`
      paste (lines and a list URL), background `t` test, window resize.
- [ ] Manual attach-modal check: `a`, hold arrows, move the mouse, switch layout
      with Alt+Shift - the modal must stay open; `/` filter, `Esc` closes once.
- [ ] macOS field test: build the dylib, run `tests/posix_selftest.sh` on real
      hardware, document signing / SIP limits.
- [ ] Optional latency history per proxy, kept in `state/`.

## In Progress

- [~] Windows console input rework (`ReadConsoleInputW`): code and unit tests
      done (69 tests pass); only the manual attach-modal check above remains.


## Done

- [x] Windows TUI input via console records: Alt+Shift / mouse / focus no longer
      close modals; `Esc` in attach clears typed text first, then closes.
      Verified by `tests/test_tui_windows.py`.

- [x] Interactive console rework: stdlib TUI (`ppp/tui/`) with hotkeys, modals
      (paste proxies / list URL, attach, launch, rotation, confirm, help),
      scrollable viewport and filters. Old console kept as `rot.py console`.
      Verified by `tests/test_tui_keys.py` + `tests/test_tui_view.py`.
- [x] Binaries moved to `bin/`; DLL/shim resolve the data folder via
      `PROXYINJECT_DIR` -> own folder -> parent. Root cleaned, `.gitignore` added.
- [x] POSIX shim `src/proxyinject_posix.c` (connect + getaddrinfo via `dlsym`),
      `ppp/platform.py` launcher for `LD_PRELOAD` / `DYLD_INSERT_LIBRARIES`,
      zig targets `x86_64-linux-gnu`, `aarch64-linux-gnu`, `aarch64-macos`.
      Verified in WSL: fake SOCKS5 exit and a real HTTP proxy with auth.
- [x] `.agents/` scaffolding.
- [x] `host:port:user:pass` parsing + unit test.
- [x] `proxy/*.txt` auto-import into `proxies.txt`.
- [x] `state/<pid>.txt` written by the DLL, liveness shown in the console.
- [x] Interactive console (`python rot.py`, `run.cmd`, `run.sh`).
- [x] `run.cmd` and `run.sh` launchers.
- [x] Windows launch/attach/kill verified with `notepad.exe` and `inject.exe --selftest`.
