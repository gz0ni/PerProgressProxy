# Rules

## Code Style

- Rust: stable, edition 2024, `cargo fmt`-style, `Result` over panics, no new
  dependencies without a reason (current: clap, serde, serde_json, ureq,
  windows-sys, libc).
- C: C11, two-space indent, function names `snake_case`, globals prefixed `g_`.
- File names: lowercase, no spaces. Directories: `src/`, `rust/`, `tests/`,
  `proxy/`, `state/`, `logs/`.

## Languages

- User-facing console text: Russian is allowed, English is fine; keep one
  language per screen.
- Identifiers, file names, commits, docs: English.
- Proxy lists and logs: raw data, never translated.

## Design

- Contract with the C side: `proxies.db` (tables `proxies(id, text)` and
  `settings(key, value)`, read-only for C, journal mode DELETE), `state/`, `logs/`. Do not add
  IPC, services, or a database server.
- The interactive console is a hand-rolled TUI: raw console/termios input plus
  ANSI output, no ratatui/crossterm. Rendering is a pure function over a
  `ViewState` (`rust/src/tui/view.rs`) so it can be tested without a terminal.
- File mutations go through `Store` and `processes`; the TUI reaches them via
  `rust/src/tui/model.rs`.

## State and Persistence

- Only the DLL writes `state/<pid>.txt` and `logs/<pid>.log`.
- Only `Store` (`rust/src/store.rs`) writes `proxies.db`.
- `state/` and `logs/` are disposable: deleting them must never break settings.

## Testing

- `cargo test`; unit tests sit in `#[cfg(test)]` modules next to the code and use
  temp folders, never the real `proxies.db`.
- Anything that parses user input (proxy specs, import) needs a test with a real
  sample line from `proxy/`.
- Windows-only behaviour is tested manually; document the manual step in the task.

## Generated and External Files

- `proxyinject.dll` and `inject.exe` are generated artifacts; rebuild from `src/`.
- `.tools/` is a vendored Zig toolchain; do not edit or commit changes to it.
- `proxy/*.txt` and `proxies.db` contain credentials; never paste them into
  docs, commits, or chat.

## Git

- Do not commit, push, or create PRs without an explicit user request.
- The directory is currently not a git repository; do not initialize one
  unprompted.
