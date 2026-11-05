use std::collections::VecDeque;
use std::io::{self, IsTerminal, Write};
use std::path::PathBuf;
use std::time::{Duration, Instant};

pub const ESC: char = '\x1b';

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Key {
    Char(char),
    Enter,
    Tab,
    Esc,
    Backspace,
    Delete,
    Insert,
    Up,
    Down,
    Left,
    Right,
    Home,
    End,
    PgUp,
    PgDn,
    F(u8),
    CtrlC,
    CtrlS,
    CtrlV,
    CtrlU,
    Unknown,
}

pub trait Term {
    fn open(&mut self) -> io::Result<()>;
    fn close(&mut self);
    fn is_tty(&self) -> bool;
    fn update_size(&mut self);
    fn size(&self) -> (usize, usize);
    fn draw(&mut self, text: &str);
    fn write(&mut self, text: &str);
    fn read_key(&mut self, timeout: Duration) -> Option<Key>;
}

pub fn is_printable(c: char) -> bool {
    !c.is_control() && (c == ' ' || !c.is_whitespace())
}

fn csi_final(c: char) -> Key {
    match c {
        'A' => Key::Up,
        'B' => Key::Down,
        'C' => Key::Right,
        'D' => Key::Left,
        'H' => Key::Home,
        'F' => Key::End,
        'E' => Key::Enter,
        _ => Key::Unknown,
    }
}

fn csi_tilde(code: &str) -> Key {
    match code {
        "1" | "7" => Key::Home,
        "2" => Key::Insert,
        "3" => Key::Delete,
        "4" | "8" => Key::End,
        "5" => Key::PgUp,
        "6" => Key::PgDn,
        "11" => Key::F(1),
        "12" => Key::F(2),
        "13" => Key::F(3),
        "14" => Key::F(4),
        "15" => Key::F(5),
        "17" => Key::F(6),
        "18" => Key::F(7),
        "19" => Key::F(8),
        "20" => Key::F(9),
        "21" => Key::F(10),
        "23" => Key::F(11),
        "24" => Key::F(12),
        _ => Key::Unknown,
    }
}

fn ss3(c: char) -> Key {
    match c {
        'A' => Key::Up,
        'B' => Key::Down,
        'C' => Key::Right,
        'D' => Key::Left,
        'P' => Key::F(1),
        'Q' => Key::F(2),
        'R' => Key::F(3),
        'S' => Key::F(4),
        'H' => Key::Home,
        'F' => Key::End,
        _ => Key::Unknown,
    }
}

fn windows_scan(c: char) -> Key {
    match c {
        'H' => Key::Up,
        'P' => Key::Down,
        'K' => Key::Left,
        'M' => Key::Right,
        'I' => Key::PgUp,
        'Q' => Key::PgDn,
        'G' => Key::Home,
        'O' => Key::End,
        'R' => Key::Insert,
        'S' => Key::Delete,
        ';' => Key::F(1),
        '<' => Key::F(2),
        '=' => Key::F(3),
        '>' => Key::F(4),
        '?' => Key::F(5),
        '@' => Key::F(6),
        'A' => Key::F(7),
        'B' => Key::F(8),
        'C' => Key::F(9),
        'D' => Key::F(10),
        _ => Key::Unknown,
    }
}

fn control(c: char) -> Option<Key> {
    match c {
        '\r' | '\n' => Some(Key::Enter),
        '\t' => Some(Key::Tab),
        '\x7f' | '\x08' => Some(Key::Backspace),
        '\x03' => Some(Key::CtrlC),
        '\x13' => Some(Key::CtrlS),
        '\x16' => Some(Key::CtrlV),
        '\x15' => Some(Key::CtrlU),
        _ => None,
    }
}

