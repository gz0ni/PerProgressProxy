from __future__ import annotations

import os
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent

CONFIG = ROOT / "config.ini"
PROXIES = ROOT / "proxies.txt"
PROXY_DIR = ROOT / "proxy"
STATE_DIR = ROOT / "state"
LOGS_DIR = ROOT / "logs"
BIN_DIR = ROOT / "bin"
TESTS_DIR = ROOT / "tests"


def binary(name: str):
    """Prefer bin/, fall back to the repository root for old layouts."""
    candidate = BIN_DIR / name
    if candidate.exists():
        return candidate
    return ROOT / name


DLL = binary("proxyinject.dll")
INJECT = binary("inject.exe")

IS_WINDOWS = os.name == "nt"
