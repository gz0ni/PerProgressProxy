"""Injected process registry: state/<pid>.txt plus liveness helpers."""

from __future__ import annotations

import ctypes
import os
import signal
import subprocess
import time
from dataclasses import dataclass
from pathlib import Path

from .paths import IS_WINDOWS, ROOT, STATE_DIR

STILL_ACTIVE = 259
PROCESS_QUERY_LIMITED_INFORMATION = 0x1000
PROCESS_TERMINATE = 0x0001


@dataclass
class ProcessState:
    pid: int
    current: int
    proxies: int
    enabled: bool
    rotated_ms: int
    exe: str
    age: float
    path: Path

    @property
    def alive(self) -> bool:
        return process_alive(self.pid)


def ensure_state_dir() -> Path:
    STATE_DIR.mkdir(parents=True, exist_ok=True)
    return STATE_DIR


def migrate_legacy() -> int:
    """Move state_<pid>.txt from the repository root into state/."""
    moved = 0
    if not ROOT.exists():
        return moved
    ensure_state_dir()
    for old in sorted(ROOT.glob("state_*.txt")):
        dest = STATE_DIR / old.name.replace("state_", "", 1)
        try:
            if not dest.exists():
                old.replace(dest)
                moved += 1
            else:
                old.unlink()
        except OSError:
            continue
    return moved


def _win_handle(pid: int, access: int):
    k32 = ctypes.WinDLL("kernel32", use_last_error=True)
    return k32, k32.OpenProcess(access, False, pid)


def process_alive(pid: int) -> bool:
    if pid <= 0:
        return False
    if not IS_WINDOWS:
        try:
            os.kill(pid, 0)
            return True
        except OSError:
            return False
    k32, handle = _win_handle(pid, PROCESS_QUERY_LIMITED_INFORMATION)
    if not handle:
        return False
    try:
        code = ctypes.c_ulong()
        if not k32.GetExitCodeProcess(handle, ctypes.byref(code)):
            return False
        return code.value == STILL_ACTIVE
    finally:
        k32.CloseHandle(handle)


def process_name(pid: int) -> str:
    if pid <= 0:
        return ""
    if not IS_WINDOWS:
        try:
            return (Path(f"/proc/{pid}/comm").read_text().strip()) or f"pid {pid}"
        except OSError:
            pass
        try:
            out = subprocess.run(
                ["ps", "-o", "comm=", "-p", str(pid)], capture_output=True, text=True, timeout=5
            ).stdout.strip()
            return Path(out).name if out else f"pid {pid}"
        except (OSError, subprocess.SubprocessError):
            return f"pid {pid}"
    k32, handle = _win_handle(pid, PROCESS_QUERY_LIMITED_INFORMATION)
    if not handle:
        return ""
    try:
        buf = ctypes.create_unicode_buffer(1024)
        size = ctypes.c_ulong(len(buf))
        if not k32.QueryFullProcessImageNameW(handle, 0, buf, ctypes.byref(size)):
            return ""
        return Path(buf.value).name
    finally:
        k32.CloseHandle(handle)


def kill_process(pid: int) -> bool:
    if pid <= 0:
        return False
    if not IS_WINDOWS:
        try:
            os.kill(pid, signal.SIGTERM)
            return True
        except OSError:
            return False
    k32, handle = _win_handle(pid, PROCESS_TERMINATE)
    if not handle:
        return False
    try:
        return bool(k32.TerminateProcess(handle, 1))
    finally:
        k32.CloseHandle(handle)


def state_files() -> list[Path]:
    files: list[Path] = []
    if STATE_DIR.exists():
        files.extend(sorted(STATE_DIR.glob("*.txt")))
    files.extend(sorted(ROOT.glob("state_*.txt")))
    return files


def read_state(path: Path) -> ProcessState | None:
    try:
        fields = dict(
            line.split("=", 1) for line in path.read_text(encoding="utf-8").splitlines() if "=" in line
        )
    except OSError:
        return None
    try:
        pid = int(fields.get("pid", "0"))
    except ValueError:
        return None
    if pid <= 0:
        return None
    exe = fields.get("exe", "")
    if not exe:
        exe = process_name(pid) or "-"
    try:
        age = max(0.0, time.time() - path.stat().st_mtime)
    except OSError:
        age = 0.0
    try:
        current = int(fields.get("current", "0"))
    except ValueError:
        current = 0
    try:
        proxies = int(fields.get("proxies", "0"))
    except ValueError:
        proxies = 0
    try:
        rotated = int(fields.get("rotated_ms", "0"))
    except ValueError:
        rotated = 0
    return ProcessState(
        pid=pid,
        current=current,
        proxies=proxies,
        enabled=fields.get("enabled", "0").strip() == "1",
        rotated_ms=rotated,
        exe=exe,
        age=age,
        path=path,
    )


def read_states(prune: bool = False) -> list[ProcessState]:
    out: list[ProcessState] = []
    for path in state_files():
        st = read_state(path)
        if st is None:
            if prune:
                _unlink(path)
            continue
        if prune and not st.alive:
            _unlink(path)
            continue
        out.append(st)
    return out


def clean_dead() -> int:
    removed = 0
    for path in state_files():
        st = read_state(path)
        if st is None or not st.alive:
            if _unlink(path):
                removed += 1
    return removed


def _unlink(path: Path) -> bool:
    try:
        path.unlink()
        return True
    except OSError:
        return False
