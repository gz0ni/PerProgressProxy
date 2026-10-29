"""Raw terminal access: keyboard decoding, terminal size, ANSI output.

Standard library only. Keyboard decoding is a pure function over text so it can
be tested without a TTY; the Terminal class only wraps the OS-specific reads.
"""

from __future__ import annotations

import codecs
import os
import select
import sys
import time
from dataclasses import dataclass

IS_WINDOWS = os.name == "nt"

ESC = "\x1b"

# Keys the UI cares about. "char" carries the printable character in .char.
KEY_NAMES = (
    "char", "enter", "tab", "esc", "backspace", "delete", "insert",
    "up", "down", "left", "right", "home", "end", "pgup", "pgdn",
    "f1", "f2", "f3", "f4", "f5", "f6", "f7", "f8", "f9", "f10", "f11", "f12",
    "ctrl_c", "ctrl_s", "ctrl_v", "ctrl_u", "unknown",
)


@dataclass(frozen=True)
class Key:
    name: str
    char: str = ""

    def __post_init__(self) -> None:
        if self.name not in KEY_NAMES:
            raise ValueError(f"unknown key name: {self.name}")


CSI_FINAL = {
    "A": "up", "B": "down", "C": "right", "D": "left",
    "H": "home", "F": "end", "E": "enter",
}

CSI_TILDE = {
    "1": "home", "2": "insert", "3": "delete", "4": "end", "5": "pgup",
    "6": "pgdn", "7": "home", "8": "end",
    "11": "f1", "12": "f2", "13": "f3", "14": "f4", "15": "f5",
    "17": "f6", "18": "f7", "19": "f8", "20": "f9", "21": "f10",
    "23": "f11", "24": "f12",
}

SS3 = {
    "A": "up", "B": "down", "C": "right", "D": "left",
    "P": "f1", "Q": "f2", "R": "f3", "S": "f4", "H": "home", "F": "end",
}

# msvcrt returns 0x00 / 0xe0 followed by a scan code character.
WINDOWS_SCAN = {
    "H": "up", "P": "down", "K": "left", "M": "right",
    "I": "pgup", "Q": "pgdn", "G": "home", "O": "end",
    "R": "insert", "S": "delete",
    ";": "f1", "<": "f2", "=": "f3", ">": "f4", "?": "f5",
    "@": "f6", "A": "f7", "B": "f8", "C": "f9", "D": "f10",
}

CONTROL = {
    "\r": Key("enter"), "\n": Key("enter"),
    "\t": Key("tab"),
    "\x7f": Key("backspace"), "\x08": Key("backspace"),
    "\x03": Key("ctrl_c"), "\x13": Key("ctrl_s"), "\x16": Key("ctrl_v"), "\x15": Key("ctrl_u"),
}


def _escape(text: str, i: int) -> tuple[Key | None, int]:
    """Decode one escape sequence at text[i]; None means "incomplete"."""
    if i + 1 >= len(text):
        return None, 0
    nxt = text[i + 1]
    if nxt == "[":
        j = i + 2
        while j < len(text) and not ("@" <= text[j] <= "~"):
            j += 1
        if j >= len(text):
            return None, 0
        body, final = text[i + 2:j], text[j]
        if final == "~":
            name = CSI_TILDE.get(body.split(";")[0], "unknown")
        else:
            name = CSI_FINAL.get(final, "unknown")
        return Key(name), j - i + 1
    if nxt == "O":
        if i + 2 >= len(text):
            return None, 0
        return Key(SS3.get(text[i + 2], "unknown")), 3
    return Key("esc"), 1


def split_keys(text: str) -> tuple[list[Key], str]:
    """Split decoded input into keys; an incomplete tail is returned as rest."""
    keys: list[Key] = []
    i = 0
    while i < len(text):
        ch = text[i]
        if ch == ESC:
            key, used = _escape(text, i)
            if key is None:
                return keys, text[i:]
            keys.append(key)
            i += used
            continue
        if ch in ("\x00", "\xe0"):
            if i + 1 >= len(text):
                return keys, text[i:]
            keys.append(Key(WINDOWS_SCAN.get(text[i + 1], "unknown")))
            i += 2
            continue
        known = CONTROL.get(ch)
        if known is not None:
            keys.append(known)
        elif ch.isprintable():
            keys.append(Key("char", ch))
        else:
            keys.append(Key("unknown"))
        i += 1
    return keys, ""


