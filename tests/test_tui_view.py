from __future__ import annotations

import sys
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent.parent))

from ppp.specs import parse_spec
from ppp.tui.model import Screen
from ppp.tui.view import (
    AttachModal,
    ConfirmModal,
    HelpModal,
    InputModal,
    PasteModal,
    ProcessView,
    ProxyView,
    TestView,
    ViewState,
    render,
)


def make_row(index: int) -> ProxyView:
    health = None
    if index % 3 == 0:
        health = {"ok": True, "ms": 100 + index, "ip": f"10.0.0.{index % 250}"}
    elif index % 3 == 1:
        health = {"ok": False, "ms": 0, "ip": "", "note": "timeout"}
    return ProxyView(index, f"10.0.{index // 250}.{index % 250}:{8000 + index}", health)


def make_state(rows: int = 300, selected: int = 0, scroll: int = 0) -> ViewState:
    return ViewState(
        enabled=True,
        current_index=0,
        current_display="10.0.0.0:8000",
        proxy_total=rows,
        rows=[make_row(i) for i in range(rows)],
        selected=selected,
        scroll=scroll,
        states=[ProcessView(1234, "Telegram.exe", True, 3, 12.0)],
    )


class RenderTest(unittest.TestCase):
    def test_frame_is_exactly_width_by_height(self):
        for width, height in ((100, 30), (80, 24), (40, 12), (200, 50)):
            frame = render(make_state(), width, height)
            self.assertEqual(len(frame.lines), height, (width, height))
            for line in frame.lines:
                self.assertEqual(len(line), width, (width, height, line.plain()))

    def test_long_proxy_list_does_not_fill_the_screen(self):
        frame = render(make_state(rows=1000), 100, 24)
        self.assertEqual(len(frame.lines), 24)
        text = "\n".join(line.plain() for line in frame.lines)
        self.assertNotIn("10.0.1.0:8000", text)  # far rows stay outside the viewport

    def test_scroll_window_follows_selection(self):
        state = make_state(selected=200, scroll=200)
        frame = render(state, 100, 24)
        text = "\n".join(line.plain() for line in frame.lines)
        self.assertIn("10.0.0.200:8200", text)
        self.assertIn(">", text)

    def test_current_proxy_is_marked(self):
        state = make_state(rows=5)
        frame = render(state, 100, 24)
        text = "\n".join(line.plain() for line in frame.lines)
        self.assertIn("*", text)

    def test_empty_list_explains_itself(self):
        state = ViewState(proxy_total=0)
        text = "\n".join(line.plain() for line in render(state, 100, 24).lines)
        self.assertIn("proxies.txt is empty", text)

    def test_test_progress_visible(self):
        state = make_state()
        state.test = TestView(12, 300)
        text = "\n".join(line.plain() for line in render(state, 120, 30).lines)
        self.assertIn("12/300", text)

    def test_message_line(self):
        state = make_state()
        state.message = "added 3 proxies"
        text = "\n".join(line.plain() for line in render(state, 100, 24).lines)
        self.assertIn("added 3 proxies", text)

    def test_no_ansi_when_colour_is_off(self):
        frame = render(make_state(), 100, 24)
        self.assertNotIn("\x1b", frame.render(color=False))

    def test_colour_adds_escapes(self):
        frame = render(make_state(), 100, 24)
        self.assertIn("\x1b[", frame.render(color=True))

    def test_narrow_terminal_is_safe(self):
        frame = render(make_state(), 20, 8)
        self.assertEqual(len(frame.lines), 8)
        for line in frame.lines:
            self.assertEqual(len(line), 20)


