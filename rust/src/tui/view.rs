use std::ops::Add;

use crate::netcheck::HealthEntry;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Style {
    Plain,
    Dim,
    Bold,
    Green,
    Red,
    Yellow,
    Cyan,
}

impl Style {
    fn code(self) -> Option<&'static str> {
        match self {
            Style::Plain => None,
            Style::Dim => Some("2"),
            Style::Bold => Some("1"),
            Style::Green => Some("32"),
            Style::Red => Some("31"),
            Style::Yellow => Some("33"),
            Style::Cyan => Some("36"),
        }
    }
}

const RULE_CHAR: &str = "-";

pub fn char_len(text: &str) -> usize {
    text.chars().count()
}

pub fn take_chars(text: &str, n: usize) -> String {
    text.chars().take(n).collect()
}

fn skip_chars(text: &str, n: usize) -> String {
    text.chars().skip(n).collect()
}

fn ljust(text: &str, width: usize) -> String {
    let len = char_len(text);
    if len >= width {
        text.to_string()
    } else {
        format!("{}{}", text, " ".repeat(width - len))
    }
}

pub fn fmt_age(seconds: f64) -> String {
    if seconds < 90.0 {
        format!("{}s", seconds as i64)
    } else if seconds < 5400.0 {
        format!("{}m", (seconds / 60.0) as i64)
    } else {
        format!("{}h", (seconds / 3600.0) as i64)
    }
}

#[derive(Clone, Debug, Default)]
pub struct Line {
    pub parts: Vec<(String, Style)>,
}

impl Default for Style {
    fn default() -> Style {
        Style::Plain
    }
}

impl Line {
    pub fn new() -> Line {
        Line::default()
    }

    pub fn add(mut self, text: impl AsRef<str>, style: Style) -> Line {
        let text = text.as_ref();
        if !text.is_empty() {
            self.parts.push((text.to_string(), style));
        }
        self
    }

    pub fn plain(&self) -> String {
        self.parts.iter().map(|(text, _)| text.as_str()).collect()
    }

    pub fn len(&self) -> usize {
        self.parts.iter().map(|(text, _)| char_len(text)).sum()
    }

    pub fn crop(&self, width: usize) -> Line {
        let mut out = Line::new();
        let mut left = width;
        for (text, style) in &self.parts {
            if left == 0 {
                break;
            }
            out = out.add(take_chars(text, left), *style);
            left = left.saturating_sub(char_len(text));
        }
        out
    }

    pub fn slice(&self, start: usize, end: usize) -> Line {
        let mut out = Line::new();
        let mut pos = 0;
        for (text, style) in &self.parts {
            let seg_end = pos + char_len(text);
            if seg_end > start && pos < end {
                let from = start.max(pos) - pos;
                let to = end.min(seg_end) - pos;
                out = out.add(take_chars(&skip_chars(text, from), to - from), *style);
            }
            pos = seg_end;
            if pos >= end {
                break;
            }
        }
        out
    }

    pub fn pad(&self, width: usize) -> Line {
        let out = self.crop(width);
        let gap = width.saturating_sub(out.len());
        if gap > 0 { out.add(" ".repeat(gap), Style::Plain) } else { out }
    }

    pub fn render(&self, color: bool) -> String {
        if !color {
            return self.plain();
        }
        let mut out = String::new();
        for (text, style) in &self.parts {
            match style.code() {
                Some(code) => {
                    out.push_str("\x1b[");
                    out.push_str(code);
                    out.push('m');
                    out.push_str(text);
                    out.push_str("\x1b[0m");
                }
                None => out.push_str(text),
            }
        }
        out
    }
}

impl Add for Line {
    type Output = Line;

    fn add(mut self, other: Line) -> Line {
        self.parts.extend(other.parts);
        self
    }
}

#[derive(Clone, Debug, Default)]
pub struct Frame {
    pub lines: Vec<Line>,
    pub cursor: Option<(usize, usize)>,
}

