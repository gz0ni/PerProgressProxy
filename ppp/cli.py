"""Command handlers and the argument parser."""

from __future__ import annotations

import argparse
import ctypes
import os
import sys
import time
from pathlib import Path

from . import netcheck, platform as backend, processes, store
from .paths import CONFIG, DLL, INJECT, IS_WINDOWS, PROXIES, PROXY_DIR, ROOT, STATE_DIR
from .specs import SpecError


def _health() -> dict[str, dict]:
    return netcheck.load_health()


def _health_text(proxy) -> str:
    entry = _health().get(proxy.label)
    if not entry:
        return "untested"
    if not entry.get("ok"):
        return f"FAIL {entry.get('note', '')[:28]}"
    return f"ok {entry.get('ms', 0)}ms"


def _fmt_age(seconds: float) -> str:
    if seconds < 90:
        return f"{int(seconds)}s"
    if seconds < 5400:
        return f"{int(seconds // 60)}m"
    return f"{int(seconds // 3600)}h"


def cmd_status(_args: argparse.Namespace) -> int:
    cfg = store.read_config()
    rows = store.proxies()
    cur = store.current_index()
    health = _health()
    print(f"folder        {ROOT}")
    if IS_WINDOWS:
        print(f"dll           {'ok' if DLL.exists() else 'MISSING'}   inject.exe "
              f"{'ok' if INJECT.exists() else 'MISSING'}")
    else:
        shim = backend.shim_path()
        print(f"shim          {shim if shim else 'MISSING (build src/proxyinject_posix.c)'}")
    print(f"proxying      {'ON' if cfg.get('enabled', '1') == '1' else 'OFF (pass-through)'}")
    print(f"fail closed   {cfg.get('fail_closed', '1')}")
    print(f"udp           {'blocked' if cfg.get('block_udp', '1') == '1' else 'allowed (leaks)'}")
    rot = cfg.get("rotate_sec", "0")
    print(f"auto rotate   {'every ' + rot + 's' if rot not in ('0', '') else 'off'}")
    print(f"proxies       {len(rows)}  (source folder: {PROXY_DIR.name}/)")
    if rows:
        idx, _text, p = rows[cur] if 0 <= cur < len(rows) else rows[0]
        print(f"current       #{cur}  {p.display}  [{_health_text(p)}]")
    else:
        print("current       -    (no proxies, traffic goes direct)")

    injected = processes.read_states()
    if injected:
        alive = sum(1 for st in injected if st.alive)
        print(f"injected      {alive} alive / {len(injected)} recorded")
        for st in injected:
            state = "alive" if st.alive else "gone "
            print(f"  pid {st.pid:>6}  {state}  {st.exe:<24} proxy #{st.current}  "
                  f"seen {_fmt_age(st.age)} ago")
    else:
        print("injected      none")
    stale = [st for st in injected if not st.alive]
    if stale:
        print(f"stale state   {len(stale)} (run: python rot.py clean)")
    return 0


def cmd_list(_args: argparse.Namespace) -> int:
    rows = store.read_proxy_lines()
    if not rows:
        print(f"proxies.txt is empty - drop a list into {PROXY_DIR.name}/ or use: python rot.py add host:port")
        return 0
    cur = store.current_index()
    n = 0
    for _line, text, p, err in rows:
        if p is None:
            print(f"    !  {text[:50]}   [invalid: {err}]")
            continue
        mark = "*" if n == cur else " "
        print(f"{mark} {n:>3}  {p.display:<46} {_health_text(p)}")
        n += 1
    return 0


def cmd_add(args: argparse.Namespace) -> int:
    added, notes = store.add_specs(args.specs)
    for note in notes:
        print(note)
    total = len(store.read_proxy_lines())
    print(f"{added} added, {total} total")
    return 0 if added or total else 1


def cmd_import(args: argparse.Namespace) -> int:
    if args.file:
        src = Path(args.file)
        try:
            added, skipped, total = store.import_file(src)
        except FileNotFoundError:
            print(f"no such file: {src}")
            return 1
        print(f"imported {added}, skipped {skipped}, total {total}")
        return 0
    return cmd_sync(args)


