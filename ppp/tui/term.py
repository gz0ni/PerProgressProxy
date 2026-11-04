"""Raw terminal access: keyboard decoding, terminal size, ANSI output.

Standard library only. Keyboard decoding is a pure function over text (or over
Windows console records) so it can be tested without a TTY; the Terminal class
only wraps the OS-specific reads.
"""

from __future__ import annotations

import codecs
import ctypes
import os
import select
import sys
import time
from collections import deque
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


# -- Windows console records ------------------------------------------------

EVENT_KEY = 0x0001
EVENT_MOUSE = 0x0002
EVENT_RESIZE = 0x0004
EVENT_MENU = 0x0008
EVENT_FOCUS = 0x0010

RIGHT_ALT = 0x0001
LEFT_ALT = 0x0002
RIGHT_CTRL = 0x0004
LEFT_CTRL = 0x0008
SHIFT = 0x0010

ALT_MASK = RIGHT_ALT | LEFT_ALT
CTRL_MASK = RIGHT_CTRL | LEFT_CTRL

VK_BACK = 0x08
VK_TAB = 0x09
VK_RETURN = 0x0D
VK_ESCAPE = 0x1B
VK_PRIOR = 0x21
VK_NEXT = 0x22
VK_END = 0x23
VK_HOME = 0x24
VK_LEFT = 0x25
VK_UP = 0x26
VK_RIGHT = 0x27
VK_DOWN = 0x28
VK_INSERT = 0x2D
VK_DELETE = 0x2E
VK_F1 = 0x70
VK_F12 = 0x7B

VK_NAV = {
    VK_UP: "up", VK_DOWN: "down", VK_LEFT: "left", VK_RIGHT: "right",
    VK_PRIOR: "pgup", VK_NEXT: "pgdn", VK_HOME: "home", VK_END: "end",
    VK_INSERT: "insert", VK_DELETE: "delete", VK_RETURN: "enter",
    VK_TAB: "tab", VK_BACK: "backspace",
}

VK_CTRL = {ord("C"): "ctrl_c", ord("S"): "ctrl_s", ord("V"): "ctrl_v", ord("U"): "ctrl_u"}


def map_windows_key(vk: int, control: int, char: str) -> Key | None:
    """Translate one console key event, or None when it must be ignored.

    Alt combinations never produce Esc: keyboard-layout switches (Alt+Shift)
    and window shortcuts must not close a modal dialog.
    """
    if control & ALT_MASK:
        if char and char.isprintable():  # AltGr text, e.g. "@" on some layouts
            return Key("char", char)
        return None
    if vk == VK_ESCAPE:
        return Key("esc")
    name = VK_NAV.get(vk)
    if name is not None:
        return Key(name)
    if VK_F1 <= vk <= VK_F12:
        return Key(f"f{vk - VK_F1 + 1}")
    if control & CTRL_MASK:
        mapped = VK_CTRL.get(vk)
        return Key(mapped) if mapped else None
    if char and char.isprintable():
        return Key("char", char)
    return None


def record_key(event_type: int, key_down: bool, vk: int, control: int, char: str) -> Key | None:
    """Pure wrapper over a console INPUT_RECORD: only real key presses map to keys."""
    if event_type != EVENT_KEY or not key_down:
        return None
    return map_windows_key(vk, control, char)


class _COORD(ctypes.Structure):
    _fields_ = [("X", ctypes.c_short), ("Y", ctypes.c_short)]


class _KEY_EVENT_RECORD(ctypes.Structure):
    _fields_ = [
        ("bKeyDown", ctypes.c_int),
        ("wRepeatCount", ctypes.c_ushort),
        ("wVirtualKeyCode", ctypes.c_ushort),
        ("wVirtualScanCode", ctypes.c_ushort),
        ("UnicodeChar", ctypes.c_wchar),
        ("dwControlKeyState", ctypes.c_uint),
    ]


class _EVENT_UNION(ctypes.Union):
    _fields_ = [("KeyEvent", _KEY_EVENT_RECORD), ("_pad", ctypes.c_byte * 16)]


class _INPUT_RECORD(ctypes.Structure):
    _fields_ = [("EventType", ctypes.c_ushort), ("Event", _EVENT_UNION)]


_WIN32 = None  # lazily bound kernel32


def _kernel32():
    global _WIN32
    if _WIN32 is None:
        k32 = ctypes.WinDLL("kernel32", use_last_error=True)
        k32.GetStdHandle.restype = ctypes.c_void_p
        k32.GetStdHandle.argtypes = [ctypes.c_uint]
        k32.GetConsoleMode.argtypes = [ctypes.c_void_p, ctypes.POINTER(ctypes.c_uint)]
        k32.SetConsoleMode.argtypes = [ctypes.c_void_p, ctypes.c_uint]
        k32.WaitForSingleObject.argtypes = [ctypes.c_void_p, ctypes.c_uint]
        k32.WaitForSingleObject.restype = ctypes.c_uint
        k32.ReadConsoleInputW.argtypes = [
            ctypes.c_void_p,
            ctypes.POINTER(_INPUT_RECORD),
            ctypes.c_uint,
            ctypes.POINTER(ctypes.c_uint),
        ]
        k32.ReadConsoleInputW.restype = ctypes.c_int
        _WIN32 = k32
    return _WIN32


