# TODO

Task checklist. Keep it honest: mark `[x]` only after verification.

## Open

- [ ] Rebuild `bin/proxyinject.so` / `-aarch64.so` from the current
      `src/proxyinject_posix.c` (needs zig) and rerun the WSL selftest.
- [ ] Windows: `cargo build` after the `rot run` change (not compiled on the Mac).

- [ ] Rust port: manual TUI pass on Windows Terminal; interactive TUI on Linux
      (only `rot status` / `--version` were run, in WSL) and on macOS.
- [ ] Manual TUI check on Windows Terminal: scroll a long proxy list, `v`
      paste (lines and a list URL), background `t` test, window resize.
- [ ] Manual attach-modal check: `a`, hold arrows, move the mouse, switch layout
      with Alt+Shift - the modal must stay open; `/` filter, `Esc` closes once.
- [ ] Optional latency history per proxy, kept in `state/`.

## In Progress

- [~] Windows console input rework (`ReadConsoleInputW`): code and unit tests
      done (69 tests pass); only the manual attach-modal check above remains.


## Done

- [x] macOS: opencode proxied. DNS-SD + `getaddrinfo_async_start` hooks, `rot run`
      (foreground exec), *_PROXY stripped. 96 tests; selftest 10.9.9.9;
      `rot run` cli -> 10.9.9.9; `rot run opencode run --standalone` with a dead
      proxy logs `failed for openrouter.ai:443` etc. Linux `.so` not rebuilt
      (no zig on the Mac; the shared C change is a pure refactor there).

- [x] Docs: `README.md` (English, main) + `README.ru.md`, step-by-step guides
      `docs/USAGE.md` / `docs/USAGE.ru.md`. Commands, options, defaults and
      hotkeys checked against `rot --help` and the source; relative links checked.

- [x] macOS port: `__interpose` hooks in the shim, universal `bin/proxyinject.dylib`
      and `bin/rot-macos`, `run.sh` Darwin case, macOS-aware selftest. 95 tests
      pass on macOS; `tests/posix_selftest.sh` -> 10.9.9.9; `rot set enabled 1` +
      `rot launch` -> 10.9.9.9; x86_64 client under Rosetta -> 10.9.9.9.

- [x] Settings in the `settings` table, `config.ini` removed; 95 tests; Windows selftest
      followed `rot set enabled 1/0` (10.9.9.9 vs direct IP); WSL check passed;
      the real `config.ini` (`current = 12`) was migrated.

- [x] Proxy list only in SQLite (`proxies.db`); DLL and shim read it directly. 93 tests;
      Windows `inject.exe --selftest` returned the fake exit IP with no txt present;
      WSL `tests/wsl_db_check.sh` followed a live `rot use 1` (10.9.9.9 -> 10.9.9.10).

- [x] Python -> Rust port of the control panel (`rust/`): 89 unit tests pass;
      `list`/`status` output identical to Python on real data; `test` verified
      against `tests/testsocks.py`; TUI render checked in a terminal tab.

- [x] Windows TUI input via console records: Alt+Shift / mouse / focus no longer
      close modals; `Esc` in attach clears typed text first, then closes.
      Verified by `tests/test_tui_windows.py`.

- [x] Interactive console rework: stdlib TUI (`ppp/tui/`) with hotkeys, modals
      (paste proxies / list URL, attach, launch, rotation, confirm, help),
      scrollable viewport and filters. Old console kept as `rot console`.
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
- [x] Interactive console (`rot`, `run.cmd`, `run.sh`).
- [x] `run.cmd` and `run.sh` launchers.
- [x] Windows launch/attach/kill verified with `notepad.exe` and `inject.exe --selftest`.