fn escape(chars: &[char], i: usize) -> Option<(Key, usize)> {
    if i + 1 >= chars.len() {
        return None;
    }
    match chars[i + 1] {
        '[' => {
            let mut j = i + 2;
            while j < chars.len() && !('@'..='~').contains(&chars[j]) {
                j += 1;
            }
            if j >= chars.len() {
                return None;
            }
            let body: String = chars[i + 2..j].iter().collect();
            let key = if chars[j] == '~' {
                csi_tilde(body.split(';').next().unwrap_or(""))
            } else {
                csi_final(chars[j])
            };
            Some((key, j - i + 1))
        }
        'O' => {
            if i + 2 >= chars.len() {
                return None;
            }
            Some((ss3(chars[i + 2]), 3))
        }
        _ => Some((Key::Esc, 1)),
    }
}

pub fn split_keys(text: &str) -> (Vec<Key>, String) {
    let chars: Vec<char> = text.chars().collect();
    let mut keys = Vec::new();
    let mut i = 0;
    while i < chars.len() {
        let ch = chars[i];
        if ch == ESC {
            match escape(&chars, i) {
                None => return (keys, chars[i..].iter().collect()),
                Some((key, used)) => {
                    keys.push(key);
                    i += used;
                    continue;
                }
            }
        }
        if ch == '\x00' || ch == '\u{e0}' {
            if i + 1 >= chars.len() {
                return (keys, chars[i..].iter().collect());
            }
            keys.push(windows_scan(chars[i + 1]));
            i += 2;
            continue;
        }
        if let Some(known) = control(ch) {
            keys.push(known);
        } else if is_printable(ch) {
            keys.push(Key::Char(ch));
        } else {
            keys.push(Key::Unknown);
        }
        i += 1;
    }
    (keys, String::new())
}

pub const EVENT_KEY: u16 = 0x0001;
pub const EVENT_MOUSE: u16 = 0x0002;
pub const EVENT_RESIZE: u16 = 0x0004;
pub const EVENT_MENU: u16 = 0x0008;
pub const EVENT_FOCUS: u16 = 0x0010;

pub const RIGHT_ALT: u32 = 0x0001;
pub const LEFT_ALT: u32 = 0x0002;
pub const RIGHT_CTRL: u32 = 0x0004;
pub const LEFT_CTRL: u32 = 0x0008;
pub const SHIFT: u32 = 0x0010;

const ALT_MASK: u32 = RIGHT_ALT | LEFT_ALT;
const CTRL_MASK: u32 = RIGHT_CTRL | LEFT_CTRL;

pub const VK_BACK: u16 = 0x08;
pub const VK_TAB: u16 = 0x09;
pub const VK_RETURN: u16 = 0x0D;
pub const VK_ESCAPE: u16 = 0x1B;
pub const VK_PRIOR: u16 = 0x21;
pub const VK_NEXT: u16 = 0x22;
pub const VK_END: u16 = 0x23;
pub const VK_HOME: u16 = 0x24;
pub const VK_LEFT: u16 = 0x25;
pub const VK_UP: u16 = 0x26;
pub const VK_RIGHT: u16 = 0x27;
pub const VK_DOWN: u16 = 0x28;
pub const VK_INSERT: u16 = 0x2D;
pub const VK_DELETE: u16 = 0x2E;
pub const VK_F1: u16 = 0x70;
pub const VK_F12: u16 = 0x7B;

fn vk_nav(vk: u16) -> Option<Key> {
    Some(match vk {
        VK_UP => Key::Up,
        VK_DOWN => Key::Down,
        VK_LEFT => Key::Left,
        VK_RIGHT => Key::Right,
        VK_PRIOR => Key::PgUp,
        VK_NEXT => Key::PgDn,
        VK_HOME => Key::Home,
        VK_END => Key::End,
        VK_INSERT => Key::Insert,
        VK_DELETE => Key::Delete,
        VK_RETURN => Key::Enter,
        VK_TAB => Key::Tab,
        VK_BACK => Key::Backspace,
        _ => return None,
    })
}

fn vk_ctrl(vk: u16) -> Option<Key> {
    match vk as u8 {
        b'C' => Some(Key::CtrlC),
        b'S' => Some(Key::CtrlS),
        b'V' => Some(Key::CtrlV),
        b'U' => Some(Key::CtrlU),
        _ => None,
    }
}

