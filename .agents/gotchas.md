# Gotchas

Traps already hit in this project. Check before repeating similar work.

## Format

```
### <Short trap title> (YYYY-MM-DD)

- What happened / what was wrong
- Root cause
- How to avoid next time
```

---

### opencode under the shim still went direct (2026-11-05)

- What happened: `opencode` launched with the dylib showed no `proxy ...` lines.
- Root causes, stacked:
  1. The OpenCode desktop app runs `opencode-cli serve --service`; the CLI talks
     to it over `127.0.0.1`, which is never proxied. Fix on the user side:
     `opencode --standalone`.
  2. The shell exports `HTTP(S)_PROXY=http://127.0.0.1:7890` (Clash); Bun honours
     it and connects to loopback. `rot launch` / `rot run` now strip these.
  3. Bun resolves through `DNSServiceQueryRecordWithAttribute` (via `dlsym`),
     not `getaddrinfo`, so fake-ip `198.18.x.x` had no name and was refused
     silently. Now interposed.
- Avoid: when a target "is not proxied", trace with a scratch build that logs
  every `connect`, `dlsym` and `dlopen`; check `env | grep -i proxy` and
  `ps` for helper servers first.

### zsh does not word-split `$SQ` (2026-11-05)

- What happened: `cc ... $SQ` failed with `DSQLITE_...` undeclared.
- Root cause: zsh passes an unquoted variable as one word.
- Avoid: run build snippets with `sh script.sh`, not inline in zsh.

---

### macOS: shim loads but nothing is proxied (2026-11-05)

- What happened: with `DYLD_INSERT_LIBRARIES` the dylib logged INIT, yet every
  connection went direct.
- Root causes, three separate ones:
  1. Two-level namespaces ignore exported `connect` overrides; hooks need
     `__DATA,__interpose`.
  2. SIP strips `DYLD_*` from `/usr/bin/*` (curl, python3, and `arch` itself,
     so `arch -x86_64 env ...` loses them too).
  3. Apps that honour the system proxy (here Clash on `127.0.0.1:7890`, seen via
     `scutil --proxy`) connect to loopback, which the shim deliberately skips;
     Python `urllib` does this.
- Avoid: test with a Homebrew or self-built client, `ProxyHandler({})` in Python,
  `arch -x86_64 -e VAR=...` under Rosetta; check `scutil --proxy` first.

### macOS `/usr/bin/python3` cannot run `tests/testsocks.py` (2026-11-05)

- What happened: `SyntaxError: f-string expression part cannot include a backslash`.
- Root cause: the system python is 3.9; the helper needs 3.12+.
- Avoid: use Homebrew `python3` (the selftest picks it on Darwin).

---

### Backslashes get eaten by shell heredocs (2026-11-05)

- What happened: Windows paths written through bash/Python snippets lost
  their backslashes (`bin\rot.exe` became CR + `ot.exe`).
- Root cause: escape processing in the shell/Python layers.
- Avoid: write files with the Write/Edit tools; verify with `cat -A`.

---

### Windows arrow keys are scan codes, not ANSI (2026-10-29)

- What happened: the first TUI draft decoded only `\x1b[A`-style sequences and
  arrows did nothing on Windows.
- Root cause: with console input mode untouched, `msvcrt.getwch()` returns
  `\x00` / `\xe0` plus a scan code character, and `\x1b` only appears for Esc
  and Alt.
- Avoid: decode both forms (`ppp/tui/term.py`, `WINDOWS_SCAN`); test the
  decoder as a pure function (`tests/test_tui_keys.py`) instead of by hand.

### Alt+Shift and focus events closed modals (2026-11-05)

- What happened: switching the keyboard layout with Alt+Shift, or moving the
  mouse, closed the attach modal; holding arrows did the same.
- Root cause: `msvcrt.getwch()` reports Alt/layout switches as a bare ``,
  indistinguishable from Esc, and non-key console events leaked into the stream.
- Avoid: on Windows read `INPUT_RECORD`s via `ReadConsoleInputW`
  (`Terminal._read_console`), drop key-up, mouse, focus and Alt-only events
  (`map_windows_key`, tested in `tests/test_tui_windows.py`). Set
  `PPP_DEBUG_KEYS=1` to trace raw records into `logs/keys.log`.

### `Line + Line` does not exist in the renderer (2026-10-29)

- What happened: overlaying a modal on the base frame raised
  `TypeError: unsupported operand type(s) for +`.
- Root cause: `ppp/tui/view.py` uses a segment list, not `str`; concatenation
  needs an explicit `__add__`.
- Avoid: keep `Line` immutable-ish and compose with `+`, `slice()`, `pad()`;
  never build rows as plain strings.

---

### Moving the DLL breaks config discovery (2026-10-28)

- What happened: after moving `proxyinject.dll` into `bin/`, the injected process
  loaded 0 proxies and wrote state next to the DLL.
- Root cause: the DLL derived the data folder from its own module path only.
- Avoid: keep the `PROXYINJECT_DIR` -> own folder -> parent chain in both
  `src/proxyinject.c` and `src/proxyinject_posix.c`; when relocating binaries,
  smoke-test with `inject.exe --selftest` and check `state/<pid>.txt` appears in
  the root.

### WSL `localhost` warning floods stderr (2026-10-28)

- What happened: every `wsl.exe` invocation for the POSIX tests printed a long
  UTF-16 warning about the localhost proxy before the actual output.
- Root cause: the host has a proxied `localhost` configuration that WSL2 NAT
  cannot honour; harmless, but it garbles piped output.
- Avoid: read the tail of the output, or run the test scripts directly inside
  WSL (`bash /mnt/d/.../tests/posix_selftest.sh`) instead of parsing stdout.

### PowerShell `$PID` is read-only (2026-10-28)

- What happened: a verification snippet assigning `$pid = ...` failed with
  "cannot overwrite variable PID", and the next `kill` used the shell's own PID.
- Root cause: `$PID` is a reserved automatic variable.
- Avoid: name it `$procId`; never `kill` a value you did not parse from a state
  file.

### `proxy/webshare.txt` imported as 0 entries (2026-10-28)

- What happened: `proxy/webshare.txt` (10 lines, `host:port:user:pass`) imported
  as `imported 0, skipped 10`.
- Root cause: `parse_spec()` only handled `>= 4` colons, so the 3-colon form fell
  through to "expected host:port"; the C parser handled it correctly all along.
- Avoid: keep `ppp/specs.py` and `parse_proxy_spec()` in `src/proxyinject.c` in
  sync, and test with a real line from `proxy/`.
