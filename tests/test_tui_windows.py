from __future__ import annotations

import sys
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent.parent))

from ppp.tui.term import (
    EVENT_FOCUS,
    EVENT_KEY,
    EVENT_MOUSE,
    EVENT_RESIZE,
    LEFT_ALT,
    LEFT_CTRL,
    RIGHT_ALT,
    SHIFT,
    VK_ESCAPE,
    Key,
    map_windows_key,
    record_key,
)


class MapWindowsKeyTest(unittest.TestCase):
    def test_escape(self):
        self.assertEqual(map_windows_key(VK_ESCAPE, 0, "\x1b"), Key("esc"))

    def test_alt_shift_does_not_close_dialogs(self):
        # Switching the keyboard layout sends Esc together with Alt+Shift.
        self.assertIsNone(map_windows_key(VK_ESCAPE, LEFT_ALT | SHIFT, ""))
        self.assertIsNone(map_windows_key(VK_ESCAPE, RIGHT_ALT, "\x1b"))

    def test_altgr_still_types_characters(self):
        self.assertEqual(map_windows_key(ord("Q"), RIGHT_ALT | LEFT_CTRL, "@"), Key("char", "@"))

    def test_arrows_and_paging(self):
        self.assertEqual(
            [map_windows_key(vk, 0, "") for vk in (0x26, 0x28, 0x25, 0x27, 0x21, 0x22)],
            [Key("up"), Key("down"), Key("left"), Key("right"), Key("pgup"), Key("pgdn")],
        )
        self.assertEqual(
            [map_windows_key(vk, 0, "") for vk in (0x24, 0x23, 0x2D, 0x2E)],
            [Key("home"), Key("end"), Key("insert"), Key("delete")],
        )

    def test_function_keys_up_to_f12(self):
        self.assertEqual(
            [map_windows_key(vk, 0, "") for vk in (0x70, 0x71, 0x74, 0x7B)],
            [Key("f1"), Key("f2"), Key("f5"), Key("f12")],
        )

    def test_control_combinations(self):
        self.assertEqual(map_windows_key(ord("C"), LEFT_CTRL, "\x03"), Key("ctrl_c"))
        self.assertEqual(map_windows_key(ord("S"), LEFT_CTRL, "\x13"), Key("ctrl_s"))
        self.assertIsNone(map_windows_key(ord("X"), LEFT_CTRL, "\x18"))

    def test_cyrillic_character_passes_through(self):
        self.assertEqual(map_windows_key(ord("M"), 0, "м"), Key("char", "м"))

    def test_modifier_only_events_are_ignored(self):
        self.assertIsNone(map_windows_key(0x10, SHIFT, ""))  # Shift
        self.assertIsNone(map_windows_key(0x11, LEFT_CTRL, ""))  # Ctrl
        self.assertIsNone(map_windows_key(0x12, LEFT_ALT, ""))  # Alt

    def test_enter_tab_backspace(self):
        self.assertEqual(
            [map_windows_key(vk, 0, ch) for vk, ch in ((0x0D, "\r"), (0x09, "\t"), (0x08, "\x08"))],
            [Key("enter"), Key("tab"), Key("backspace")],
        )


class RecordKeyTest(unittest.TestCase):
    def test_key_down_is_mapped(self):
        self.assertEqual(record_key(EVENT_KEY, True, 0x26, 0, ""), Key("up"))

    def test_key_up_is_ignored(self):
        self.assertIsNone(record_key(EVENT_KEY, False, 0x26, 0, ""))

    def test_mouse_and_focus_events_are_ignored(self):
        for event in (EVENT_MOUSE, EVENT_FOCUS, EVENT_RESIZE):
            self.assertIsNone(record_key(event, True, 0x26, 0, "x"))




class AttachEscapeTest(unittest.TestCase):
    def make_app(self):
        from ppp.tui.app import App
        from ppp.tui.view import AttachModal

        app = App.__new__(App)
        app.modal = AttachModal(entries=[(1, "a.exe")], manual="abc", note="x")
        return app

    def test_first_escape_clears_manual_input(self):
        app = self.make_app()
        modal = app.modal
        app._attach_key(modal, Key("esc"))
        self.assertIs(app.modal, modal)
        self.assertEqual((modal.manual, modal.note), ("", ""))

    def test_second_escape_closes_modal(self):
        app = self.make_app()
        modal = app.modal
        app._attach_key(modal, Key("esc"))
        app._attach_key(modal, Key("esc"))
        self.assertIsNone(app.modal)


if __name__ == "__main__":
    unittest.main()
