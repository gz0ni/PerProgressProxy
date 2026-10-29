"""Interactive console (stdlib only): hotkeys, modals, scrollable lists."""

from __future__ import annotations


def run() -> int:
    from .app import run as _run

    return _run()


__all__ = ["run"]
