"""Injection backend per platform."""

from __future__ import annotations

import os
import shlex
import subprocess

from .paths import BIN_DIR, INJECT, IS_WINDOWS, ROOT


class UnsupportedPlatform(RuntimeError):
    pass


def shim_path():
    for folder in (BIN_DIR, ROOT):
        for name in ("proxyinject.so", "proxyinject.dylib"):
            candidate = folder / name
            if candidate.exists():
                return candidate
    return None


def available() -> bool:
    if IS_WINDOWS:
        return INJECT.exists()
    return shim_path() is not None


def _run_windows(args: list[str]) -> tuple[int, list[str]]:
    if not INJECT.exists():
        raise FileNotFoundError(INJECT.name)
    proc = subprocess.run([str(INJECT), *args], capture_output=True, text=True, timeout=300)
    lines = [line for line in (proc.stdout or "").splitlines() if line.strip()]
    lines += [line for line in (proc.stderr or "").splitlines() if line.strip()]
    return proc.returncode, lines


def list_processes() -> tuple[int, list[str]]:
    if not IS_WINDOWS:
        raise UnsupportedPlatform("process listing uses the Windows injector; use ps for now")
    return _run_windows(["--list"])


def inject_pid(pid: int) -> tuple[int, list[str]]:
    if not IS_WINDOWS:
        raise UnsupportedPlatform(
            "attaching to an already running process needs ptrace and is not implemented; "
            f"start the app through the shim instead: launch \"...\""
        )
    return _run_windows(["--pid", str(pid)])


def inject_name(name: str) -> tuple[int, list[str]]:
    if not IS_WINDOWS:
        raise UnsupportedPlatform(
            "attaching by name is Windows-only; start the app through the shim instead"
        )
    return _run_windows(["--name", name])


def shim_env() -> dict[str, str]:
    shim = shim_path()
    if shim is None:
        raise UnsupportedPlatform(
            "no shim built; run: zig cc -target <triple> -O2 -shared -fPIC "
            "-o proxyinject.so src/proxyinject_posix.c -ldl"
        )
    env = dict(os.environ)
    var = "DYLD_INSERT_LIBRARIES" if shim.suffix == ".dylib" else "LD_PRELOAD"
    existing = env.get(var, "")
    env[var] = f"{shim}{':' + existing if existing else ''}"
    env["PROXYINJECT_DIR"] = str(ROOT)
    return env


def launch(command: str) -> tuple[int, list[str]]:
    if IS_WINDOWS:
        return _run_windows(["--launch", command])
    argv = shlex.split(command)
    if not argv:
        raise UnsupportedPlatform("empty command")
    env = shim_env()
    try:
        proc = subprocess.Popen(argv, env=env, cwd=str(ROOT))
    except FileNotFoundError as exc:
        raise UnsupportedPlatform(f"cannot start {argv[0]}: {exc}") from exc
    shim = shim_path()
    return 0, [f"started pid {proc.pid} under {shim.name if shim else '?'}",
               f"target keeps running; stop it with: kill {proc.pid}"]