class ModalRenderTest(unittest.TestCase):
    def check_overlay(self, modal, width=100, height=30):
        state = make_state()
        state.modal = modal
        frame = render(state, width, height)
        self.assertEqual(len(frame.lines), height)
        for line in frame.lines:
            self.assertEqual(len(line), width)
        return frame

    def test_paste_modal_has_cursor(self):
        frame = self.check_overlay(PasteModal(lines=["socks5://1.2.3.4:1080"], row=0, col=5))
        self.assertIsNotNone(frame.cursor)
        row, col = frame.cursor
        self.assertTrue(0 <= row < 30 and 0 <= col < 100)
        self.assertIn("paste", "\n".join(line.plain() for line in frame.lines))

    def test_paste_modal_scrolls_long_text(self):
        modal = PasteModal(lines=[f"10.0.0.{i}:8080" for i in range(40)], row=39, col=0)
        frame = self.check_overlay(modal)
        text = "\n".join(line.plain() for line in frame.lines)
        self.assertIn("10.0.0.39:8080", text)

    def test_attach_modal(self):
        frame = self.check_overlay(AttachModal(entries=[(1234, "Telegram.exe"), (5678, "app.exe")], selected=1))
        text = "\n".join(line.plain() for line in frame.lines)
        self.assertIn("Telegram.exe", text)
        self.assertIn("attach", text)
        self.assertIsNotNone(frame.cursor)

    def test_input_modal(self):
        frame = self.check_overlay(InputModal("r: rotation", "120", "seconds"))
        text = "\n".join(line.plain() for line in frame.lines)
        self.assertIn("120", text)
        self.assertIsNotNone(frame.cursor)

    def test_confirm_modal(self):
        frame = self.check_overlay(ConfirmModal("delete", ["#3  1.2.3.4:8080"]))
        text = "\n".join(line.plain() for line in frame.lines)
        self.assertIn("1.2.3.4:8080", text)
        self.assertIn("Enter/y", text)

    def test_help_modal_shows_hotkeys(self):
        frame = self.check_overlay(HelpModal())
        text = "\n".join(line.plain() for line in frame.lines)
        self.assertIn("hotkeys", text)
        self.assertIn("filter", text)


class FilterTest(unittest.TestCase):
    def make_screen(self) -> Screen:
        screen = Screen()
        specs = ["1.1.1.1:80", "2.2.2.2:80", "3.3.3.3:80", "4.4.4.4:80"]
        screen.rows = [(i, text, parse_spec(text)) for i, text in enumerate(specs)]
        screen.direct = "9.9.9.9"
        screen.health = {
            screen.rows[0][2].label: {"ok": True, "ms": 10, "ip": "8.8.8.8", "note": ""},
            screen.rows[1][2].label: {"ok": False, "ms": 0, "ip": "", "note": "timeout"},
            screen.rows[2][2].label: {"ok": True, "ms": 20, "ip": "9.9.9.9", "note": ""},
        }
        return screen

    def test_filters(self):
        screen = self.make_screen()
        self.assertEqual(len(screen.visible()), 4)
        screen.filter = "working"
        self.assertEqual([row[0] for row in screen.visible()], [0])
        screen.filter = "untested"
        self.assertEqual([row[0] for row in screen.visible()], [3])
        screen.filter = "failed"
        self.assertEqual([row[0] for row in screen.visible()], [1])
        screen.filter = "same"
        self.assertEqual([row[0] for row in screen.visible()], [2])

    def test_cycle_filter_returns_to_all(self):
        screen = self.make_screen()
        seen = []
        for _ in range(5):
            screen.cycle_filter()
            seen.append(screen.filter)
        self.assertEqual(screen.filter, "all")
        self.assertEqual(seen[0], "working")

    def test_query_matches_host_and_scheme(self):
        screen = self.make_screen()
        screen.set_query("2.2.2")
        self.assertEqual([row[0] for row in screen.visible()], [1])
        screen.set_query("socks4")
        self.assertEqual(screen.visible(), [])

    def test_scroll_clamp_keeps_selection_visible(self):
        screen = self.make_screen()
        screen.selected = 3
        screen.clamp(page=2)
        self.assertEqual(screen.scroll, 2)
        screen.selected = 0
        screen.clamp(page=2)
        self.assertEqual(screen.scroll, 0)

    def test_move_stays_inside_the_list(self):
        screen = self.make_screen()
        screen.move(-5)
        self.assertEqual(screen.selected, 0)
        screen.move(50)
        self.assertEqual(screen.selected, 3)


if __name__ == "__main__":
    unittest.main()
