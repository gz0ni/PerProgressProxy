"""Plain-text console for the proxy rotator.

Redraws after every command; no third-party dependencies, no raw keyboard mode.
"""

from __future__ import annotations

import os
import sys
import time

from . import netcheck, platform as backend, processes, store
from .paths import PROXY_DIR
from .specs import SpecError

RULES = "-" * 78


def _enable_ansi() -> bool:
    if os.environ.get("NO_COLOR"):
        return False
    if not sys.stdout.isatty():
        return False
    if os.name == "nt":
        try:
            import ctypes

            k32 = ctypes.windll.kernel32
            handle = k32.GetStdHandle(-11)
            mode = ctypes.c_uint32()
            if not k32.GetConsoleMode(handle, ctypes.byref(mode)):
                return False
            return bool(k32.SetConsoleMode(handle, mode.value | 0x0004))
        except Exception:
            return False
    return True


USE_COLOR = _enable_ansi()


def c(text: str, code: str) -> str:
    return f"\x1b[{code}m{text}\x1b[0m" if USE_COLOR else text


def green(text: str) -> str:
    return c(text, "32")


def red(text: str) -> str:
    return c(text, "31")


def yellow(text: str) -> str:
    return c(text, "33")


def dim(text: str) -> str:
    return c(text, "2")


def cyan(text: str) -> str:
    return c(text, "36")


def fmt_age(seconds: float) -> str:
    if seconds < 90:
        return f"{int(seconds)}s"
    if seconds < 5400:
        return f"{int(seconds // 60)}m"
    return f"{int(seconds // 3600)}h"


def health_cell(proxy_label: str) -> str:
    entry = netcheck.load_health().get(proxy_label)
    if not entry:
        return dim("untested")
    if not entry.get("ok"):
        note = (entry.get("note") or "failed")[:22]
        return red(f"FAIL {note}")
    ms = entry.get("ms", 0)
    ip = entry.get("ip") or ""
    tag = green(f"ok {ms}ms")
    return f"{tag} {dim(ip)}"


def render() -> None:
    cfg = store.read_config()
    rows = store.proxies()
    cur = store.current_index()
    states = processes.read_states()

    enabled = cfg.get("enabled", "1") == "1"
    power = green("PROXY: ON ") if enabled else yellow("PROXY: OFF")
    if rows:
        idx, _text, active = rows[cur] if 0 <= cur < len(rows) else rows[0]
        current = f"current #{idx} {active.display}"
    else:
        current = "no proxies"
    rotate = cfg.get("rotate_sec", "0")
    rotate_txt = f"rotate {rotate}s" if rotate not in ("0", "") else "rotate manual"

    print()
    print(f"  {cyan('PerProcessProxy')}   {power}   {current}   {dim(rotate_txt)}")
    print(f"  {dim(RULES)}")

    print(f"  {dim('PROXIES')}  {dim(str(len(rows)) + ' entries, newest import from ' + PROXY_DIR.name + '/')}")
    if rows:
        for idx, _text, proxy in rows:
            mark = cyan("*") if idx == cur else " "
            print(f"  {mark} {idx:>3}  {proxy.display:<46} {health_cell(proxy.label)}")
    else:
        print(f"      {dim('none - drop a list into ' + PROXY_DIR.name + '/ and run: import')}")

    alive = sum(1 for st in states if st.alive)
    print(f"  {dim(RULES)}")
    print(f"  {dim('PROCESSES')}  {dim(f'{alive} alive / {len(states)} recorded')}")
    if states:
        for st in sorted(states, key=lambda s: (not s.alive, s.pid)):
            tag = green("alive") if st.alive else red("gone ")
            proxy_txt = f"#{st.current}" if st.alive else "-"
            print(f"      {st.pid:>7}  {st.exe[:24]:<24} {tag}  {proxy_txt:<4} seen {fmt_age(st.age)} ago")
    else:
        print(f"      {dim('none - use: launch \"C:\\path\\app.exe\" or attach <pid|name>')}")

    print(f"  {dim(RULES)}")
    print(f"  {dim('3 | next | on | off | test [--prune] | import | attach <pid|name>')}")
    print(f"  {dim('launch \"app.exe\" | stop [pid] | kill <pid> | clean | rm <n...> | add <spec>')}")
    print(f"  {dim('status | list | doctor | help | quit')}")


def _read(prompt: str) -> str:
    try:
        return input(prompt).strip()
    except EOFError:
        return "quit"


def _print_lines(lines: list[str]) -> None:
    for line in lines:
        print(f"  {line}")


def _cmd_test(rest: list[str]) -> None:
    prune = "--prune" in rest
    rows = store.proxies()
    if not rows:
        print("  no proxies to test")
        return
    print(f"  testing {len(rows)} proxies in parallel, this may take ~{int(12)}s...")
    results, direct = _run_silent_tests(rows)
    good = 0
    for r in results:
        if r.ok and not (direct and r.exit_ip == direct):
            good += 1
            mark = green("ok  ")
        elif r.ok:
            mark = yellow("same")
        else:
            mark = red("FAIL")
        print(f"  {mark} #{r.idx:<3} {r.proxy.display:<46} {r.ms:>5}ms  "
              f"{r.exit_ip or '-':<16} {r.note}")
    print(f"  {good} usable, {len(results) - good} failed")
    if prune:
        keep_labels = {r.proxy.label for r in results if r.ok and not (direct and r.exit_ip == direct)}
        keep = [text for _idx, text, p in rows if p.label in keep_labels]
        store.write_proxy_lines(keep)
        print(f"  pruned proxies.txt to {len(keep)} working entries")


def _run_silent_tests(rows):
    direct = netcheck.direct_ip()
    results = netcheck.test_all(rows)
    netcheck.save_health(results)
    return results, direct