pub fn map_windows_key(vk: u16, control: u32, ch: Option<char>) -> Option<Key> {
    let printable = ch.filter(|c| is_printable(*c));
    if control & ALT_MASK != 0 {
        return printable.map(Key::Char);
    }
    if vk == VK_ESCAPE {
        return Some(Key::Esc);
    }
    if let Some(key) = vk_nav(vk) {
        return Some(key);
    }
    if (VK_F1..=VK_F12).contains(&vk) {
        return Some(Key::F((vk - VK_F1 + 1) as u8));
    }
    if control & CTRL_MASK != 0 {
        return if vk < 256 { vk_ctrl(vk) } else { None };
    }
    printable.map(Key::Char)
}

pub fn record_key(event_type: u16, key_down: bool, vk: u16, control: u32, ch: Option<char>) -> Option<Key> {
    if event_type != EVENT_KEY || !key_down {
        return None;
    }
    map_windows_key(vk, control, ch)
}

fn debug_key(logs_dir: &Option<PathBuf>, text: &str) {
    if std::env::var_os("PPP_DEBUG_KEYS").is_none() {
        return;
    }
    let Some(dir) = logs_dir else { return };
    let _ = std::fs::create_dir_all(dir);
    if let Ok(mut handle) = std::fs::OpenOptions::new().create(true).append(true).open(dir.join("keys.log")) {
        let _ = writeln!(handle, "{}", text);
    }
}

#[cfg(windows)]
mod native {
    use super::*;
    use windows_sys::Win32::Foundation::{HANDLE, INVALID_HANDLE_VALUE, WAIT_OBJECT_0};
    use windows_sys::Win32::System::Console::{
        GetConsoleMode, GetConsoleScreenBufferInfo, GetStdHandle, ReadConsoleInputW, SetConsoleMode,
        CONSOLE_SCREEN_BUFFER_INFO, ENABLE_ECHO_INPUT, ENABLE_EXTENDED_FLAGS, ENABLE_LINE_INPUT,
        ENABLE_MOUSE_INPUT, ENABLE_PROCESSED_INPUT, ENABLE_QUICK_EDIT_MODE, ENABLE_WINDOW_INPUT,
        INPUT_RECORD, STD_INPUT_HANDLE, STD_OUTPUT_HANDLE,
    };
    use windows_sys::Win32::System::Threading::WaitForSingleObject;

    const ENABLE_VIRTUAL_TERMINAL_PROCESSING: u32 = 0x0004;

    pub struct Input {
        handle: HANDLE,
        saved_mode: u32,
        high_surrogate: Option<u16>,
    }

    pub fn enable_vt() {
        unsafe {
            let handle = GetStdHandle(STD_OUTPUT_HANDLE);
            let mut mode = 0u32;
            if handle != INVALID_HANDLE_VALUE && !handle.is_null() && GetConsoleMode(handle, &mut mode) != 0 {
                SetConsoleMode(handle, mode | ENABLE_VIRTUAL_TERMINAL_PROCESSING);
            }
        }
    }

    pub fn console_size() -> Option<(usize, usize)> {
        unsafe {
            let handle = GetStdHandle(STD_OUTPUT_HANDLE);
            if handle == INVALID_HANDLE_VALUE || handle.is_null() {
                return None;
            }
            let mut info: CONSOLE_SCREEN_BUFFER_INFO = std::mem::zeroed();
            if GetConsoleScreenBufferInfo(handle, &mut info) == 0 {
                return None;
            }
            let width = (info.srWindow.Right - info.srWindow.Left + 1) as i32;
            let height = (info.srWindow.Bottom - info.srWindow.Top + 1) as i32;
            if width > 0 && height > 0 { Some((width as usize, height as usize)) } else { None }
        }
    }

