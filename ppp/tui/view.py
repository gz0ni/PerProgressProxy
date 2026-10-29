"""Pure rendering: turns a ViewState into Frame lines. No terminal access."""

from __future__ import annotations

from dataclasses import dataclass, field

STYLES = {
    "dim": "2",
    "bold": "1",
    "green": "32",
    "red": "31",
    "yellow": "33",
    "cyan": "36",
    "reverse": "7",
}

RULE_CHAR = "-"


def fmt_age(seconds: float) -> str:
    if seconds < 90:
        return f"{int(seconds)}s"
    if seconds < 5400:
        return f"{int(seconds // 60)}m"
    return f"{int(seconds // 3600)}h"


@dataclass
class Line:
    """A row of styled text segments."""

    parts: list[tuple[str, str]] = field(default_factory=list)

    def add(self, text: object, style: str = "") -> "Line":
        if text:
            self.parts.append((str(text), style))
        return self

    def plain(self) -> str:
        return "".join(text for text, _style in self.parts)

    def __len__(self) -> int:
        return sum(len(text) for text, _style in self.parts)

    def crop(self, width: int) -> "Line":
        out = Line()
        left = max(0, width)
        for text, style in self.parts:
            if left <= 0:
                break
            out.add(text[:left], style)
            left -= len(text)
        return out

    def slice(self, start: int, end: int) -> "Line":
        out = Line()
        pos = 0
        for text, style in self.parts:
            seg_end = pos + len(text)
            if seg_end > start and pos < end:
                out.add(text[max(start, pos) - pos:min(end, seg_end) - pos], style)
            pos = seg_end
            if pos >= end:
                break
        return out

    def pad(self, width: int) -> "Line":
        out = self.crop(width)
        gap = width - len(out)
        if gap > 0:
            out.add(" " * gap)
        return out

    def __add__(self, other: "Line") -> "Line":
        return Line(list(self.parts) + list(other.parts))

    def render(self, color: bool = True) -> str:
        if not color:
            return self.plain()
        chunks = []
        for text, style in self.parts:
            code = STYLES.get(style)
            chunks.append(f"\x1b[{code}m{text}\x1b[0m" if code else text)
        return "".join(chunks)


@dataclass
class Frame:
    lines: list[Line] = field(default_factory=list)
    cursor: tuple[int, int] | None = None

    def render(self, color: bool = True) -> str:
        if not color:
            return "\n".join(line.plain() for line in self.lines)
        body = "\x1b[K\r\n".join(line.render(color) for line in self.lines) + "\x1b[K"
        if self.cursor is not None:
            row, col = self.cursor
            body += f"\x1b[?25h\x1b[{row + 1};{col + 1}H"
        else:
            body += "\x1b[?25l"
        return body


# -- view model ------------------------------------------------------------


@dataclass
class ProxyView:
    index: int
    display: str
    health: dict | None = None


@dataclass
class ProcessView:
    pid: int
    exe: str
    alive: bool
    current: int = 0
    age: float = 0.0


@dataclass
class TestView:
    done: int
    total: int


@dataclass
class PasteModal:
    lines: list[str] = field(default_factory=lambda: [""])
    row: int = 0
    col: int = 0
    note: str = ""


@dataclass
class AttachModal:
    entries: list[tuple[int, str]] = field(default_factory=list)
    selected: int = 0
    manual: str = ""
    note: str = ""


@dataclass
class InputModal:
    title: str = ""
    value: str = ""
    hint: str = ""


@dataclass
class ConfirmModal:
    title: str = ""
    body: list[str] = field(default_factory=list)


@dataclass
class HelpModal:
    pass


@dataclass
class ViewState:
    enabled: bool = True
    current_index: int = 0
    current_display: str = ""
    rotate_sec: str = "0"
    proxy_total: int = 0
    rows: list[ProxyView] = field(default_factory=list)
    selected: int = 0
    scroll: int = 0
    filter_name: str = "all"
    query: str = ""
    states: list[ProcessView] = field(default_factory=list)
    direct_ip: str = ""
    test: TestView | None = None
    message: str = ""
    modal: object | None = None


# -- helpers ---------------------------------------------------------------

HEALTH_NOTE_MAX = 22