def cmd_sync(_args: argparse.Namespace) -> int:
    result = store.sync_proxy_dir()
    if not result["files"]:
        print(f"{PROXY_DIR.name}/ is empty - drop proxy lists there and rerun")
        return 0
    print(f"scanned {result['files']} file(s) in {PROXY_DIR.name}/: "
          f"added {result['added']}, unusable {result['skipped']}, total {result['total']}")
    return 0


def _set_current(index: int) -> None:
    rows = store.proxies()
    if not rows:
        raise SystemExit("proxies.txt is empty")
    index %= len(rows)
    store.set_config(current=index)
    print(f"current = #{index}  {rows[index][2].display}")


def cmd_use(args: argparse.Namespace) -> int:
    _set_current(args.index)
    return 0


def cmd_next(_args: argparse.Namespace) -> int:
    _set_current(store.current_index() + 1)
    return 0


def cmd_auto(args: argparse.Namespace) -> int:
    store.set_config(rotate_sec=args.seconds)
    if args.seconds:
        print(f"rotating every {args.seconds}s (live sockets are dropped on each rotation)")
    else:
        print("automatic rotation off")
    return 0


def cmd_power(args: argparse.Namespace) -> int:
    store.set_config(enabled=1 if args.on else 0)
    print("proxying ON" if args.on else "proxying OFF - traffic passes straight through")
    return 0


def cmd_rm(args: argparse.Namespace) -> int:
    removed = store.remove_indices(args.indices)
    print(f"removed {removed}, {len(store.read_proxy_lines())} left")
    return 0


def run_tests(concurrency: int = 24, timeout: float = 12.0, verbose: bool = True) -> tuple[list[netcheck.TestResult], str | None]:
    rows = store.proxies()
    if not rows:
        print("no proxies to test")
        return [], None
    if verbose:
        print("finding this machine's direct IP...")
    direct = netcheck.direct_ip()
    if verbose:
        print(f"direct IP: {direct or 'unknown (offline?)'}\n")
    results = netcheck.test_all(rows, concurrency=concurrency, timeout=timeout)
    netcheck.save_health(results)
    if verbose:
        good = 0
        for r in results:
            mark = "ok  "
            note = r.note
            if r.ok and direct and r.exit_ip == direct:
                mark = "same"
                note = "exit IP equals your direct IP - useless for hiding"
            elif not r.ok:
                mark = "FAIL"
            else:
                good += 1
            print(f"{mark} #{r.idx:<3} {r.proxy.display:<46} {r.ms:>5}ms  "
                  f"{r.exit_ip or '-':<16} {note}")
        print(f"\n{good} usable, {len(results) - good} failed (out of {len(results)})")
    return results, direct


def cmd_test(args: argparse.Namespace) -> int:
    results, direct = run_tests(args.concurrency, args.timeout)
    if not results:
        return 1
    good = [r for r in results if r.ok and not (direct and r.exit_ip == direct)]
    if args.prune:
        keep_labels = {r.proxy.label for r in good}
        rows = store.proxies()
        keep = [text for _idx, text, p in rows if p.label in keep_labels]
        store.write_proxy_lines(keep)
        print(f"pruned proxies.txt to {len(keep)} working entries")
    return 0 if good else 1


def cmd_inject(args: argparse.Namespace) -> int:
    try:
        if args.launch:
            rc, lines = backend.launch(args.launch)
        elif args.name:
            rc, lines = backend.inject_name(args.name)
        elif args.pid:
            rc, lines = 0, []
            for pid in args.pid:
                pid_rc, pid_lines = backend.inject_pid(pid)
                rc = rc or pid_rc
                lines += pid_lines
        elif args.list:
            rc, lines = backend.list_processes()
        else:
            print("choose --launch, --name, --pid or --list")
            return 1
    except (FileNotFoundError, backend.UnsupportedPlatform) as exc:
        print(f"inject: {exc}")
        return 1
    for line in lines:
        print(line)
    return rc