    impl Input {
        pub fn open() -> io::Result<Input> {
            unsafe {
                let handle = GetStdHandle(STD_INPUT_HANDLE);
                let mut mode = 0u32;
                if handle == INVALID_HANDLE_VALUE || handle.is_null() || GetConsoleMode(handle, &mut mode) == 0 {
                    return Err(io::Error::new(io::ErrorKind::Other, "standard input is not a console"));
                }
                let mut wanted = mode | ENABLE_EXTENDED_FLAGS | ENABLE_WINDOW_INPUT;
                wanted &= !ENABLE_MOUSE_INPUT & !ENABLE_QUICK_EDIT_MODE;
                wanted &= !ENABLE_PROCESSED_INPUT & !ENABLE_LINE_INPUT & !ENABLE_ECHO_INPUT;
                if SetConsoleMode(handle, wanted) == 0 {
                    return Err(io::Error::last_os_error());
                }
                Ok(Input { handle, saved_mode: mode, high_surrogate: None })
            }
        }

        pub fn restore(&mut self) {
            unsafe {
                SetConsoleMode(self.handle, self.saved_mode);
            }
        }

        fn decode_unit(&mut self, unit: u16) -> Option<char> {
            if unit == 0 {
                return None;
            }
            if (0xD800..0xDC00).contains(&unit) {
                self.high_surrogate = Some(unit);
                return None;
            }
            if (0xDC00..0xE000).contains(&unit) {
                let high = self.high_surrogate.take()?;
                return char::decode_utf16([high, unit]).next().and_then(Result::ok);
            }
            self.high_surrogate = None;
            char::from_u32(unit as u32)
        }

        pub fn read(&mut self, timeout: Duration, pending: &mut VecDeque<Key>, logs: &Option<PathBuf>, resized: &mut bool) -> Option<Key> {
            let deadline = Instant::now() + timeout;
            loop {
                let remaining = deadline.saturating_duration_since(Instant::now());
                if remaining.is_zero() {
                    return None;
                }
                let wait_ms = remaining.as_millis().clamp(1, 200) as u32;
                if unsafe { WaitForSingleObject(self.handle, wait_ms) } != WAIT_OBJECT_0 {
                    continue;
                }
                let mut records: [INPUT_RECORD; 32] = unsafe { std::mem::zeroed() };
                let mut count = 0u32;
                if unsafe { ReadConsoleInputW(self.handle, records.as_mut_ptr(), 32, &mut count) } == 0 {
                    return None;
                }
                for record in records.iter().take(count as usize) {
                    if record.EventType == EVENT_RESIZE {
                        *resized = true;
                        continue;
                    }
                    if record.EventType == EVENT_KEY {
                        let key_event = unsafe { record.Event.KeyEvent };
                        let unit = unsafe { key_event.uChar.UnicodeChar };
                        let ch = self.decode_unit(unit);
                        let down = key_event.bKeyDown != 0;
                        let key = record_key(record.EventType, down, key_event.wVirtualKeyCode, key_event.dwControlKeyState, ch);
                        debug_key(
                            logs,
                            &format!(
                                "vk={:#04x} ctrl={:#06x} char={:?} down={} -> {:?}",
                                key_event.wVirtualKeyCode, key_event.dwControlKeyState, ch, down, key
                            ),
                        );
                        if let Some(key) = key {
                            let repeats = (key_event.wRepeatCount as usize).clamp(1, 8);
                            for _ in 0..repeats {
                                pending.push_back(key.clone());
                            }
                        }
                    } else {
                        debug_key(logs, &format!("event={:#06x} ignored", record.EventType));
                    }
                }
                if let Some(key) = pending.pop_front() {
                    return Some(key);
                }
            }
        }
    }
}

#[cfg(unix)]
mod native {
    use super::*;

    pub struct Input {
        saved: libc::termios,
        buf: Vec<u8>,
        text: String,
    }

    pub fn enable_vt() {}

    pub fn console_size() -> Option<(usize, usize)> {
        unsafe {
            let mut ws: libc::winsize = std::mem::zeroed();
            if libc::ioctl(1, libc::TIOCGWINSZ, &mut ws) == 0 && ws.ws_col > 0 && ws.ws_row > 0 {
                Some((ws.ws_col as usize, ws.ws_row as usize))
            } else {
                None
            }
        }
    }