def _health_text(health: dict | None, direct: str) -> tuple[str, str]:
    if not health:
        return "untested", "dim"
    if not health.get("ok"):
        note = (health.get("note") or "failed")[:HEALTH_NOTE_MAX]
        return f"FAIL {note}", "red"
    ms = health.get("ms", 0)
    ip = health.get("ip") or "-"
    if direct and ip == direct:
        return f"same {ms}ms {ip}", "yellow"
    return f"ok {ms}ms {ip}", "green"


def _rule(width: int) -> Line:
    return Line().add(RULE_CHAR * max(0, width), "dim")


def _hints(items: list[tuple[str, str]], width: int) -> Line:
    line = Line().add(" ")
    for key, label in items:
        line.add(f"[{key}]", "bold").add(label + " ")
    return line.crop(width)


def _center(text: str, width: int) -> Line:
    if len(text) >= width:
        return Line().add(text[:width])
    left = max(0, (width - len(text)) // 2)
    return Line().add(" " * left + text)


# -- sections --------------------------------------------------------------


def _header(state: ViewState, width: int) -> Line:
    line = Line().add(" PerProcessProxy ", "cyan")
    line.add(" ")
    if state.enabled:
        line.add("PROXY: ON ", "green")
    else:
        line.add("PROXY: OFF", "yellow")
    line.add(" ")
    if state.proxy_total:
        line.add(f"#{state.current_index}", "bold").add(" ").add(state.current_display or "-")
    else:
        line.add("no proxies", "dim")
    line.add(" ")
    rotate = state.rotate_sec
    line.add("rotate " + (f"{rotate}s" if rotate not in ("0", "") else "manual"), "dim")
    if state.direct_ip and width > 80:
        tail = f"direct {state.direct_ip} "
        filler = width - len(line) - len(tail) - 1
        if filler > 0:
            line.add(" " * filler)
            line.add(tail, "dim")
    return line.pad(width)


def _proxy_header(state: ViewState, width: int) -> Line:
    shown = len(state.rows)
    line = Line().add(" PROXIES ", "dim")
    line.add(f" {state.proxy_total} total, {shown} shown", "")
    line.add("  filter: ", "dim").add(state.filter_name, "cyan")
    if state.query:
        line.add('  search: "', "dim").add(state.query, "cyan").add('"', "dim")
    if state.test is not None:
        line.add("  test ", "dim").add(f"{state.test.done}/{state.test.total}", "yellow")
    return line.crop(width)


def _proxy_line(row: ProxyView, selected: bool, active: bool, width: int, direct: str) -> Line:
    line = Line()
    line.add("> " if selected else "  ", "cyan" if selected else "")
    line.add("* " if active else "  ", "cyan" if active else "dim")
    line.add(f"{row.index:>3}  ", "bold" if selected else "")
    text, style = _health_text(row.health, direct)
    column = max(16, width - len(text) - 13)
    line.add(row.display[:column].ljust(column), "bold" if selected else "")
    line.add(text, style)
    return line.crop(width)


def _proxy_rows(state: ViewState, width: int, height: int) -> list[Line]:
    out: list[Line] = []
    if not state.rows:
        if state.query or state.filter_name != "all":
            msg = "nothing matches this filter - press f to widen it"
        elif state.proxy_total == 0:
            msg = "proxies.txt is empty - press v and paste a proxy or a list URL"
        else:
            msg = "no proxies"
        out.append(Line().add("   " + msg, "dim").crop(width))
        return out
    current = state.current_index
    for offset in range(height):
        pos = state.scroll + offset
        if pos >= len(state.rows):
            out.append(Line())
            continue
        row = state.rows[pos]
        out.append(_proxy_line(row, pos == state.selected, row.index == current, width, state.direct_ip))
    return out


def _process_header(state: ViewState, width: int) -> Line:
    alive = sum(1 for st in state.states if st.alive)
    line = Line().add(" PROCESSES ", "dim")
    line.add(f" {alive} alive / {len(state.states)} recorded", "")
    return line.crop(width)


def _process_rows(state: ViewState, width: int, height: int) -> list[Line]:
    out: list[Line] = []
    if not state.states:
        out.append(Line().add('   none - press a to attach, or l to launch "C:\\path\\app.exe"', "dim").crop(width))
        return out
    limit = max(1, height)
    for st in state.states[:limit]:
        line = Line().add(f"   {st.pid:>7}  ")
        line.add(st.exe[:24].ljust(24), "")
        line.add("alive " if st.alive else "gone  ", "green" if st.alive else "red")
        line.add(f" #{st.current:<3} " if st.alive else " -   ")
        line.add(f"seen {fmt_age(st.age)} ago", "dim")
        out.append(line.crop(width))
    if len(state.states) > limit:
        out.append(Line().add(f"   + {len(state.states) - limit} more (clean with c)", "dim").crop(width))
    return out


def _footer(state: ViewState, width: int) -> list[Line]:
    first = _hints([
        ("v", "paste"), ("a", "attach/kill"), ("l", "launch"), ("t", "test"),
        ("n", "next"), ("p", "prev"), ("o", "on/off"), ("r", "rotate"),
    ], width)
    second = _hints([
        ("/", "search"), ("f", "filter"), ("d", "delete"), ("c", "clean"),
        ("?", "help"), ("q", "quit"),
    ], width)
    second.add("   j/k or arrows scroll", "dim")
    return [first, second]


def _message(state: ViewState, width: int) -> Line:
    if not state.message:
        return Line()
    return Line().add(" ! ", "yellow").add(state.message, "yellow").crop(width)


# -- modals ----------------------------------------------------------------


def _box_top(title: str, width: int) -> Line:
    title = title[: max(0, width - 6)]
    fill = max(0, width - 5 - len(title))
    return Line().add("+- ", "dim").add(title, "bold").add(" " + "-" * fill + "+", "dim")


def _box_body(text: str, width: int, style: str = "") -> Line:
    inner = max(0, width - 4)
    return Line().add("| ", "dim").add(text[:inner].ljust(inner), style).add(" |", "dim")


def _box_bottom(width: int) -> Line:
    return Line().add("+" + "-" * max(0, width - 2) + "+", "dim")


def _paste_box(modal: PasteModal, width: int, height: int) -> tuple[list[Line], tuple[int, int] | None]:
    box_width = min(width - 2, 76)
    inner = max(10, box_width - 4)
    visible = max(3, min(height - 7, 12))
    box: list[Line] = [_box_top("v: paste proxies or a list URL", box_width)]
    start = 0
    if modal.row >= visible:
        start = modal.row - visible + 1
    for offset in range(visible):
        pos = start + offset
        text = modal.lines[pos] if pos < len(modal.lines) else ""
        box.append(_box_body(text, box_width))
    footer = modal.note or "Enter = new line   F2 = add   Esc = cancel"
    box.append(_box_body(footer, box_width, "yellow" if modal.note else "dim"))
    box.append(_box_bottom(box_width))
    cursor = (1 + modal.row - start, 2 + min(modal.col, inner - 1))
    return box, cursor


def _attach_box(modal: AttachModal, width: int, height: int) -> tuple[list[Line], tuple[int, int] | None]:
    box_width = min(width - 2, 62)
    rows = max(3, min(height - 9, 12))
    box: list[Line] = [_box_top("a: attach or kill a process", box_width)]
    if modal.entries:
        start = 0
        if modal.selected >= rows:
            start = modal.selected - rows + 1
        for offset in range(rows):
            pos = start + offset
            if pos >= len(modal.entries):
                box.append(_box_body("", box_width))
                continue
            pid, label = modal.entries[pos]
            mark = "> " if pos == modal.selected else "  "
            box.append(_box_body(f"{mark}{pid:>7}  {label}", box_width, "bold" if pos == modal.selected else ""))
    else:
        box.append(_box_body("no running processes listed", box_width, "dim"))
        for _ in range(rows - 1):
            box.append(_box_body("", box_width))
    box.append(_box_body("manual: " + modal.manual, box_width, "cyan"))
    footer = modal.note or "Enter = attach   x = kill   type pid/exe   Esc = cancel"
    box.append(_box_body(footer, box_width, "yellow" if modal.note else "dim"))
    box.append(_box_bottom(box_width))
    cursor = (len(box) - 3, 10 + min(len(modal.manual), box_width - 12))
    return box, cursor


def _input_box(modal: InputModal, width: int) -> tuple[list[Line], tuple[int, int] | None]:
    box_width = min(width - 2, 70)
    box = [
        _box_top(modal.title or "input", box_width),
        _box_body("", box_width),
        _box_body("> " + modal.value, box_width, "cyan"),
        _box_body("", box_width),
        _box_body(modal.hint or "Enter = apply   Esc = cancel", box_width, "dim"),
        _box_bottom(box_width),
    ]
    cursor = (2, 4 + min(len(modal.value), box_width - 6))
    return box, cursor


def _confirm_box(modal: ConfirmModal, width: int) -> tuple[list[Line], tuple[int, int] | None]:
    box_width = min(width - 2, 68)
    box: list[Line] = [_box_top(modal.title or "confirm", box_width)]
    for text in modal.body:
        box.append(_box_body(text, box_width))
    if not modal.body:
        box.append(_box_body("", box_width))
    box.append(_box_body("", box_width))
    box.append(_box_body("Enter/y = yes    Esc/n = no", box_width, "dim"))
    box.append(_box_bottom(box_width))
    return box, None


HELP_LINES = [
    "j/k, arrows    move in the proxy list",
    "PgUp/PgDn      move a page, Home/End jump",
    "Enter          use the selected proxy",
    "n / p          next / previous proxy",
    "o              proxying on / pass-through",
    "r              automatic rotation interval",
    "t              test every proxy in the background",
    "/              search by host, port or scheme",
    "f              cycle filter: all/working/untested/failed/same",
    "d              delete the selected proxy",
    "v              paste proxy lines or an http(s) list URL",
    "a              attach to a running process (x = kill)",
    "l              launch a program already injected",
    "c              drop state files of dead processes",
    "q              quit",
]


def _help_box(width: int) -> tuple[list[Line], tuple[int, int] | None]:
    box_width = min(width - 2, 72)
    box: list[Line] = [_box_top("help - hotkeys", box_width)]
    for text in HELP_LINES:
        box.append(_box_body(text, box_width))
    box.append(_box_body("", box_width))
    box.append(_box_body("Esc / ? = close", box_width, "dim"))
    box.append(_box_bottom(box_width))
    return box, None


def modal_box(modal: object, width: int, height: int) -> tuple[list[Line], tuple[int, int] | None]:
    if isinstance(modal, PasteModal):
        return _paste_box(modal, width, height)
    if isinstance(modal, AttachModal):
        return _attach_box(modal, width, height)
    if isinstance(modal, InputModal):
        return _input_box(modal, width)
    if isinstance(modal, ConfirmModal):
        return _confirm_box(modal, width)
    if isinstance(modal, HelpModal):
        return _help_box(width)
    return [Line().add("?")], None


def _overlay(base: list[Line], box: list[Line], top: int, left: int, width: int) -> list[Line]:
    out = list(base)
    for offset, box_line in enumerate(box):
        row = top + offset
        if row < 0 or row >= len(out):
            continue
        line = out[row].pad(width)
        out[row] = line.slice(0, left) + box_line + line.slice(left + len(box_line), width)
    return out


# -- entry point -----------------------------------------------------------


def render(state: ViewState, width: int, height: int) -> Frame:
    width = max(20, int(width))
    height = max(8, int(height))

    proc_rows = max(1, min(len(state.states), 3))
    fixed = 9 + proc_rows
    view_height = max(1, height - fixed)

    lines: list[Line] = [_header(state, width), _rule(width), _proxy_header(state, width)]
    lines.extend(_proxy_rows(state, width, view_height))
    lines.append(_rule(width))
    lines.append(_process_header(state, width))
    lines.extend(_process_rows(state, width, proc_rows))
    lines.append(_rule(width))
    lines.extend(_footer(state, width))
    lines.append(_message(state, width))

    lines = [line.pad(width) for line in lines][:height]
    while len(lines) < height:
        lines.append(Line().add(" " * width))

    cursor = None
    if state.modal is not None:
        box, relative = modal_box(state.modal, width, height)
        box = [line.pad(len(box[0])) for line in box]
        top = max(0, min(height - len(box), (height - len(box)) // 2))
        left = max(0, (width - len(box[0])) // 2)
        lines = _overlay(lines, box, top, left, width)
        if relative is not None:
            cursor = (top + relative[0], left + relative[1])
    return Frame(lines, cursor)
