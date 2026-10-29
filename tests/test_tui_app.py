from __future__ import annotations

import sys
import time
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent.parent))

from ppp.tui import app as app_mod
from ppp.tui.term import Key


class FakeTerminal:
    def __init__(self, stream=None) -> None:
        self.width = 100
        self.height = 30
        self.keys: list[Key] = []
        self.drawn: list[str] = []
        self.opened = False
        self.closed = False

    def open(self) -> None:
        self.opened = True

    def close(self) -> None:
        self.closed = True

    def is_tty(self) -> bool:
        return True

    def update_size(self) -> None:
        pass

    def draw(self, text: str) -> None:
        self.drawn.append(text)

    def write(self, text: str) -> None:
        self.drawn.append(text)

    def read_key(self, timeout: float = 0.3) -> Key | None:
        if self.keys:
            return self.keys.pop(0)
        return Key("ctrl_c")

    def text(self) -> str:
        return "\n".join(self.drawn)


def make_app(keys: list[Key]) -> app_mod.App:
    real = app_mod.term_mod.Terminal
    app_mod.term_mod.Terminal = FakeTerminal  # type: ignore[assignment]
    try:
        app = app_mod.App()
    finally:
        app_mod.term_mod.Terminal = real  # type: ignore[assignment]
    app.color = False
    app.term.keys = list(keys)
    return app


class AppLoopTest(unittest.TestCase):
    def test_quit_closes_the_terminal(self):
        app = make_app([Key("char", "q")])
        self.assertEqual(app.run(), 0)
        self.assertTrue(app.term.opened)
        self.assertTrue(app.term.closed)
        self.assertTrue(app.term.drawn)

    def test_ctrl_c_quits(self):
        app = make_app([Key("ctrl_c")])
        self.assertEqual(app.run(), 0)
        self.assertTrue(app.term.closed)

    def test_ctrl_c_quit_is_clean(self):
        app = make_app([Key("ctrl_c")])
        app.run()
        self.assertIn("PerProcessProxy", app.term.text())

    def test_paste_modal_renders_and_closes(self):
        app = make_app([
            Key("char", "v"),
            Key("char", "1"),
            Key("char", "2"),
            Key("esc"),
            Key("char", "q"),
        ])
        app.run()
        text = app.term.text()
        self.assertIn("paste proxies", text)
        self.assertIsNone(app.modal)

    def test_filter_key_cycles(self):
        app = make_app([Key("char", "f"), Key("char", "q")])
        app.run()
        self.assertEqual(app.screen.filter, "working")

    def test_navigation_moves_selection(self):
        app = make_app([Key("down"), Key("down"), Key("char", "q")])
        app.run()
        self.assertEqual(app.screen.selected, 2)

    def test_search_is_reachable(self):
        app = make_app([Key("char", "/"), Key("char", "8"), Key("enter"), Key("char", "q")])
        app.run()
        self.assertEqual(app.screen.query, "8")

    def test_unknown_keys_do_not_crash(self):
        app = make_app([Key("unknown"), Key("f3"), Key("char", "z"), Key("esc"), Key("char", "q")])
        self.assertEqual(app.run(), 0)

    def test_hotkeys_work_in_cyrillic_layout(self):
        app = make_app([Key("char", "а"), Key("char", "й")])  # а = filter, й = quit
        app.run()
        self.assertEqual(app.screen.filter, "working")


class BackgroundTestRunTest(unittest.TestCase):
    def test_results_stream_and_finish(self):
        from ppp import netcheck
        from ppp.specs import parse_spec
        from ppp.tui import model as model_mod

        rows = [(0, "1.1.1.1:80", parse_spec("1.1.1.1:80")), (1, "2.2.2.2:80", parse_spec("2.2.2.2:80"))]
        saved = (netcheck.direct_ip, netcheck.test_one, netcheck.save_health)
        netcheck.direct_ip = lambda *_a, **_k: "9.9.9.9"  # type: ignore[assignment]
        netcheck.test_one = lambda idx, proxy, timeout: netcheck.TestResult(idx, proxy, True, "8.8.8.8", 5, "")  # type: ignore[assignment]
        netcheck.save_health = lambda results: None  # type: ignore[assignment]
        try:
            run = model_mod.TestRun(rows)
            collected = []
            for _ in range(200):
                collected.extend(run.drain())
                if run.finished:
                    break
                time.sleep(0.01)
        finally:
            netcheck.direct_ip, netcheck.test_one, netcheck.save_health = saved

        self.assertTrue(run.finished)
        self.assertEqual(run.done, 2)
        self.assertEqual(run.good, 2)
        self.assertEqual(len(collected), 2)


if __name__ == "__main__":
    unittest.main()