def cmd_launch(args: argparse.Namespace) -> int:
    args.launch = args.command
    args.name = None
    args.pid = None
    args.list = False
    return cmd_inject(args)


def cmd_attach(args: argparse.Namespace) -> int:
    try:
        if args.target.isdigit():
            rc, lines = backend.inject_pid(int(args.target))
        else:
            rc, lines = backend.inject_name(args.target)
    except (FileNotFoundError, backend.UnsupportedPlatform) as exc:
        print(f"attach: {exc}")
        return 1
    for line in lines:
        print(line)
    return rc


def cmd_kill(args: argparse.Namespace) -> int:
    ok = processes.kill_process(args.pid)
    processes.clean_dead()
    print(f"pid {args.pid}: {'terminated' if ok else 'not running or access denied'}")
    return 0 if ok else 1


def cmd_clean(_args: argparse.Namespace) -> int:
    removed = processes.clean_dead()
    print(f"removed {removed} stale state file(s)")
    return 0


def cmd_watch(args: argparse.Namespace) -> int:
    print(f"watching for {args.name} every {args.interval}s - Ctrl+C to stop")
    try:
        while True:
            try:
                _rc, lines = backend.inject_name(args.name)
            except backend.UnsupportedPlatform as exc:
                print(f"watch: {exc}")
                return 1
            for line in lines:
                if "injected," in line:
                    print(time.strftime("%H:%M:%S"), line.strip(), flush=True)
            time.sleep(max(0.5, args.interval))
    except KeyboardInterrupt:
        print("\nstopped")
    return 0


def cmd_doctor(_args: argparse.Namespace) -> int:
    print(f"python        {sys.version.split()[0]}  ({sys.executable})")
    admin = bool(ctypes.windll.shell32.IsUserAnAdmin()) if IS_WINDOWS else os.geteuid() == 0
    print(f"admin         {'yes' if admin else 'no (needed only for protected/elevated targets)'}")
    print(f"dll           {DLL if DLL.exists() else 'MISSING ' + str(DLL)}")
    print(f"injector      {INJECT if INJECT.exists() else 'MISSING ' + str(INJECT)}")
    print(f"config        {CONFIG if CONFIG.exists() else 'missing'}")
    print(f"proxies       {PROXIES if PROXIES.exists() else 'missing'}")
    print(f"proxy folder  {PROXY_DIR if PROXY_DIR.exists() else 'missing'}")
    print(f"state folder  {STATE_DIR if STATE_DIR.exists() else 'missing'}")
    if backend.available():
        print(f"backend       {backend.shim_path().name if not IS_WINDOWS else 'windows (inject.exe)'}")
    else:
        print("backend       unavailable - see README")

    ip = netcheck.direct_ip()
    print(f"direct IP     {ip or 'unreachable'}")

    for port in (7897, 7890, 7891):
        try:
            import socket as _socket

            with _socket.create_connection(("127.0.0.1", port), 0.4):
                print(f"clash         listening on 127.0.0.1:{port}")
                break
        except OSError:
            continue
    else:
        print("clash         not detected on 7890/7891/7897")

    import socket as _socket

    try:
        addrs = sorted({a[4][0] for a in _socket.getaddrinfo("example.com", 80, _socket.AF_INET)})
        fake = [a for a in addrs if a.startswith(("198.18.", "198.19."))]
        if fake:
            print(f"dns           fake-ip detected ({', '.join(fake)}) - handled: "
                  f"hostnames are sent to the proxy instead of these addresses")
        else:
            print(f"dns           example.com -> {', '.join(addrs)}")
    except OSError as exc:
        print(f"dns           lookup failed: {exc}")

    states = processes.read_states()
    if states:
        alive = sum(1 for st in states if st.alive)
        print(f"injected      {alive} alive / {len(states)} recorded")
        for st in states:
            if not st.alive:
                print(f"  stale state {st.path.name} (process gone)")
    else:
        print("injected      none")
    print("\nReminder: public proxies are often dead or blocked; `python rot.py test` "
          "tells you which ones really work from this machine.")
    return 0


