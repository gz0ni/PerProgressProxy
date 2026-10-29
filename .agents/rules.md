# Rules

## Code Style

- Python: 3.8 compatible syntax at module level, `from __future__ import annotations`
  where unions are used, standard library only, no third-party packages.
- C: C11, two-space indent, function names `snake_case`, globals prefixed `g_`.
- File names: lowercase, no spaces. Directories: `src/`, `ppp/`, `tests/`,
  `proxy/`, `state/`, `logs/`.

## Languages

- User-facing console text: Russian is allowed, English is fine; keep one
  language per screen.
- Identifiers, file names, commits, docs: English.
- Proxy lists and logs: raw data, never translated.

## Design

- The file contract (`config.ini`, `proxies.txt`, `state/`, `logs/`) is the API.
  Do not add IPC, services, or a database for what a file already solves.
- The interactive console is a stdlib-only TUI: raw `msvcrt` / `termios` input
  plus ANSI output, no curses/rich/textual. Rendering is a pure function over a
  `ViewState` (`ppp/tui/view.py`) so it can be tested without a terminal.
- Command handlers in `ppp/cli.py` stay the only place that mutates files; the
  TUI calls them through `ppp/tui/model.py`.

## State and Persistence

- Only the DLL writes `state/<pid>.txt` and `logs/<pid>.log`.
- Only `ppp.store` writes `config.ini` and `proxies.txt`.
- `state/` and `logs/` are disposable: deleting them must never break settings.

## Testing

- `unittest`, no pytest dependency. Tests live in `tests/test_*.py`.
- Anything that parses user input (proxy specs, import) needs a test with a real
  sample line from `proxy/`.
- Windows-only behaviour is tested manually; document the manual step in the task.

## Generated and External Files

- `proxyinject.dll` and `inject.exe` are generated artifacts; rebuild from `src/`.
- `.tools/` is a vendored Zig toolchain; do not edit or commit changes to it.
- `proxy/*.txt` and `proxies.txt` contain credentials; never paste them into
  docs, commits, or chat.

## Git

- Do not commit, push, or create PRs without an explicit user request.
- The directory is currently not a git repository; do not initialize one
  unprompted.