def _cmd_stop(rest: list[str]) -> None:
    states = processes.read_states()
    targets = [st for st in states if st.alive]
    if rest:
        try:
            pid = int(rest[0])
        except ValueError:
            print("  usage: stop [pid]")
            return
        targets = [st for st in targets if st.pid == pid]
    if not targets:
        print("  nothing to stop")
        return
    if not rest and len(targets) > 1:
        answer = _read(f"  terminate {len(targets)} process(es)? [y/N] ")
        if answer.lower() not in ("y", "yes", "д", "да"):
            print("  cancelled")
            return
    for st in targets:
        ok = processes.kill_process(st.pid)
        print(f"  pid {st.pid} {st.exe}: {'terminated' if ok else 'failed'}")
    processes.clean_dead()


def _cmd_attach(rest: list[str]) -> None:
    if not rest:
        print("  usage: attach <pid|exe name>")
        return
    target = rest[0]
    try:
        if target.isdigit():
            _rc, lines = backend.inject_pid(int(target))
        else:
            _rc, lines = backend.inject_name(target)
    except backend.UnsupportedPlatform as exc:
        print(f"  {exc}")
        return
    _print_lines(lines)
    time.sleep(0.7)


def _cmd_launch(rest: list[str]) -> None:
    command = " ".join(rest).strip()
    if len(command) >= 2 and command[0] == command[-1] and command[0] in "\"'":
        command = command[1:-1]
    if not command:
        print("  usage: launch \"C:\\path\\app.exe\"")
        return
    try:
        _rc, lines = backend.launch(command)
    except backend.UnsupportedPlatform as exc:
        print(f"  {exc}")
        return
    _print_lines(lines)
    time.sleep(0.7)


def _cmd_rm(rest: list[str]) -> None:
    try:
        indices = [int(item) for item in rest]
    except ValueError:
        print("  usage: rm <n> [n...]")
        return
    removed = store.remove_indices(indices)
    print(f"  removed {removed} entries")


def dispatch(line: str) -> bool:
    """Returns False when the shell should exit."""
    if not line:
        return True
    parts = line.split()
    cmd, rest = parts[0].lower(), parts[1:]
    raw_rest = line[len(parts[0]):].strip()

    if cmd in ("quit", "exit", "q"):
        return False
    if cmd in ("help", "?"):
        _print_lines([
            "<number>        use that proxy",
            "next            next proxy in the list",
            "on / off        proxy / pass everything through",
            "test [--prune]  check every proxy, optionally delete the failures",
            "import          merge proxy/*.txt into proxies.txt",
            "attach <pid|exe> inject into a running process",
            "launch \"app\"    start a program already injected",
            "stop [pid]      terminate injected process(es)",
            "kill <pid>      same as stop <pid>",
            "clean           drop state files of dead processes",
            "rm <n...>       remove proxies by index",
            "add <spec...>   append proxies",
            "status/list/doctor  detailed views",
        ])
        return True
    if cmd.isdigit():
        try:
            index = int(cmd) % max(1, len(store.proxies()))
        except SpecError:
            return True
        rows = store.proxies()
        if not rows:
            print("  no proxies")
            return True
        store.set_config(current=index)
        print(f"  current #{index}  {rows[index][2].display}")
        return True
    if cmd in ("next", "n"):
        rows = store.proxies()
        if not rows:
            print("  no proxies")
            return True
        index = (store.current_index() + 1) % len(rows)
        store.set_config(current=index)
        print(f"  current #{index}  {rows[index][2].display}")
        return True
    if cmd in ("on", "off"):
        store.set_config(enabled=1 if cmd == "on" else 0)
        print(f"  proxying {'ON' if cmd == 'on' else 'OFF - traffic passes straight through'}")
        return True
    if cmd == "test":
        _cmd_test(rest)
        return True
    if cmd in ("import", "sync"):
        result = store.sync_proxy_dir()
        if not result["files"]:
            print(f"  {PROXY_DIR.name}/ is empty")
        else:
            print(f"  scanned {result['files']} file(s): added {result['added']}, "
                  f"unusable {result['skipped']}, total {result['total']}")
        return True
    if cmd in ("attach", "inject"):
        _cmd_attach(rest)
        return True
    if cmd == "launch":
        _cmd_launch([raw_rest] if raw_rest else [])
        return True
    if cmd in ("stop", "kill"):
        _cmd_stop(rest)
        return True
    if cmd == "clean":
        print(f"  removed {processes.clean_dead()} stale state file(s)")
        return True
    if cmd == "rm":
        _cmd_rm(rest)
        return True
    if cmd == "add":
        if not rest:
            print("  usage: add host:port or host:port:user:pass")
            return True
        added, notes = store.add_specs(rest)
        _print_lines(notes)
        print(f"  {added} added")
        return True
    if cmd == "status":
        from .cli import cmd_status

        cmd_status(None)
        return True
    if cmd == "list":
        from .cli import cmd_list

        cmd_list(None)
        return True
    if cmd == "doctor":
        from .cli import cmd_doctor

        cmd_doctor(None)
        return True

    print(f"  unknown command: {parts[0]} (try help)")
    return True


def run() -> int:
    store.sync_proxy_dir()
    processes.migrate_legacy()
    print(f"  {dim('PerProcessProxy console. Type help for commands, quit to exit.')}")
    while True:
        render()
        try:
            line = _read("\n  > ")
        except KeyboardInterrupt:
            print("\n  bye")
            return 0
        try:
            if not dispatch(line):
                print("  bye")
                return 0
        except KeyboardInterrupt:
            print("\n  ^C")
            return 0
        except (SpecError, ValueError) as exc:
            print(f"  error: {exc}")
