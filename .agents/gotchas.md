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

### Non-ASCII text in `.cmd` breaks the whole script (2026-10-29)

- What happened: after adding a `rem` line with an em dash to `run.cmd`,
  launching it printed `'et' is not recognized` and `'not' is not recognized`,
  skipped the Python line, and the window closed instantly.
- Root cause: `chcp 65001` changes the code page in the middle of a batch file;
  cmd keeps its read position in bytes and re-reads the remaining lines with the
  new encoding, so multi-byte characters shift offsets and lines get chopped.
- Avoid: keep launcher files ASCII-only (checked with
  `python -c "print([b for b in open('run.cmd','rb').read() if b > 127])"`), and
  put any Unicode text in the Python side instead.

---

### Windows arrow keys are scan codes, not ANSI (2026-10-29)

- What happened: the first TUI draft decoded only `\x1b[A`-style sequences and
  arrows did nothing on Windows.
- Root cause: with console input mode untouched, `msvcrt.getwch()` returns
  `\x00` / `\xe0` plus a scan code character, and `\x1b` only appears for Esc
  and Alt.
- Avoid: decode both forms (`ppp/tui/term.py`, `WINDOWS_SCAN`); test the
  decoder as a pure function (`tests/test_tui_keys.py`) instead of by hand.

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