impl Frame {
    pub fn render(&self, color: bool) -> String {
        if !color {
            return self.lines.iter().map(Line::plain).collect::<Vec<_>>().join("\n");
        }
        let rows: Vec<String> = self.lines.iter().map(|line| line.render(true)).collect();
        let mut body = rows.join("\x1b[K\r\n");
        body.push_str("\x1b[K");
        match self.cursor {
            Some((row, col)) => body.push_str(&format!("\x1b[?25h\x1b[{};{}H", row + 1, col + 1)),
            None => body.push_str("\x1b[?25l"),
        }
        body
    }
}

#[derive(Clone, Debug)]
pub struct ProxyView {
    pub index: usize,
    pub display: String,
    pub health: Option<HealthEntry>,
}

#[derive(Clone, Debug)]
pub struct ProcessView {
    pub pid: u32,
    pub exe: String,
    pub alive: bool,
    pub current: i64,
    pub age: f64,
}

#[derive(Clone, Debug)]
pub struct TestView {
    pub done: usize,
    pub total: usize,
}

#[derive(Clone, Debug)]
pub struct PasteModal {
    pub lines: Vec<String>,
    pub row: usize,
    pub col: usize,
    pub note: String,
}

impl Default for PasteModal {
    fn default() -> PasteModal {
        PasteModal { lines: vec![String::new()], row: 0, col: 0, note: String::new() }
    }
}