    impl Input {
        pub fn open() -> io::Result<Input> {
            unsafe {
                let mut saved: libc::termios = std::mem::zeroed();
                if libc::tcgetattr(0, &mut saved) != 0 {
                    return Err(io::Error::last_os_error());
                }
                let mut raw = saved;
                libc::cfmakeraw(&mut raw);
                if libc::tcsetattr(0, libc::TCSANOW, &raw) != 0 {
                    return Err(io::Error::last_os_error());
                }
                Ok(Input { saved, buf: Vec::new(), text: String::new() })
            }
        }

        pub fn restore(&mut self) {
            unsafe {
                libc::tcsetattr(0, libc::TCSADRAIN, &self.saved);
            }
        }

        fn feed(&mut self, data: &[u8]) {
            self.buf.extend_from_slice(data);
            loop {
                match std::str::from_utf8(&self.buf) {
                    Ok(valid) => {
                        self.text.push_str(valid);
                        self.buf.clear();
                        return;
                    }
                    Err(e) => {
                        let good = e.valid_up_to();
                        self.text.push_str(std::str::from_utf8(&self.buf[..good]).unwrap_or(""));
                        match e.error_len() {
                            Some(bad) => {
                                self.text.push('\u{fffd}');
                                self.buf.drain(..good + bad);
                            }
                            None => {
                                self.buf.drain(..good);
                                return;
                            }
                        }
                    }
                }
            }
        }

        pub fn read(&mut self, timeout: Duration, pending: &mut VecDeque<Key>, _logs: &Option<PathBuf>, _resized: &mut bool) -> Option<Key> {
            let deadline = Instant::now() + timeout;
            loop {
                if let Some(key) = pending.pop_front() {
                    return Some(key);
                }
                if !self.text.is_empty() {
                    let (keys, rest) = split_keys(&self.text);
                    if !keys.is_empty() {
                        self.text = rest;
                        pending.extend(keys);
                        continue;
                    }
                    if !rest.starts_with(|c| c == ESC || c == '\x00' || c == '\u{e0}') {
                        self.text.clear();
                        return Some(Key::Unknown);
                    }
                }
                let remaining = deadline.saturating_duration_since(Instant::now());
                if remaining.is_zero() {
                    let leftover = std::mem::take(&mut self.text);
                    if leftover == "\x1b" {
                        return Some(Key::Esc);
                    }
                    return None;
                }
                let mut fds = libc::pollfd { fd: 0, events: libc::POLLIN, revents: 0 };
                let ready = unsafe { libc::poll(&mut fds, 1, remaining.as_millis().min(i32::MAX as u128) as i32) };
                if ready > 0 {
                    let mut chunk = [0u8; 2048];
                    let n = unsafe { libc::read(0, chunk.as_mut_ptr() as *mut libc::c_void, chunk.len()) };
                    if n <= 0 {
                        return Some(Key::Esc);
                    }
                    self.feed(&chunk[..n as usize]);
                }
            }
        }
    }
}

pub use native::enable_vt as enable_output_vt;

pub struct Terminal {
    width: usize,
    height: usize,
    input: Option<native::Input>,
    pending: VecDeque<Key>,
    closed: bool,
    logs_dir: Option<PathBuf>,
    resized: bool,
}

impl Terminal {
    pub fn new(logs_dir: Option<PathBuf>) -> Terminal {
        Terminal { width: 100, height: 30, input: None, pending: VecDeque::new(), closed: true, logs_dir, resized: false }
    }
}

impl Term for Terminal {
    fn open(&mut self) -> io::Result<()> {
        native::enable_vt();
        self.input = Some(native::Input::open()?);
        self.closed = false;
        self.update_size();
        self.write("\x1b[?1049h\x1b[?25l\x1b[2J\x1b[H");
        Ok(())
    }

    fn close(&mut self) {
        if self.closed {
            return;
        }
        self.closed = true;
        self.write("\x1b[?25h\x1b[0m\x1b[?1049l");
        if let Some(input) = self.input.as_mut() {
            input.restore();
        }
        self.input = None;
    }

    fn is_tty(&self) -> bool {
        io::stdout().is_terminal() && io::stdin().is_terminal()
    }

