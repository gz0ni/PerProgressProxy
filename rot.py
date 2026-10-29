#!/usr/bin/env python3
"""rot.py - control panel for proxyinject.dll.

Run without arguments for the interactive console, or use a subcommand:

    python rot.py                 # console with hotkeys
    python rot.py console         # older command-driven console
    python rot.py status
    python rot.py import          # merge proxy/*.txt
    python rot.py test --prune
    python rot.py launch "C:\\path\\app.exe"
    python rot.py next
    python rot.py on | off

Only the standard library is used, so it runs on any Python 3.8+.
"""

from __future__ import annotations

import sys
import time
import traceback

from ppp.cli import main
from ppp.processes import process_alive
from ppp.specs import Proxy, SpecError, parse_spec
from ppp.store import proxies, read_config, set_config

__all__ = [
    "main",
    "process_alive",
    "Proxy",
    "SpecError",
    "parse_spec",
    "proxies",
    "read_config",
    "set_config",
]


def _log_crash() -> str | None:
    """Keep the traceback when the console window closes too fast to read it."""
    from ppp.paths import LOGS_DIR

    try:
        LOGS_DIR.mkdir(parents=True, exist_ok=True)
        path = LOGS_DIR / "ui-crash.log"
        with path.open("a", encoding="utf-8") as handle:
            handle.write(f"[{time.strftime('%Y-%m-%d %H:%M:%S')}] rot.py {' '.join(sys.argv[1:])}\n")
            handle.write(traceback.format_exc())
            handle.write("\n")
        return str(path)
    except OSError:
        return None


if __name__ == "__main__":
    try:
        sys.exit(main())
    except KeyboardInterrupt:
        print("\ninterrupted")
        sys.exit(130)
    except SystemExit:
        raise
    except BaseException:
        traceback.print_exc()
        where = _log_crash()
        if where:
            print(f"\ncrash logged to {where}")
        sys.exit(1)