def plain_text(value: str) -> list[Key]:
    """Convenience for tests and for replaying a pasted snippet."""
    return split_keys(value)[0]


class Terminal:
    """Owns the real terminal state; call open() before drawing and close() after."""

    def __init__(self, stream=None) -> None:
        self.stream = stream if stream is not None else sys.stdout
        self.width = 100
        self.height = 30
        self._buf = ""
        self._decoder = None
        self._fd = None
        self._saved = None
        self._closed = False

    # -- lifecycle --------------------------------------------------------

    def open(self) -> None:
        if IS_WINDOWS:
            self._enable_vt()
        else:
            import termios
            import tty

            self._fd = sys.stdin.fileno()
            self._saved = termios.tcgetattr(self._fd)
            tty.setraw(self._fd)
            self._decoder = codecs.getincrementaldecoder("utf-8")("replace")
        self.update_size()
        self.write("\x1b[?1049h\x1b[?25l\x1b[2J\x1b[H")

    def close(self) -> None:
        if self._closed:
            return
        self._closed = True
        try:
            self.write("\x1b[?25h\x1b[0m\x1b[?1049l")
        except OSError:
            pass
        if not IS_WINDOWS and self._saved is not None:
            import termios

            try:
                termios.tcsetattr(self._fd, termios.TCSADRAIN, self._saved)
            except OSError:
                pass

    def is_tty(self) -> bool:
        if not bool(getattr(self.stream, "isatty", lambda: False)()):
            return False
        if IS_WINDOWS:
            return True
        try:
            return sys.stdin.isatty()
        except (AttributeError, ValueError):
            return False

    # -- output -----------------------------------------------------------

    def write(self, text: str) -> None:
        try:
            self.stream.write(text)
            self.stream.flush()
        except UnicodeEncodeError:
            encoding = getattr(self.stream, "encoding", "ascii") or "ascii"
            self.stream.write(text.encode(encoding, "replace").decode(encoding))
            self.stream.flush()

    def draw(self, text: str) -> None:
        self.write("\x1b[H" + text)

    def update_size(self) -> None:
        import shutil

        size = shutil.get_terminal_size(fallback=(100, 30))
        self.width = max(20, size.columns)
        self.height = max(8, size.lines)

    # -- input ------------------------------------------------------------

    def read_key(self, timeout: float = 0.3) -> Key | None:
        deadline = time.monotonic() + timeout
        while True:
            if self._buf:
                keys, rest = split_keys(self._buf)
                if keys:
                    self._buf = rest
                    return keys[0]
                if not rest.startswith(("\x1b", "\x00", "\xe0")):  # unparsable: drop it
                    self._buf = ""
                    return Key("unknown")
            remaining = deadline - time.monotonic()
            if remaining <= 0:
                if self._buf:
                    self._buf = ""
                    return Key("esc")
                return None
            if IS_WINDOWS:
                import msvcrt

                if msvcrt.kbhit():
                    self._buf += msvcrt.getwch()
                    continue
                time.sleep(min(0.02, remaining))
            elif select.select([self._fd], [], [], remaining)[0]:
                data = os.read(self._fd, 2048)
                if not data:
                    return Key("esc")
                self._buf += self._decoder.decode(data)

    def _enable_vt(self) -> None:
        try:
            import ctypes

            k32 = ctypes.windll.kernel32
            handle = k32.GetStdHandle(-11)
            mode = ctypes.c_uint32()
            if k32.GetConsoleMode(handle, ctypes.byref(mode)):
                k32.SetConsoleMode(handle, mode.value | 0x0004)
        except Exception:
            pass