#[derive(Clone, Debug, Default)]
pub struct AttachModal {
    pub entries: Vec<(u32, String)>,
    pub selected: usize,
    pub manual: String,
    pub note: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InputKind {
    Rotate,
    Launch,
    Search,
}

#[derive(Clone, Debug)]
pub struct InputModal {
    pub kind: InputKind,
    pub title: String,
    pub value: String,
    pub hint: String,
}

#[derive(Clone, Debug, Default)]
pub struct ConfirmModal {
    pub title: String,
    pub body: Vec<String>,
}

#[derive(Clone, Debug)]
pub enum Modal {
    Paste(PasteModal),
    Attach(AttachModal),
    Input(InputModal),
    Confirm(ConfirmModal),
    Help,
}

#[derive(Clone, Debug)]
pub struct ViewState {
    pub enabled: bool,
    pub current_index: i64,
    pub current_display: String,
    pub rotate_sec: String,
    pub proxy_total: usize,
    pub rows: Vec<ProxyView>,
    pub selected: usize,
    pub scroll: usize,
    pub filter_name: String,
    pub query: String,
    pub states: Vec<ProcessView>,
    pub direct_ip: String,
    pub test: Option<TestView>,
    pub message: String,
    pub modal: Option<Modal>,
}

impl Default for ViewState {
    fn default() -> ViewState {
        ViewState {
            enabled: true,
            current_index: 0,
            current_display: String::new(),
            rotate_sec: "0".to_string(),
            proxy_total: 0,
            rows: Vec::new(),
            selected: 0,
            scroll: 0,
            filter_name: "all".to_string(),
            query: String::new(),
            states: Vec::new(),
            direct_ip: String::new(),
            test: None,
            message: String::new(),
            modal: None,
        }
    }
}

const HEALTH_NOTE_MAX: usize = 22;

fn health_text(health: &Option<HealthEntry>, direct: &str) -> (String, Style) {
    let Some(health) = health else {
        return ("untested".to_string(), Style::Dim);
    };
    if !health.ok {
        let note = if health.note.is_empty() { "failed" } else { health.note.as_str() };
        return (format!("FAIL {}", take_chars(note, HEALTH_NOTE_MAX)), Style::Red);
    }
    let ip = if health.ip.is_empty() { "-" } else { health.ip.as_str() };
    if !direct.is_empty() && ip == direct {
        return (format!("same {}ms {}", health.ms, ip), Style::Yellow);
    }
    (format!("ok {}ms {}", health.ms, ip), Style::Green)
}

fn rule(width: usize) -> Line {
    Line::new().add(RULE_CHAR.repeat(width), Style::Dim)
}

fn hints(items: &[(&str, &str)], width: usize) -> Line {
    let mut line = Line::new().add(" ", Style::Plain);
    for (key, label) in items {
        line = line.add(format!("[{}]", key), Style::Bold).add(format!("{} ", label), Style::Plain);
    }
    line.crop(width)
}

fn header(state: &ViewState, width: usize) -> Line {
    let mut line = Line::new().add(" PerProcessProxy ", Style::Cyan).add(" ", Style::Plain);
    line = if state.enabled {
        line.add("PROXY: ON ", Style::Green)
    } else {
        line.add("PROXY: OFF", Style::Yellow)
    };
    line = line.add(" ", Style::Plain);
    line = if state.proxy_total > 0 {
        let shown = if state.current_display.is_empty() { "-" } else { state.current_display.as_str() };
        line.add(format!("#{}", state.current_index), Style::Bold).add(" ", Style::Plain).add(shown, Style::Plain)
    } else {
        line.add("no proxies", Style::Dim)
    };
    line = line.add(" ", Style::Plain);
    let rotate = state.rotate_sec.as_str();
    let rotate_text = if rotate != "0" && !rotate.is_empty() { format!("{}s", rotate) } else { "manual".to_string() };
    line = line.add(format!("rotate {}", rotate_text), Style::Dim);
    if !state.direct_ip.is_empty() && width > 80 {
        let tail = format!("direct {} ", state.direct_ip);
        let used = line.len() + char_len(&tail) + 1;
        if width > used {
            line = line.add(" ".repeat(width - used), Style::Plain).add(tail, Style::Dim);
        }
    }
    line.pad(width)
}

fn proxy_header(state: &ViewState, width: usize) -> Line {
    let mut line = Line::new().add(" PROXIES ", Style::Dim);
    line = line.add(format!(" {} total, {} shown", state.proxy_total, state.rows.len()), Style::Plain);
    line = line.add("  filter: ", Style::Dim).add(&state.filter_name, Style::Cyan);
    if !state.query.is_empty() {
        line = line.add("  search: \"", Style::Dim).add(&state.query, Style::Cyan).add("\"", Style::Dim);
    }
    if let Some(test) = &state.test {
        line = line.add("  test ", Style::Dim).add(format!("{}/{}", test.done, test.total), Style::Yellow);
    }
    line.crop(width)
}

fn proxy_line(row: &ProxyView, selected: bool, active: bool, width: usize, direct: &str) -> Line {
    let mut line = Line::new();
    line = line.add(if selected { "> " } else { "  " }, if selected { Style::Cyan } else { Style::Plain });
    line = line.add(if active { "* " } else { "  " }, if active { Style::Cyan } else { Style::Dim });
    line = line.add(format!("{:>3}  ", row.index), if selected { Style::Bold } else { Style::Plain });
    let (text, style) = health_text(&row.health, direct);
    let column = width.saturating_sub(char_len(&text) + 13).max(16);
    line = line.add(ljust(&take_chars(&row.display, column), column), if selected { Style::Bold } else { Style::Plain });
    line = line.add(text, style);
    line.crop(width)
}

fn proxy_rows(state: &ViewState, width: usize, height: usize) -> Vec<Line> {
    let mut out = Vec::new();
    if state.rows.is_empty() {
        let msg = if !state.query.is_empty() || state.filter_name != "all" {
            "nothing matches this filter - press f to widen it"
        } else if state.proxy_total == 0 {
            "the proxy list is empty - press v and paste a proxy or a list URL"
        } else {
            "no proxies"
        };
        out.push(Line::new().add(format!("   {}", msg), Style::Dim).crop(width));
        return out;
    }
    for offset in 0..height {
        let pos = state.scroll + offset;
        match state.rows.get(pos) {
            None => out.push(Line::new()),
            Some(row) => {
                let active = row.index as i64 == state.current_index;
                out.push(proxy_line(row, pos == state.selected, active, width, &state.direct_ip));
            }
        }
    }
    out
}

fn process_header(state: &ViewState, width: usize) -> Line {
    let alive = state.states.iter().filter(|st| st.alive).count();
    Line::new()
        .add(" PROCESSES ", Style::Dim)
        .add(format!(" {} alive / {} recorded", alive, state.states.len()), Style::Plain)
        .crop(width)
}

fn process_rows(state: &ViewState, width: usize, height: usize) -> Vec<Line> {
    let mut out = Vec::new();
    if state.states.is_empty() {
        out.push(
            Line::new().add("   none - press a to attach, or l to launch \"C:\\path\\app.exe\"", Style::Dim).crop(width),
        );
        return out;
    }
    let limit = height.max(1);
    for st in state.states.iter().take(limit) {
        let mut line = Line::new().add(format!("   {:>7}  ", st.pid), Style::Plain);
        line = line.add(ljust(&take_chars(&st.exe, 24), 24), Style::Plain);
        line = line.add(if st.alive { "alive " } else { "gone  " }, if st.alive { Style::Green } else { Style::Red });
        line = line.add(if st.alive { format!(" #{:<3} ", st.current) } else { " -   ".to_string() }, Style::Plain);
        line = line.add(format!("seen {} ago", fmt_age(st.age)), Style::Dim);
        out.push(line.crop(width));
    }
    if state.states.len() > limit {
        out.push(Line::new().add(format!("   + {} more (clean with c)", state.states.len() - limit), Style::Dim).crop(width));
    }
    out
}

fn footer(width: usize) -> Vec<Line> {
    let first = hints(
        &[
            ("v", "paste"),
            ("a", "attach/kill"),
            ("l", "launch"),
            ("t", "test"),
            ("n", "next"),
            ("p", "prev"),
            ("o", "on/off"),
            ("r", "rotate"),
        ],
        width,
    );
    let second = hints(
        &[("/", "search"), ("f", "filter"), ("d", "delete"), ("c", "clean"), ("?", "help"), ("q", "quit")],
        width,
    )
    .add("   j/k or arrows scroll", Style::Dim);
    vec![first, second]
}

fn message_line(state: &ViewState, width: usize) -> Line {
    if state.message.is_empty() {
        return Line::new();
    }
    Line::new().add(" ! ", Style::Yellow).add(&state.message, Style::Yellow).crop(width)
}

fn box_top(title: &str, width: usize) -> Line {
    let title = take_chars(title, width.saturating_sub(6));
    let fill = width.saturating_sub(5 + char_len(&title));
    Line::new()
        .add("+- ", Style::Dim)
        .add(&title, Style::Bold)
        .add(format!(" {}+", "-".repeat(fill)), Style::Dim)
}

fn box_body(text: &str, width: usize, style: Style) -> Line {
    let inner = width.saturating_sub(4);
    Line::new().add("| ", Style::Dim).add(ljust(&take_chars(text, inner), inner), style).add(" |", Style::Dim)
}

fn box_bottom(width: usize) -> Line {
    Line::new().add(format!("+{}+", "-".repeat(width.saturating_sub(2))), Style::Dim)
}

type Boxed = (Vec<Line>, Option<(usize, usize)>);

fn paste_box(modal: &PasteModal, width: usize, height: usize) -> Boxed {
    let box_width = width.saturating_sub(2).min(76);
    let inner = box_width.saturating_sub(4).max(10);
    let visible = height.saturating_sub(7).min(12).max(3);
    let mut out = vec![box_top("v: paste proxies or a list URL", box_width)];
    let start = if modal.row >= visible { modal.row - visible + 1 } else { 0 };
    for offset in 0..visible {
        let text = modal.lines.get(start + offset).map(String::as_str).unwrap_or("");
        out.push(box_body(text, box_width, Style::Plain));
    }
    let (footer_text, style) = if modal.note.is_empty() {
        ("Enter = new line   F2 = add   Esc = cancel", Style::Dim)
    } else {
        (modal.note.as_str(), Style::Yellow)
    };
    out.push(box_body(footer_text, box_width, style));
    out.push(box_bottom(box_width));
    let cursor = (1 + modal.row - start, 2 + modal.col.min(inner - 1));
    (out, Some(cursor))
}

fn attach_box(modal: &AttachModal, width: usize, height: usize) -> Boxed {
    let box_width = width.saturating_sub(2).min(62);
    let rows = height.saturating_sub(9).min(12).max(3);
    let mut out = vec![box_top("a: attach or kill a process", box_width)];
    if modal.entries.is_empty() {
        out.push(box_body("no running processes listed", box_width, Style::Dim));
        for _ in 1..rows {
            out.push(box_body("", box_width, Style::Plain));
        }
    } else {
        let start = if modal.selected >= rows { modal.selected - rows + 1 } else { 0 };
        for offset in 0..rows {
            let pos = start + offset;
            match modal.entries.get(pos) {
                None => out.push(box_body("", box_width, Style::Plain)),
                Some((pid, label)) => {
                    let mark = if pos == modal.selected { "> " } else { "  " };
                    let style = if pos == modal.selected { Style::Bold } else { Style::Plain };
                    out.push(box_body(&format!("{}{:>7}  {}", mark, pid, label), box_width, style));
                }
            }
        }
    }
    out.push(box_body(&format!("manual: {}", modal.manual), box_width, Style::Cyan));
    let (footer_text, style) = if modal.note.is_empty() {
        ("Enter = attach   x = kill   type pid/exe   Esc = cancel", Style::Dim)
    } else {
        (modal.note.as_str(), Style::Yellow)
    };
    out.push(box_body(footer_text, box_width, style));
    out.push(box_bottom(box_width));
    let cursor = (out.len() - 3, 10 + char_len(&modal.manual).min(box_width.saturating_sub(12)));
    (out, Some(cursor))
}

fn input_box(modal: &InputModal, width: usize) -> Boxed {
    let box_width = width.saturating_sub(2).min(70);
    let title = if modal.title.is_empty() { "input" } else { modal.title.as_str() };
    let hint = if modal.hint.is_empty() { "Enter = apply   Esc = cancel" } else { modal.hint.as_str() };
    let out = vec![
        box_top(title, box_width),
        box_body("", box_width, Style::Plain),
        box_body(&format!("> {}", modal.value), box_width, Style::Cyan),
        box_body("", box_width, Style::Plain),
        box_body(hint, box_width, Style::Dim),
        box_bottom(box_width),
    ];
    let cursor = (2, 4 + char_len(&modal.value).min(box_width.saturating_sub(6)));
    (out, Some(cursor))
}

fn confirm_box(modal: &ConfirmModal, width: usize) -> Boxed {
    let box_width = width.saturating_sub(2).min(68);
    let title = if modal.title.is_empty() { "confirm" } else { modal.title.as_str() };
    let mut out = vec![box_top(title, box_width)];
    for text in &modal.body {
        out.push(box_body(text, box_width, Style::Plain));
    }
    if modal.body.is_empty() {
        out.push(box_body("", box_width, Style::Plain));
    }
    out.push(box_body("", box_width, Style::Plain));
    out.push(box_body("Enter/y = yes    Esc/n = no", box_width, Style::Dim));
    out.push(box_bottom(box_width));
    (out, None)
}

pub const HELP_LINES: [&str; 15] = [
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
];

fn help_box(width: usize) -> Boxed {
    let box_width = width.saturating_sub(2).min(72);
    let mut out = vec![box_top("help - hotkeys", box_width)];
    for text in HELP_LINES {
        out.push(box_body(text, box_width, Style::Plain));
    }
    out.push(box_body("", box_width, Style::Plain));
    out.push(box_body("Esc / ? = close", box_width, Style::Dim));
    out.push(box_bottom(box_width));
    (out, None)
}

fn modal_box(modal: &Modal, width: usize, height: usize) -> Boxed {
    match modal {
        Modal::Paste(m) => paste_box(m, width, height),
        Modal::Attach(m) => attach_box(m, width, height),
        Modal::Input(m) => input_box(m, width),
        Modal::Confirm(m) => confirm_box(m, width),
        Modal::Help => help_box(width),
    }
}

fn overlay(base: Vec<Line>, boxed: &[Line], top: usize, left: usize, width: usize) -> Vec<Line> {
    let mut out = base;
    for (offset, box_line) in boxed.iter().enumerate() {
        let row = top + offset;
        if row >= out.len() {
            continue;
        }
        let line = out[row].pad(width);
        out[row] = line.slice(0, left) + box_line.clone() + line.slice(left + box_line.len(), width);
    }
    out
}

pub fn render(state: &ViewState, width: usize, height: usize) -> Frame {
    let width = width.max(20);
    let height = height.max(8);

    let proc_rows = state.states.len().min(3).max(1);
    let fixed = 9 + proc_rows;
    let view_height = height.saturating_sub(fixed).max(1);

    let mut lines = vec![header(state, width), rule(width), proxy_header(state, width)];
    lines.extend(proxy_rows(state, width, view_height));
    lines.push(rule(width));
    lines.push(process_header(state, width));
    lines.extend(process_rows(state, width, proc_rows));
    lines.push(rule(width));
    lines.extend(footer(width));
    lines.push(message_line(state, width));

    let mut lines: Vec<Line> = lines.iter().map(|line| line.pad(width)).take(height).collect();
    while lines.len() < height {
        lines.push(Line::new().add(" ".repeat(width), Style::Plain));
    }

    let mut cursor = None;
    if let Some(modal) = &state.modal {
        let (boxed, relative) = modal_box(modal, width, height);
        let box_width = boxed.first().map(Line::len).unwrap_or(0);
        let boxed: Vec<Line> = boxed.iter().map(|line| line.pad(box_width)).collect();
        let top = if boxed.len() >= height { 0 } else { (height - boxed.len()) / 2 };
        let left = width.saturating_sub(box_width) / 2;
        lines = overlay(lines, &boxed, top, left, width);
        if let Some((row, col)) = relative {
            cursor = Some((top + row, left + col));
        }
    }
    Frame { lines, cursor }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_row(index: usize) -> ProxyView {
        let health = match index % 3 {
            0 => Some(HealthEntry { ok: true, ms: 100 + index as u64, ip: format!("10.0.0.{}", index % 250), ..Default::default() }),
            1 => Some(HealthEntry { ok: false, note: "timeout".to_string(), ..Default::default() }),
            _ => None,
        };
        ProxyView { index, display: format!("10.0.{}.{}:{}", index / 250, index % 250, 8000 + index), health }
    }

    fn make_state(rows: usize, selected: usize, scroll: usize) -> ViewState {
        ViewState {
            current_display: "10.0.0.0:8000".to_string(),
            proxy_total: rows,
            rows: (0..rows).map(make_row).collect(),
            selected,
            scroll,
            states: vec![ProcessView { pid: 1234, exe: "Telegram.exe".to_string(), alive: true, current: 3, age: 12.0 }],
            ..Default::default()
        }
    }

    fn text_of(frame: &Frame) -> String {
        frame.lines.iter().map(Line::plain).collect::<Vec<_>>().join("\n")
    }

    #[test]
    fn frame_is_exactly_width_by_height() {
        for (width, height) in [(100, 30), (80, 24), (40, 12), (200, 50)] {
            let frame = render(&make_state(300, 0, 0), width, height);
            assert_eq!(frame.lines.len(), height);
            for line in &frame.lines {
                assert_eq!(line.len(), width, "{}", line.plain());
            }
        }
    }

    #[test]
    fn long_proxy_list_does_not_fill_the_screen() {
        let frame = render(&make_state(1000, 0, 0), 100, 24);
        assert_eq!(frame.lines.len(), 24);
        assert!(!text_of(&frame).contains("10.0.1.0:8000"));
    }

    #[test]
    fn scroll_window_follows_selection() {
        let text = text_of(&render(&make_state(300, 200, 200), 100, 24));
        assert!(text.contains("10.0.0.200:8200"));
        assert!(text.contains('>'));
    }

    #[test]
    fn current_proxy_is_marked() {
        assert!(text_of(&render(&make_state(5, 0, 0), 100, 24)).contains('*'));
    }

    #[test]
    fn empty_list_explains_itself() {
        let state = ViewState::default();
        assert!(text_of(&render(&state, 100, 24)).contains("the proxy list is empty"));
    }

    #[test]
    fn test_progress_visible() {
        let mut state = make_state(300, 0, 0);
        state.test = Some(TestView { done: 12, total: 300 });
        assert!(text_of(&render(&state, 120, 30)).contains("12/300"));
    }

    #[test]
    fn message_line_is_shown() {
        let mut state = make_state(300, 0, 0);
        state.message = "added 3 proxies".to_string();
        assert!(text_of(&render(&state, 100, 24)).contains("added 3 proxies"));
    }

    #[test]
    fn colour_switch() {
        let frame = render(&make_state(300, 0, 0), 100, 24);
        assert!(!frame.render(false).contains('\x1b'));
        assert!(frame.render(true).contains("\x1b["));
    }

    #[test]
    fn narrow_terminal_is_safe() {
        let frame = render(&make_state(300, 0, 0), 20, 8);
        assert_eq!(frame.lines.len(), 8);
        for line in &frame.lines {
            assert_eq!(line.len(), 20);
        }
    }

    fn check_overlay(modal: Modal) -> Frame {
        let mut state = make_state(300, 0, 0);
        state.modal = Some(modal);
        let frame = render(&state, 100, 30);
        assert_eq!(frame.lines.len(), 30);
        for line in &frame.lines {
            assert_eq!(line.len(), 100);
        }
        frame
    }

    #[test]
    fn paste_modal_has_cursor() {
        let frame = check_overlay(Modal::Paste(PasteModal {
            lines: vec!["socks5://1.2.3.4:1080".to_string()],
            row: 0,
            col: 5,
            note: String::new(),
        }));
        let (row, col) = frame.cursor.expect("cursor");
        assert!(row < 30 && col < 100);
        assert!(text_of(&frame).contains("paste"));
    }

    #[test]
    fn paste_modal_scrolls_long_text() {
        let lines: Vec<String> = (0..40).map(|i| format!("10.0.0.{}:8080", i)).collect();
        let frame = check_overlay(Modal::Paste(PasteModal { lines, row: 39, col: 0, note: String::new() }));
        assert!(text_of(&frame).contains("10.0.0.39:8080"));
    }

    #[test]
    fn attach_modal() {
        let frame = check_overlay(Modal::Attach(AttachModal {
            entries: vec![(1234, "Telegram.exe".to_string()), (5678, "app.exe".to_string())],
            selected: 1,
            ..Default::default()
        }));
        let text = text_of(&frame);
        assert!(text.contains("Telegram.exe"));
        assert!(text.contains("attach"));
        assert!(frame.cursor.is_some());
    }

    #[test]
    fn input_modal() {
        let frame = check_overlay(Modal::Input(InputModal {
            kind: InputKind::Rotate,
            title: "r: rotation".to_string(),
            value: "120".to_string(),
            hint: "seconds".to_string(),
        }));
        assert!(text_of(&frame).contains("120"));
        assert!(frame.cursor.is_some());
    }

    #[test]
    fn confirm_modal() {
        let frame = check_overlay(Modal::Confirm(ConfirmModal {
            title: "delete".to_string(),
            body: vec!["#3  1.2.3.4:8080".to_string()],
        }));
        let text = text_of(&frame);
        assert!(text.contains("1.2.3.4:8080"));
        assert!(text.contains("Enter/y"));
    }

    #[test]
    fn help_modal_shows_hotkeys() {
        let text = text_of(&check_overlay(Modal::Help));
        assert!(text.contains("hotkeys"));
        assert!(text.contains("filter"));
    }

    #[test]
    fn modals_fit_tiny_terminals() {
        for modal in [
            Modal::Paste(PasteModal::default()),
            Modal::Attach(AttachModal::default()),
            Modal::Help,
        ] {
            let mut state = make_state(10, 0, 0);
            state.modal = Some(modal);
            let frame = render(&state, 20, 8);
            assert_eq!(frame.lines.len(), 8);
            for line in &frame.lines {
                assert_eq!(line.len(), 20);
            }
        }
    }
}
