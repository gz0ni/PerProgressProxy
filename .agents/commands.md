# Commands

All commands run from the repository root.

## Run

```powershell
.\run.cmd                 # Windows: interactive console (hotkeys)
.\run.cmd status          # one-shot command, same as python rot.py status
python rot.py             # interactive console on any platform
python rot.py console     # old command-driven console
python rot.py list        # numbered proxy list
python rot.py import proxy\webshare.txt
```

`run.cmd` validates the interpreter (skips `WindowsApps` stubs) and logs every
launch to `logs\launcher.log`; uncaught exceptions land in `logs\ui-crash.log`.
Pin an interpreter with `set PPP_PY_CMD=C:\path\to\python.exe`.

```sh
./run.sh                  # Linux/macOS launcher (POSIX backend not ready yet)
```

## Verification

Run before claiming work is done:

```powershell
python -m unittest discover -s tests -p "test_*.py" -v
python -m compileall -q ppp rot.py
python rot.py list
python rot.py status
```

POSIX (inside WSL/Linux/macOS):

```sh
sh tests/posix_selftest.sh     # fake SOCKS5 exit, expects exit_ip=10.9.9.9
sh tests/posix_live.sh         # first credentialed proxy, prints only the exit IP
python3 rot.py launch "sleep 5" && python3 rot.py status
```

End-to-end without internet, using the fake SOCKS exits:

```powershell
# terminal 1
python tests\testsocks.py --port 11080 --fake-ip 10.9.9.9

# terminal 2
.\bin\inject.exe --selftest --proxy socks5://127.0.0.1:11080 --url http://checkip.amazonaws.com/
# expected: exit_ip=10.9.9.9, result=OK
```

## Build

Windows:

```powershell
.tools\ziglang\zig.exe cc -target x86_64-windows-gnu -O2 -shared `
  -o bin\proxyinject.dll src\proxyinject.c -lws2_32

.tools\ziglang\zig.exe cc -target x86_64-windows-gnu -O2 `
  -o bin\inject.exe src\inject.c -lws2_32 -lpsapi
```

POSIX shim:

```powershell
.tools\ziglang\zig.exe cc -target x86_64-linux-gnu -O2 -shared -fPIC `
  -o bin\proxyinject.so src\proxyinject_posix.c -ldl

.tools\ziglang\zig.exe cc -target aarch64-linux-gnu -O2 -shared -fPIC `
  -o bin\proxyinject-aarch64.so src\proxyinject_posix.c -ldl

.tools\ziglang\zig.exe cc -target aarch64-macos -O2 -dynamiclib `
  -o bin\proxyinject.dylib src\proxyinject_posix.c
```

Rebuild the shim after touching `src/`. The Python side needs no build step.

## Tests

- `tests/test_specs.py` - proxy spec parsing, import and dedup (unittest).
- `tests/test_tui_keys.py` - keyboard decoding (ANSI and Windows scan codes).
- `tests/test_tui_windows.py` - Windows console record mapping (Alt/Esc, AltGr,
  Ctrl, F-keys, ignored events) and attach-modal `Esc` behaviour.
- `tests/test_tui_view.py` - pure rendering: layout, viewport, filters, modals.
- `tests/testsocks.py` - fake SOCKS5 exit used by both selftests.
- `tests/poller.py` - prints the exit IP every second, used to watch rotation.
- `tests/posix_selftest.sh` - shim end-to-end against the fake exit.
- `tests/posix_live.sh` - shim against the first credentialed proxy.
- No CI configured.

## Packaging

Zip the repository root minus `proxy/` contents and `state/`; the binaries are
already committed next to the sources.