def cmd_ui(_args: argparse.Namespace) -> int:
    from .tui import run

    return run()


def cmd_console(_args: argparse.Namespace) -> int:
    from .ui import run

    return run()


def build_parser() -> argparse.ArgumentParser:
    ap = argparse.ArgumentParser(
        prog="rot.py",
        description="Control panel for proxyinject.dll. No arguments opens the interactive console.",
    )
    sub = ap.add_subparsers(dest="cmd", required=True)

    sub.add_parser("ui", help="interactive console with hotkeys (default)")
    sub.add_parser("console", help="plain-text console driven by typed commands")
    sub.add_parser("status", help="settings, current proxy, injected processes")
    sub.add_parser("list", help="list proxies in proxies.txt")
    sub.add_parser("doctor", help="check the environment")
    sub.add_parser("sync", help="merge proxy/*.txt into proxies.txt")
    sub.add_parser("clean", help="drop state files of dead processes")

    p = sub.add_parser("add", help="append proxies")
    p.add_argument("specs", nargs="+")

    p = sub.add_parser("import", help="import a file, or sync the proxy/ folder")
    p.add_argument("file", nargs="?")

    p = sub.add_parser("rm", help="delete proxies by index")
    p.add_argument("indices", nargs="+", type=int)

    p = sub.add_parser("use", help="switch to proxy #N now")
    p.add_argument("index", type=int)

    sub.add_parser("next", help="switch to the next proxy")

    p = sub.add_parser("auto", help="rotate every N seconds (0 = off)")
    p.add_argument("seconds", type=int)

    sub.add_parser("on", help="enable proxying")
    sub.add_parser("off", help="disable proxying (pass through)")

    p = sub.add_parser("test", help="check every proxy and print its exit IP")
    p.add_argument("--concurrency", type=int, default=24)
    p.add_argument("--timeout", type=float, default=12.0)
    p.add_argument("--prune", action="store_true", help="delete the entries that failed")

    p = sub.add_parser("watch", help="inject every new instance of an exe")
    p.add_argument("name", metavar="EXE")
    p.add_argument("--interval", type=float, default=2.0)

    p = sub.add_parser("inject", help="inject the DLL into a process")
    p.add_argument("--launch", metavar="CMD")
    p.add_argument("--name", metavar="EXE")
    p.add_argument("--pid", type=int, action="append")
    p.add_argument("--list", action="store_true")

    p = sub.add_parser("launch", help="start a program already injected")
    p.add_argument("command", metavar="CMD")

    p = sub.add_parser("attach", help="inject into a running process (pid or exe name)")
    p.add_argument("target", metavar="PID|EXE")

    p = sub.add_parser("kill", help="terminate an injected process")
    p.add_argument("pid", type=int)

    return ap


HANDLERS = {
    "ui": cmd_ui,
    "console": cmd_console,
    "status": cmd_status,
    "list": cmd_list,
    "add": cmd_add,
    "import": cmd_import,
    "sync": cmd_sync,
    "rm": cmd_rm,
    "use": cmd_use,
    "next": cmd_next,
    "auto": cmd_auto,
    "test": cmd_test,
    "watch": cmd_watch,
    "inject": cmd_inject,
    "launch": cmd_launch,
    "attach": cmd_attach,
    "kill": cmd_kill,
    "clean": cmd_clean,
    "doctor": cmd_doctor,
}


def main(argv: list[str] | None = None) -> int:
    processes.migrate_legacy()
    argv = sys.argv[1:] if argv is None else argv
    if not argv:
        if not sys.stdout.isatty():
            print("no terminal detected: use a one-shot command instead, e.g. `python rot.py status`")
            return 2
        argv = ["ui"]
    elif argv[0] in ("on", "off"):
        store.set_config(enabled=1 if argv[0] == "on" else 0)
        print("proxying ON" if argv[0] == "on" else "proxying OFF - traffic passes straight through")
        return 0
    ap = build_parser()
    args = ap.parse_args(argv)
    handler = HANDLERS.get(args.cmd)
    if handler is None:
        ap.print_help()
        return 1
    return handler(args)