STD_INPUT_HANDLE = 0xFFFFFFF0 & 0xFFFFFFFF
ENABLE_PROCESSED_INPUT = 0x0001
ENABLE_LINE_INPUT = 0x0002
ENABLE_ECHO_INPUT = 0x0004
ENABLE_WINDOW_INPUT = 0x0008
ENABLE_MOUSE_INPUT = 0x0010
ENABLE_QUICK_EDIT_MODE = 0x0040
ENABLE_EXTENDED_FLAGS = 0x0080
WAIT_OBJECT_0 = 0x00000000
WAIT_TIMEOUT = 0x00000102


def _debug_key(text: str) -> None:
    """Opt-in key trace: PPP_DEBUG_KEYS=1 writes logs/keys.log."""
    if not os.environ.get("PPP_DEBUG_KEYS"):
        return
    try:
        from ..paths import LOGS_DIR

        LOGS_DIR.mkdir(parents=True, exist_ok=True)
        with (LOGS_DIR / "keys.log").open("a", encoding="utf-8") as handle:
            handle.write(f"{time.strftime('%H:%M:%S')} {text}\n")
    except OSError:
        pass


# -- text decoding ----------------------------------------------------------


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
        self._pending: deque[Key] = deque()
        self._win_handle = None
        self._win_mode = None

    # -- lifecycle --------------------------------------------------------

    def open(self) -> None:
        if IS_WINDOWS:
            self._enable_vt()
            self._open_console_input()
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
        if IS_WINDOWS:
            self._close_console_input()
        elif self._saved is not None:
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

    # -- Windows console mode --------------------------------------------

    def _open_console_input(self) -> None:
        try:
            k32 = _kernel32()
            handle = k32.GetStdHandle(STD_INPUT_HANDLE)
            mode = ctypes.c_uint()
            if not handle or not k32.GetConsoleMode(handle, ctypes.byref(mode)):
                return  # redirected input: the msvcrt fallback handles it
            wanted = (mode.value | ENABLE_EXTENDED_FLAGS | ENABLE_WINDOW_INPUT)
            wanted &= ~ENABLE_MOUSE_INPUT & ~ENABLE_QUICK_EDIT_MODE
            wanted &= ~ENABLE_PROCESSED_INPUT & ~ENABLE_LINE_INPUT & ~ENABLE_ECHO_INPUT
            if k32.SetConsoleMode(handle, wanted):
                self._win_handle = handle
                self._win_mode = mode.value
        except (OSError, AttributeError):
            self._win_handle = None

    def _close_console_input(self) -> None:
        if self._win_handle is None or self._win_mode is None:
            return
        try:
            _kernel32().SetConsoleMode(self._win_handle, self._win_mode)
        except (OSError, AttributeError):
            pass
        self._win_handle = None
        self._win_mode = None

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
        if self._pending:
            return self._pending.popleft()
        if IS_WINDOWS and self._win_handle is not None:
            return self._read_console(timeout)
        return self._read_stream(timeout)

    def _read_console(self, timeout: float) -> Key | None:
        k32 = _kernel32()
        deadline = time.monotonic() + timeout
        while True:
            remaining = deadline - time.monotonic()
            if remaining <= 0:
                return None
            wait_ms = max(1, min(int(remaining * 1000), 200))
            if k32.WaitForSingleObject(self._win_handle, wait_ms) != WAIT_OBJECT_0:
                continue
            records = (_INPUT_RECORD * 32)()
            count = ctypes.c_uint()
            if not k32.ReadConsoleInputW(self._win_handle, records, 32, ctypes.byref(count)):
                return None
            for index in range(count.value):
                record = records[index]
                if record.EventType == EVENT_RESIZE:
                    self.update_size()
                    continue
                key_event = record.Event.KeyEvent
                if record.EventType == EVENT_KEY:
                    key = record_key(
                        record.EventType,
                        bool(key_event.bKeyDown),
                        key_event.wVirtualKeyCode,
                        key_event.dwControlKeyState,
                        key_event.UnicodeChar,
                    )
                    _debug_key(
                        f"vk={key_event.wVirtualKeyCode:#04x} ctrl={key_event.dwControlKeyState:#06x} "
                        f"char={key_event.UnicodeChar!r} down={bool(key_event.bKeyDown)} -> "
                        f"{key.name if key else '-'}"
                    )
                    if key is not None:
                        repeats = max(1, min(int(key_event.wRepeatCount), 8))
                        for _ in range(repeats):
                            self._pending.append(key)
                else:
                    _debug_key(f"event={record.EventType:#06x} ignored")
            if self._pending:
                return self._pending.popleft()

    def _read_stream(self, timeout: float) -> Key | None:
        """msvcrt on Windows without a console handle, termios elsewhere."""
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
                leftover, self._buf = self._buf, ""
                if leftover == ESC:
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
            k32 = _kernel32()
            handle = k32.GetStdHandle(-11 & 0xFFFFFFFF)
            mode = ctypes.c_uint()
            if k32.GetConsoleMode(handle, ctypes.byref(mode)):
                k32.SetConsoleMode(handle, mode.value | 0x0004)
        except (OSError, AttributeError):
            pass