    fn update_size(&mut self) {
        let env = |name: &str| std::env::var(name).ok().and_then(|v| v.trim().parse::<usize>().ok()).filter(|v| *v > 0);
        let (os_w, os_h) = native::console_size().unwrap_or((100, 30));
        self.width = env("COLUMNS").unwrap_or(os_w).max(20);
        self.height = env("LINES").unwrap_or(os_h).max(8);
    }

    fn size(&self) -> (usize, usize) {
        (self.width, self.height)
    }

    fn draw(&mut self, text: &str) {
        let mut frame = String::with_capacity(text.len() + 3);
        frame.push_str("\x1b[H");
        frame.push_str(text);
        self.write(&frame);
    }

    fn write(&mut self, text: &str) {
        let mut out = io::stdout().lock();
        let _ = out.write_all(text.as_bytes());
        let _ = out.flush();
    }

    fn read_key(&mut self, timeout: Duration) -> Option<Key> {
        if let Some(key) = self.pending.pop_front() {
            return Some(key);
        }
        let input = self.input.as_mut()?;
        let key = input.read(timeout, &mut self.pending, &self.logs_dir, &mut self.resized);
        if self.resized {
            self.resized = false;
            self.update_size();
        }
        key
    }
}

impl Drop for Terminal {
    fn drop(&mut self) {
        self.close();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn names(text: &str) -> Vec<Key> {
        split_keys(text).0
    }

    fn chars(text: &str) -> String {
        split_keys(text).0.into_iter().filter_map(|k| if let Key::Char(c) = k { Some(c) } else { None }).collect()
    }

    #[test]
    fn plain_text_is_one_key_per_character() {
        assert_eq!(names("abc").len(), 3);
        assert_eq!(chars("abc"), "abc");
    }

    #[test]
    fn cyrillic_text_survives() {
        assert_eq!(chars("мфд"), "мфд");
    }

    #[test]
    fn ansi_arrows() {
        assert_eq!(names("\x1b[A\x1b[B\x1b[C\x1b[D"), vec![Key::Up, Key::Down, Key::Right, Key::Left]);
    }

    #[test]
    fn windows_arrows() {
        assert_eq!(names("\u{e0}H\u{e0}P\u{e0}K\u{e0}M"), vec![Key::Up, Key::Down, Key::Left, Key::Right]);
    }

    #[test]
    fn paging_and_home_end() {
        assert_eq!(
            names("\x1b[5~\x1b[6~\x1b[H\x1b[F\x1b[1~\x1b[4~"),
            vec![Key::PgUp, Key::PgDn, Key::Home, Key::End, Key::Home, Key::End]
        );
        assert_eq!(names("\u{e0}I\u{e0}Q\u{e0}G\u{e0}O"), vec![Key::PgUp, Key::PgDn, Key::Home, Key::End]);
    }

    #[test]
    fn function_keys() {
        assert_eq!(names("\x1bOQ\x1b[12~\x00<"), vec![Key::F(2), Key::F(2), Key::F(2)]);
    }

    #[test]
    fn extended_function_keys() {
        assert_eq!(names("\x1b[15~\x1b[24~\x1b[21~"), vec![Key::F(5), Key::F(12), Key::F(10)]);
    }

    #[test]
    fn control_keys() {
        assert_eq!(
            names("\x03\x13\x16\r\n\t\x7f"),
            vec![Key::CtrlC, Key::CtrlS, Key::CtrlV, Key::Enter, Key::Enter, Key::Tab, Key::Backspace]
        );
    }

    #[test]
    fn incomplete_escape_is_kept_as_rest() {
        let (keys, rest) = split_keys("\x1b[");
        assert!(keys.is_empty());
        assert_eq!(rest, "\x1b[");
    }

    #[test]
    fn escape_then_letter() {
        assert_eq!(names("\x1bq"), vec![Key::Esc, Key::Char('q')]);
    }

    #[test]
    fn unknown_sequence() {
        assert_eq!(names("\x1b[99~"), vec![Key::Unknown]);
    }

    #[test]
    fn paste_of_multiline_text() {
        let (keys, rest) = split_keys("1.2.3.4:8080\r\n5.6.7.8:1080");
        assert_eq!(rest, "");
        assert_eq!(keys[0], Key::Char('1'));
        assert!(keys.contains(&Key::Enter));
    }

    #[test]
    fn escape_maps_to_esc() {
        assert_eq!(map_windows_key(VK_ESCAPE, 0, Some('\x1b')), Some(Key::Esc));
    }

    #[test]
    fn alt_shift_does_not_close_dialogs() {
        assert_eq!(map_windows_key(VK_ESCAPE, LEFT_ALT | SHIFT, None), None);
        assert_eq!(map_windows_key(VK_ESCAPE, RIGHT_ALT, Some('\x1b')), None);
    }

    #[test]
    fn altgr_still_types_characters() {
        assert_eq!(map_windows_key(b'Q' as u16, RIGHT_ALT | LEFT_CTRL, Some('@')), Some(Key::Char('@')));
    }

    #[test]
    fn windows_arrows_and_paging() {
        let keys: Vec<_> = [0x26, 0x28, 0x25, 0x27, 0x21, 0x22].iter().map(|vk| map_windows_key(*vk, 0, None)).collect();
        assert_eq!(
            keys,
            vec![Some(Key::Up), Some(Key::Down), Some(Key::Left), Some(Key::Right), Some(Key::PgUp), Some(Key::PgDn)]
        );
        let keys: Vec<_> = [0x24, 0x23, 0x2D, 0x2E].iter().map(|vk| map_windows_key(*vk, 0, None)).collect();
        assert_eq!(keys, vec![Some(Key::Home), Some(Key::End), Some(Key::Insert), Some(Key::Delete)]);
    }

    #[test]
    fn windows_function_keys_up_to_f12() {
        let keys: Vec<_> = [0x70, 0x71, 0x74, 0x7B].iter().map(|vk| map_windows_key(*vk, 0, None)).collect();
        assert_eq!(keys, vec![Some(Key::F(1)), Some(Key::F(2)), Some(Key::F(5)), Some(Key::F(12))]);
    }

    #[test]
    fn control_combinations() {
        assert_eq!(map_windows_key(b'C' as u16, LEFT_CTRL, Some('\x03')), Some(Key::CtrlC));
        assert_eq!(map_windows_key(b'S' as u16, LEFT_CTRL, Some('\x13')), Some(Key::CtrlS));
        assert_eq!(map_windows_key(b'X' as u16, LEFT_CTRL, Some('\x18')), None);
    }

    #[test]
    fn cyrillic_character_passes_through() {
        assert_eq!(map_windows_key(b'M' as u16, 0, Some('м')), Some(Key::Char('м')));
    }

    #[test]
    fn modifier_only_events_are_ignored() {
        assert_eq!(map_windows_key(0x10, SHIFT, None), None);
        assert_eq!(map_windows_key(0x11, LEFT_CTRL, None), None);
        assert_eq!(map_windows_key(0x12, LEFT_ALT, None), None);
    }

    #[test]
    fn enter_tab_backspace() {
        let keys: Vec<_> = [(0x0D, '\r'), (0x09, '\t'), (0x08, '\x08')].iter().map(|(vk, c)| map_windows_key(*vk, 0, Some(*c))).collect();
        assert_eq!(keys, vec![Some(Key::Enter), Some(Key::Tab), Some(Key::Backspace)]);
    }

    #[test]
    fn key_down_is_mapped_and_key_up_ignored() {
        assert_eq!(record_key(EVENT_KEY, true, 0x26, 0, None), Some(Key::Up));
        assert_eq!(record_key(EVENT_KEY, false, 0x26, 0, None), None);
    }

    #[test]
    fn mouse_and_focus_events_are_ignored() {
        for event in [EVENT_MOUSE, EVENT_FOCUS, EVENT_RESIZE, EVENT_MENU] {
            assert_eq!(record_key(event, true, 0x26, 0, Some('x')), None);
        }
    }
}
