from __future__ import annotations

import sys
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent.parent))

from ppp.tui.term import Key, split_keys


def names(text: str) -> list[str]:
    return [key.name for key in split_keys(text)[0]]


def chars(text: str) -> str:
    return "".join(key.char for key in split_keys(text)[0] if key.name == "char")


class SplitKeysTest(unittest.TestCase):
    def test_plain_text_is_one_key_per_character(self):
        self.assertEqual(names("abc"), ["char", "char", "char"])
        self.assertEqual(chars("abc"), "abc")

    def test_cyrillic_text_survives(self):
        self.assertEqual(chars("мфд"), "мфд")

    def test_ansi_arrows(self):
        self.assertEqual(names("\x1b[A\x1b[B\x1b[C\x1b[D"), ["up", "down", "right", "left"])

    def test_windows_arrows(self):
        self.assertEqual(names("\xe0H\xe0P\xe0K\xe0M"), ["up", "down", "left", "right"])

    def test_paging_and_home_end(self):
        self.assertEqual(
            names("\x1b[5~\x1b[6~\x1b[H\x1b[F\x1b[1~\x1b[4~"),
            ["pgup", "pgdn", "home", "end", "home", "end"],
        )
        self.assertEqual(names("\xe0I\xe0Q\xe0G\xe0O"), ["pgup", "pgdn", "home", "end"])

    def test_function_keys(self):
        self.assertEqual(names("\x1bOQ\x1b[12~\x00<"), ["f2", "f2", "f2"])

    def test_extended_function_keys(self):
        self.assertEqual(names("\x1b[15~\x1b[24~\x1b[21~"), ["f5", "f12", "f10"])

    def test_control_keys(self):
        self.assertEqual(
            names("\x03\x13\x16\r\n\t\x7f"),
            ["ctrl_c", "ctrl_s", "ctrl_v", "enter", "enter", "tab", "backspace"],
        )

    def test_incomplete_escape_is_kept_as_rest(self):
        keys, rest = split_keys("\x1b[")
        self.assertEqual(keys, [])
        self.assertEqual(rest, "\x1b[")

    def test_escape_then_letter(self):
        self.assertEqual(names("\x1bq"), ["esc", "char"])

    def test_unknown_sequence(self):
        self.assertEqual(names("\x1b[99~"), ["unknown"])

    def test_paste_of_multiline_text(self):
        keys, rest = split_keys("1.2.3.4:8080\r\n5.6.7.8:1080")
        self.assertEqual(rest, "")
        self.assertEqual(keys[0], Key("char", "1"))
        self.assertIn("enter", [key.name for key in keys])


if __name__ == "__main__":
    unittest.main()
