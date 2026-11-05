# Commands

All commands run from the repository root.

## Build the panel

Prebuilt binaries are committed in `bin/`: `rot.exe` (Windows), `rot` (Linux
x86_64), `rot-aarch64` (Linux arm64). Rust is needed only to rebuild them or on
macOS (no prebuilt: the Apple SDK is required to link).

```powershell
cd rust; cargo build --release     # rust\target\release\rot.exe
Copy-Item rust\target\release\rot.exe bin\rot.exe -Force
```

Linux binaries are cross-built from Windows with the vendored zig as C compiler
and linker, e.g. target `x86_64-unknown-linux-gnu` with zig target
`x86_64-linux-gnu.2.17` (a small wrapper must drop cargo's `--target=` argument).
Copy results with PowerShell `Copy-Item`: Git Bash `cp` appends `.exe` and
overwrote `bin/rot.exe` once.

## Run

```powershell
.\run.cmd                 # Windows: interactive console (hotkeys)
.\run.cmd status          # one-shot command, same as rot status
.\run.cmd console         # old command-driven console
.\run.cmd list            # numbered proxy list
.\run.cmd import proxy\webshare.txt
```

`run.sh` picks `bin/rot` or `bin/rot-aarch64` by `uname`, else `rust/target/release/rot`.
`run.cmd` starts `bin\rot.exe`, else `rust\target\release\rot.exe`, and logs every
launch to `logs\launcher.log`; panics land in `logs\ui-crash.log`.
`rot --help` lists every subcommand. `PPP_ROOT` overrides the data folder
(default: cwd, else the first parent of the exe holding `config.ini`).

```sh
./run.sh                  # Linux/macOS launcher (POSIX backend not ready yet)
```

## Verification

Run before claiming work is done:

```powershell
cd rust; cargo test; cargo build --release
.\rust\target\release\rot.exe list
.\rust\target\release\rot.exe status
```

POSIX (inside WSL/Linux/macOS):

```sh
sh tests/posix_selftest.sh     # fake SOCKS5 exit, expects exit_ip=10.9.9.9
sh tests/posix_live.sh         # first credentialed proxy, prints only the exit IP
rot launch "sleep 5" && rot status
```

End-to-end without internet, using the fake SOCKS exits (needs Python for the
dev helper only):

```powershell
# terminal 1
python tests\testsocks.py --port 11080 --fake-ip 10.9.9.9

# terminal 2
.\bin\inject.exe --selftest --proxy socks5://127.0.0.1:11080 --url http://checkip.amazonaws.com/
# expected: exit_ip=10.9.9.9, result=OK

# panel against the fake exit, isolated folder
$env:PPP_ROOT = 'C:\temp\ppp'; rot add socks5://127.0.0.1:11080; rot test
```

## Build the shim

SQLite is compiled into the DLL and the shim (`src/sqlite/sqlite3.c`). Common flags:

```powershell
$sq = "-DSQLITE_THREADSAFE=0 -DSQLITE_OMIT_LOAD_EXTENSION -DSQLITE_OMIT_DEPRECATED -DSQLITE_DEFAULT_MEMSTATUS=0 -DSQLITE_OMIT_SHARED_CACHE -DSQLITE_DQS=0 -DSQLITE_TEMP_STORE=3"
```

Windows:

```powershell
.tools\ziglang\zig.exe cc -target x86_64-windows-gnu -O2 -shared $sq.Split(' ') `
  -o bin\proxyinject.dll src\proxyinject.c src\sqlite\sqlite3.c -lws2_32

.tools\ziglang\zig.exe cc -target x86_64-windows-gnu -O2 `
  -o bin\inject.exe src\inject.c -lws2_32 -lpsapi
```

POSIX shim (the hidden-visibility define is mandatory: otherwise the shim would
export `sqlite3_*` and interpose the target's own SQLite):

```sh
HID='-DSQLITE_API=__attribute__((visibility("hidden")))'
zig cc -target x86_64-linux-gnu.2.17 -O2 -s -shared -fPIC "$HID" $SQ \
  -o bin/proxyinject.so src/proxyinject_posix.c src/sqlite/sqlite3.c -ldl
zig cc -target aarch64-linux-gnu.2.17 -O2 -s -shared -fPIC "$HID" $SQ \
  -o bin/proxyinject-aarch64.so src/proxyinject_posix.c src/sqlite/sqlite3.c -ldl
zig cc -target aarch64-macos -O2 -dynamiclib "$HID" $SQ \
  -o bin/proxyinject.dylib src/proxyinject_posix.c src/sqlite/sqlite3.c
```

Rebuild the shim after touching `src/`. Run bash snippets from a file: inline
shells here eat backslashes.

## Tests

Rust unit tests live next to the code (`cargo test`, 89 tests):

- `specs.rs` - proxy spec parsing and token splitting.
- `store.rs` - config edits, import, sync, dedup, removal.
- `processes.rs` - state files, liveness, legacy migration.
- `netcheck.rs` - SOCKS5 / HTTP CONNECT handshakes against local listeners.
- `tui/term.rs` - keyboard decoding and Windows console record mapping.
- `tui/view.rs` - pure rendering: layout, viewport, modals.
- `tui/model.rs`, `tui/app.rs` - filters, selection, hotkey loop with a fake terminal.

Dev helpers and shell tests:

- `tests/testsocks.py` - fake SOCKS5 exit used by both selftests.
- `tests/poller.py` - prints the exit IP every second, used to watch rotation.
- `tests/posix_selftest.sh` - shim end-to-end against the fake exit.
- `tests/wsl_db_check.sh` - shim reads `proxies.db` and follows `rot use 1` live.
- `tests/posix_live.sh` - shim against the first credentialed proxy.
- No CI configured.

## Packaging

Zip the repository root minus `proxy/` contents, `state/` and `rust/target/`;
the shim binaries are already committed next to the sources.
