"""Interactive console: main loop, hotkeys, modal dialogs."""

from __future__ import annotations

import os
import threading

from .. import platform as backend, processes, store
from . import term as term_mod
from .model import Screen, download, is_url
from .view import (
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

# Latin and Cyrillic keys, so the console never needs a layout switch.
HOTKEYS = {
    "v": "paste", "м": "paste",
    "a": "attach", "ф": "attach",
    "l": "launch", "д": "launch",
    "t": "test", "е": "test",
    "n": "next", "т": "next",
    "p": "prev", "з": "prev",
    "o": "power", "щ": "power",
    "r": "rotate", "к": "rotate",
    "f": "filter", "а": "filter",
    "d": "delete", "в": "delete",
    "c": "clean", "с": "clean",
    "?": "help",
    "/": "search",
    "q": "quit", "й": "quit",
}


class App:
    def __init__(self) -> None:
        self.term = term_mod.Terminal()
        self.screen = Screen()
        self.modal: object | None = None
        self.color = not os.environ.get("NO_COLOR")
        self.page = 10
        self._pending = None
        self._size: tuple[int, int] | None = None

    # -- lifecycle --------------------------------------------------------

    def run(self) -> int:
        if not self.term.is_tty():
            print("the interactive console needs a terminal; use one-shot commands instead")
            return 1
        self.screen.reload(force=True)
        try:
            self.term.open()
        except Exception as exc:  # no console to talk to
            print(f"cannot start the console: {exc}")
            return 1
        try:
            while True:
                self._tick()
                self._draw()
                key = self.term.read_key(0.3)
                if key is None:
                    continue
                if not self._handle(key):
                    break
        except KeyboardInterrupt:
            pass
        finally:
            self.term.close()
        return 0

    def _tick(self) -> None:
        self.term.update_size()
        self.page = max(3, self.term.height - 12)
        self.screen.reload()
        self.screen.poll_test()
        self.screen.clamp(self.page)

    def _draw(self) -> None:
        size = (self.term.width, self.term.height)
        if size != self._size:
            self._size = size
            self.term.write("\x1b[2J")
        frame = render(self._state(), self.term.width, self.term.height)
        self.term.draw(frame.render(self.color and self.term.is_tty()))

    def _state(self) -> ViewState:
        screen = self.screen
        visible = screen.visible()
        states = sorted(screen.states, key=lambda st: (not st.alive, st.pid))
        return ViewState(
            enabled=screen.enabled,
            current_index=screen.current,
            current_display=screen.current_display(),
            rotate_sec=screen.rotate_sec,
            proxy_total=len(screen.rows),
            rows=[ProxyView(index, proxy.display, screen.health.get(proxy.label)) for index, _text, proxy in visible],
            selected=screen.selected,
            scroll=screen.scroll,
            filter_name=screen.filter,
            query=screen.query,
            states=[
                ProcessView(st.pid, st.exe, st.alive, st.current, st.age)
                for st in states
            ],
            direct_ip=screen.direct,
            test=TestView(screen.test.done, screen.test.total) if screen.test else None,
            message=screen.message,
            modal=self.modal,
        )

    # -- keys -------------------------------------------------------------

    def _handle(self, key: term_mod.Key) -> bool:
        if key.name == "ctrl_c":
            if self.modal is not None:
                self.modal = None
                self._pending = None
                return True
            return False
        if self.modal is not None:
            self._modal_key(key)
            return True
        if key.name == "char":
            return self._hotkey(key.char)
        if key.name == "up":
            self.screen.move(-1)
        elif key.name == "down":
            self.screen.move(1)
        elif key.name == "pgup":
            self.screen.move(-self.page)
        elif key.name == "pgdn":
            self.screen.move(self.page)
        elif key.name == "home":
            self.screen.jump("home")
        elif key.name == "end":
            self.screen.jump("end")
        elif key.name == "enter":
            if self.screen.visible():
                self._say(self.screen.use_selected())
        return True

    def _hotkey(self, char: str) -> bool:
        action = HOTKEYS.get(char)
        if action is None:
            if char.isdigit():
                self._use_index(int(char))
            return True
        if action == "quit":
            return False
        if action == "paste":
            self.modal = PasteModal()
        elif action == "attach":
            self._open_attach()
        elif action == "launch":
            self.modal = InputModal("l: launch an injected program", "", r'full path, e.g. "C:\Program Files\App\app.exe"')
        elif action == "test":
            self._say(self.screen.start_test())
        elif action == "next":
            self._say(self.screen.step(1))
        elif action == "prev":
            self._say(self.screen.step(-1))
        elif action == "power":
            self._say(self.screen.toggle_power())
        elif action == "rotate":
            self.modal = InputModal("r: automatic rotation (0 = off)", self.screen.rotate_sec, "seconds between rotations")
        elif action == "filter":
            self.screen.cycle_filter()
        elif action == "delete":
            self._ask_delete()
        elif action == "clean":
            self._say(self.screen.clean())
        elif action == "help":
            self.modal = HelpModal()
        elif action == "search":
            self.modal = InputModal("/: search proxies", self.screen.query, "host, port or scheme   empty = show all")
        return True

    def _use_index(self, digit: int) -> None:
        rows = self.screen.visible()
        if not rows:
            return
        position = min(digit, len(rows) - 1)
        self.screen.selected = position
        self._say(self.screen.use_selected())

    def _say(self, message: str) -> None:
        if message:
            self.screen.message = message

    # -- modals -----------------------------------------------------------

    def _modal_key(self, key: term_mod.Key) -> None:
        modal = self.modal
        if isinstance(modal, PasteModal):
            self._paste_key(modal, key)
        elif isinstance(modal, AttachModal):
            self._attach_key(modal, key)
        elif isinstance(modal, InputModal):
            self._input_key(modal, key)
        elif isinstance(modal, ConfirmModal):
            self._confirm_key(modal, key)
        else:
            if key.name in ("esc", "enter") or (key.name == "char" and key.char in "?qй"):
                self.modal = None

    def _paste_key(self, modal: PasteModal, key: term_mod.Key) -> None:
        if key.name == "esc":
            self.modal = None
            return
        if key.name in ("ctrl_s", "f2"):
            self._apply_paste(modal)
            return
        line = modal.lines[modal.row]
        if key.name == "enter":
            modal.lines[modal.row:modal.row + 1] = [line[:modal.col], line[modal.col:]]
            modal.row += 1
            modal.col = 0
        elif key.name == "backspace":
            if modal.col > 0:
                modal.lines[modal.row] = line[:modal.col - 1] + line[modal.col:]
                modal.col -= 1
            elif modal.row > 0:
                previous = modal.lines[modal.row - 1]
                modal.lines[modal.row - 1:modal.row + 1] = [previous + line]
                modal.row -= 1
                modal.col = len(previous)
        elif key.name == "delete":
            if modal.col < len(line):
                modal.lines[modal.row] = line[:modal.col] + line[modal.col + 1:]
            elif modal.row < len(modal.lines) - 1:
                modal.lines[modal.row:modal.row + 2] = [line + modal.lines[modal.row + 1]]
        elif key.name == "left":
            if modal.col > 0:
                modal.col -= 1
            elif modal.row > 0:
                modal.row -= 1
                modal.col = len(modal.lines[modal.row])
        elif key.name == "right":
            if modal.col < len(line):
                modal.col += 1
            elif modal.row < len(modal.lines) - 1:
                modal.row += 1
                modal.col = 0
        elif key.name == "up":
            modal.row = max(0, modal.row - 1)
            modal.col = min(modal.col, len(modal.lines[modal.row]))
        elif key.name == "down":
            modal.row = min(len(modal.lines) - 1, modal.row + 1)
            modal.col = min(modal.col, len(modal.lines[modal.row]))
        elif key.name == "home":
            modal.col = 0
        elif key.name == "end":
            modal.col = len(line)
        elif key.name == "ctrl_v":
            pass
        elif key.name == "char":
            modal.lines[modal.row] = line[:modal.col] + key.char + line[modal.col:]
            modal.col += len(key.char)

    def _apply_paste(self, modal: PasteModal) -> None:
        text = "\n".join(modal.lines).strip()
        if not text:
            self.modal = None
            return
        urls: list[str] = []
        specs: list[str] = []
        for line in text.splitlines():
            item = line.strip()
            if not item:
                continue
            if is_url(item):
                urls.append(item)
            else:
                specs.append(item)
        if specs:
            added, skipped, total = store.import_text("\n".join(specs))
            self.screen.reload(force=True)
            self._say(f"added {added}, skipped {skipped}, total {total}")
        for url in urls:
            self._fetch(url)
        self.modal = None

    def _fetch(self, url: str) -> None:
        self._say(f"downloading {url[:48]}...")

        def work() -> None:
            try:
                raw = download(url)
            except Exception as exc:
                self.screen.notice = f"download failed: {exc}"
                return
            added, skipped, total = store.import_text(raw)
            self.screen.notice = f"list: added {added}, skipped {skipped}, total {total}"

        threading.Thread(target=work, daemon=True).start()

    def _open_attach(self) -> None:
        entries, note = self._process_entries()
        self.modal = AttachModal(entries, 0, "", note)

    def _process_entries(self) -> tuple[list[tuple[int, str]], str]:
        try:
            _rc, lines = backend.list_processes()
        except (FileNotFoundError, backend.UnsupportedPlatform) as exc:
            return [], str(exc)
        entries: list[tuple[int, str]] = []
        for line in lines:
            parts = line.split(None, 2)
            if len(parts) < 3 or not parts[0].isdigit():
                continue
            entries.append((int(parts[0]), parts[2].strip()))
        return entries[:200], ""

    def _attach_key(self, modal: AttachModal, key: term_mod.Key) -> None:
        if key.name == "esc":
            if modal.manual:
                modal.manual = ""
                modal.note = ""
            else:
                self.modal = None
            return
        if key.name == "up":
            modal.selected = max(0, modal.selected - 1)
            return
        if key.name == "down":
            modal.selected = min(max(0, len(modal.entries) - 1), modal.selected + 1)
            return
        if key.name == "backspace":
            modal.manual = modal.manual[:-1]
            return
        if key.name == "enter":
            self._attach(modal)
            return
        if key.name == "char":
            if key.char in "xч" and modal.entries and not modal.manual:
                self._kill(modal)
            else:
                modal.manual += key.char

    def _attach(self, modal: AttachModal) -> None:
        target = modal.manual.strip()
        if not target and modal.entries:
            target = str(modal.entries[min(modal.selected, len(modal.entries) - 1)][0])
        if not target:
            modal.note = "type a pid or an exe name first"
            return
        self.modal = None
        try:
            if target.isdigit():
                _rc, lines = backend.inject_pid(int(target))
            else:
                _rc, lines = backend.inject_name(target)
        except (FileNotFoundError, backend.UnsupportedPlatform) as exc:
            self._say(f"attach: {exc}")
            return
        self._say("; ".join(lines[-2:]) if lines else f"attached to {target}")
        self.screen.reload(force=True)

    def _kill(self, modal: AttachModal) -> None:
        if not modal.entries:
            return
        pid, label = modal.entries[min(modal.selected, len(modal.entries) - 1)]
        ok = processes.kill_process(pid)
        processes.clean_dead()
        modal.note = f"{label}: {'terminated' if ok else 'not running or access denied'}"
        entries, _note = self._process_entries()
        modal.entries = entries
        modal.selected = min(modal.selected, max(0, len(entries) - 1))

    def _input_key(self, modal: InputModal, key: term_mod.Key) -> None:
        if key.name == "esc":
            self.modal = None
        elif key.name == "enter":
            self._submit(modal)
        elif key.name == "backspace":
            modal.value = modal.value[:-1]
        elif key.name == "char":
            modal.value += key.char

    def _submit(self, modal: InputModal) -> None:
        title = modal.title
        value = modal.value.strip()
        if title.startswith("r:"):
            try:
                seconds = int(value or "0")
            except ValueError:
                modal.hint = "whole seconds, for example 120 (0 = off)"
                return
            self.modal = None
            self._say(self.screen.set_rotate(seconds))
        elif title.startswith("l:"):
            if not value:
                modal.hint = "a command is required"
                return
            self.modal = None
            try:
                _rc, lines = backend.launch(value)
            except (FileNotFoundError, backend.UnsupportedPlatform) as exc:
                self._say(f"launch: {exc}")
                return
            self._say("; ".join(lines[-2:]) if lines else f"launched {value}")
            self.screen.reload(force=True)
        elif title.startswith("/:"):
            self.modal = None
            self.screen.set_query(value)
            self._say(f'search: "{value}"' if value else "search cleared")

    def _ask_delete(self) -> None:
        row = self.screen.selected_row()
        if row is None:
            self._say("nothing to delete")
            return
        self.modal = ConfirmModal(
            "d: delete proxy",
            [f"#{row[0]}  {row[2].display}", "", "this removes the line from proxies.txt"],
        )
        self._pending = ("delete", row[0])

    def _confirm_key(self, modal: ConfirmModal, key: term_mod.Key) -> None:
        yes = key.name == "enter" or (key.name == "char" and key.char in "yднY")
        no = key.name == "esc" or (key.name == "char" and key.char in "nлN")
        if not yes and not no:
            return
        self.modal = None
        pending, self._pending = self._pending, None
        if not yes or pending is None:
            return
        if pending[0] == "delete":
            removed = store.remove_indices([pending[1]])
            self.screen.reload(force=True)
            self._say(f"removed {removed} proxy line(s)")


def run() -> int:
    return App().run()
