use std::time::Duration;

use crate::backend;
use crate::paths::Paths;
use crate::processes;
use crate::store::Store;

use super::model::{download, is_url, Screen};
use super::term::{Key, Term};
use super::view::{
    char_len, render, AttachModal, ConfirmModal, InputKind, InputModal, Modal, PasteModal, ProcessView, ProxyView,
    TestView, ViewState,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Action {
    Paste,
    Attach,
    Launch,
    Test,
    Next,
    Prev,
    Power,
    Rotate,
    Filter,
    Delete,
    Clean,
    Help,
    Search,
    Quit,
}

fn hotkey_action(c: char) -> Option<Action> {
    Some(match c {
        'v' | 'м' => Action::Paste,
        'a' | 'ф' => Action::Attach,
        'l' | 'д' => Action::Launch,
        't' | 'е' => Action::Test,
        'n' | 'т' => Action::Next,
        'p' | 'з' => Action::Prev,
        'o' | 'щ' => Action::Power,
        'r' | 'к' => Action::Rotate,
        'f' | 'а' => Action::Filter,
        'd' | 'в' => Action::Delete,
        'c' | 'с' => Action::Clean,
        '?' => Action::Help,
        '/' => Action::Search,
        'q' | 'й' => Action::Quit,
        _ => return None,
    })
}

fn split_at_char(text: &str, index: usize) -> (&str, &str) {
    let byte = text.char_indices().nth(index).map(|(i, _)| i).unwrap_or(text.len());
    text.split_at(byte)
}

fn remove_char(text: &str, index: usize) -> String {
    let (head, tail) = split_at_char(text, index);
    let mut tail_chars = tail.chars();
    tail_chars.next();
    format!("{}{}", head, tail_chars.as_str())
}

fn insert_char(text: &str, index: usize, ch: char) -> String {
    let (head, tail) = split_at_char(text, index);
    format!("{}{}{}", head, ch, tail)
}

fn split_whitespace_n(line: &str, n: usize) -> Vec<&str> {
    let mut parts = Vec::new();
    let mut rest = line.trim_start();
    while parts.len() + 1 < n {
        match rest.find(char::is_whitespace) {
            Some(i) => {
                parts.push(&rest[..i]);
                rest = rest[i..].trim_start();
            }
            None => break,
        }
    }
    if !rest.is_empty() {
        parts.push(rest.trim_end());
    }
    parts
}

fn last_two(lines: &[String]) -> String {
    let start = lines.len().saturating_sub(2);
    lines[start..].join("; ")
}

pub struct App<T: Term> {
    pub term: T,
    pub screen: Screen,
    pub modal: Option<Modal>,
    pub color: bool,
    pub page: usize,
    paths: Paths,
    store: Store,
    pending_delete: Option<usize>,
    size: Option<(usize, usize)>,
}

impl<T: Term> App<T> {
    pub fn new(term: T, screen: Screen) -> App<T> {
        let paths = screen.paths.clone();
        App {
            term,
            store: Store::new(paths.clone()),
            paths,
            screen,
            modal: None,
            color: std::env::var_os("NO_COLOR").is_none(),
            page: 10,
            pending_delete: None,
            size: None,
        }
    }

    pub fn run(&mut self) -> i32 {
        if !self.term.is_tty() {
            println!("the interactive console needs a terminal; use one-shot commands instead");
            return 1;
        }
        self.screen.reload(true);
        if let Err(e) = self.term.open() {
            println!("cannot start the console: {}", e);
            return 1;
        }
        loop {
            self.tick();
            self.draw();
            let Some(key) = self.term.read_key(Duration::from_millis(300)) else {
                continue;
            };
            if !self.handle(key) {
                break;
            }
        }
        self.term.close();
        0
    }

    fn tick(&mut self) {
        self.term.update_size();
        let (_, height) = self.term.size();
        self.page = height.saturating_sub(12).max(3);
        self.screen.reload(false);
        self.screen.poll_test();
        self.screen.clamp(self.page);
    }

    fn draw(&mut self) {
        let size = self.term.size();
        if self.size != Some(size) {
            self.size = Some(size);
            self.term.write("\x1b[2J");
        }
        let frame = render(&self.state(), size.0, size.1);
        let color = self.color && self.term.is_tty();
        let text = frame.render(color);
        self.term.draw(&text);
    }

    fn state(&self) -> ViewState {
        let screen = &self.screen;
        let rows = screen
            .visible()
            .into_iter()
            .map(|row| ProxyView {
                index: row.index,
                display: row.proxy.display(),
                health: screen.health.get(&row.proxy.label()).cloned(),
            })
            .collect();
        let mut states: Vec<ProcessView> = screen
            .states
            .iter()
            .map(|st| ProcessView { pid: st.pid, exe: st.exe.clone(), alive: st.alive(), current: st.current, age: st.age })
            .collect();
        states.sort_by_key(|st| (!st.alive, st.pid));
        ViewState {
            enabled: screen.enabled(),
            current_index: screen.current,
            current_display: screen.current_display(),
            rotate_sec: screen.rotate_sec(),
            proxy_total: screen.rows.len(),
            rows,
            selected: screen.selected,
            scroll: screen.scroll,
            filter_name: screen.filter.to_string(),
            query: screen.query.clone(),
            states,
            direct_ip: screen.direct.clone(),
            test: screen.test.as_ref().map(|t| TestView { done: t.done, total: t.total }),
            message: screen.message.clone(),
            modal: self.modal.clone(),
        }
    }

    pub fn handle(&mut self, key: Key) -> bool {
        if key == Key::CtrlC {
            if self.modal.is_some() {
                self.modal = None;
                self.pending_delete = None;
                return true;
            }
            return false;
        }
        if self.modal.is_some() {
            self.modal_key(key);
            return true;
        }
        match key {
            Key::Char(c) => return self.hotkey(c),
            Key::Up => self.screen.move_by(-1),
            Key::Down => self.screen.move_by(1),
            Key::PgUp => self.screen.move_by(-(self.page as i64)),
            Key::PgDn => self.screen.move_by(self.page as i64),
            Key::Home => self.screen.jump(true),
            Key::End => self.screen.jump(false),
            Key::Enter => {
                if !self.screen.visible().is_empty() {
                    let message = self.screen.use_selected();
                    self.say(message);
                }
            }
            _ => {}
        }
        true
    }

    fn hotkey(&mut self, c: char) -> bool {
        let Some(action) = hotkey_action(c) else {
            if let Some(digit) = c.to_digit(10) {
                self.use_index(digit as usize);
            }
            return true;
        };
        match action {
            Action::Quit => return false,
            Action::Paste => self.modal = Some(Modal::Paste(PasteModal::default())),
            Action::Attach => self.open_attach(),
            Action::Launch => {
                self.modal = Some(Modal::Input(InputModal {
                    kind: InputKind::Launch,
                    title: "l: launch an injected program".to_string(),
                    value: String::new(),
                    hint: "full path, e.g. \"C:\\Program Files\\App\\app.exe\"".to_string(),
                }))
            }
            Action::Test => {
                let message = self.screen.start_test();
                self.say(message);
            }
            Action::Next => {
                let message = self.screen.step(1);
                self.say(message);
            }
            Action::Prev => {
                let message = self.screen.step(-1);
                self.say(message);
            }
            Action::Power => {
                let message = self.screen.toggle_power();
                self.say(message);
            }
            Action::Rotate => {
                self.modal = Some(Modal::Input(InputModal {
                    kind: InputKind::Rotate,
                    title: "r: automatic rotation (0 = off)".to_string(),
                    value: self.screen.rotate_sec(),
                    hint: "seconds between rotations".to_string(),
                }))
            }
            Action::Filter => self.screen.cycle_filter(),
            Action::Delete => self.ask_delete(),
            Action::Clean => {
                let message = self.screen.clean();
                self.say(message);
            }
            Action::Help => self.modal = Some(Modal::Help),
            Action::Search => {
                self.modal = Some(Modal::Input(InputModal {
                    kind: InputKind::Search,
                    title: "/: search proxies".to_string(),
                    value: self.screen.query.clone(),
                    hint: "host, port or scheme   empty = show all".to_string(),
                }))
            }
        }
        true
    }

    fn use_index(&mut self, digit: usize) {
        let count = self.screen.visible().len();
        if count == 0 {
            return;
        }
        self.screen.selected = digit.min(count - 1);
        let message = self.screen.use_selected();
        self.say(message);
    }

    fn say(&mut self, message: String) {
        if !message.is_empty() {
            self.screen.message = message;
        }
    }

    fn modal_key(&mut self, key: Key) {
        let Some(modal) = self.modal.take() else {
            return;
        };
        self.modal = match modal {
            Modal::Paste(m) => self.paste_key(m, key),
            Modal::Attach(m) => self.attach_key(m, key),
            Modal::Input(m) => self.input_key(m, key),
            Modal::Confirm(m) => self.confirm_key(m, key),
            Modal::Help => match key {
                Key::Esc | Key::Enter => None,
                Key::Char(c) if "?qй".contains(c) => None,
                _ => Some(Modal::Help),
            },
        };
    }

    pub fn paste_key(&mut self, mut modal: PasteModal, key: Key) -> Option<Modal> {
        match key {
            Key::Esc => return None,
            Key::CtrlS | Key::F(2) => return self.apply_paste(modal),
            _ => {}
        }
        let line = modal.lines[modal.row].clone();
        let line_len = char_len(&line);
        match key {
            Key::Enter => {
                let (head, tail) = split_at_char(&line, modal.col);
                let (head, tail) = (head.to_string(), tail.to_string());
                modal.lines.splice(modal.row..modal.row + 1, [head, tail]);
                modal.row += 1;
                modal.col = 0;
            }
            Key::Backspace => {
                if modal.col > 0 {
                    modal.lines[modal.row] = remove_char(&line, modal.col - 1);
                    modal.col -= 1;
                } else if modal.row > 0 {
                    let previous = modal.lines[modal.row - 1].clone();
                    let merged = format!("{}{}", previous, line);
                    modal.lines.splice(modal.row - 1..modal.row + 1, [merged]);
                    modal.row -= 1;
                    modal.col = char_len(&previous);
                }
            }
            Key::Delete => {
                if modal.col < line_len {
                    modal.lines[modal.row] = remove_char(&line, modal.col);
                } else if modal.row + 1 < modal.lines.len() {
                    let merged = format!("{}{}", line, modal.lines[modal.row + 1]);
                    modal.lines.splice(modal.row..modal.row + 2, [merged]);
                }
            }
            Key::Left => {
                if modal.col > 0 {
                    modal.col -= 1;
                } else if modal.row > 0 {
                    modal.row -= 1;
                    modal.col = char_len(&modal.lines[modal.row]);
                }
            }
            Key::Right => {
                if modal.col < line_len {
                    modal.col += 1;
                } else if modal.row + 1 < modal.lines.len() {
                    modal.row += 1;
                    modal.col = 0;
                }
            }
            Key::Up => {
                modal.row = modal.row.saturating_sub(1);
                modal.col = modal.col.min(char_len(&modal.lines[modal.row]));
            }
            Key::Down => {
                modal.row = (modal.row + 1).min(modal.lines.len() - 1);
                modal.col = modal.col.min(char_len(&modal.lines[modal.row]));
            }
            Key::Home => modal.col = 0,
            Key::End => modal.col = line_len,
            Key::Char(c) => {
                modal.lines[modal.row] = insert_char(&line, modal.col, c);
                modal.col += 1;
            }
            _ => {}
        }
        Some(Modal::Paste(modal))
    }

    fn apply_paste(&mut self, modal: PasteModal) -> Option<Modal> {
        let joined = modal.lines.join("\n");
        let text = joined.trim();
        if text.is_empty() {
            return None;
        }
        let mut urls = Vec::new();
        let mut specs = Vec::new();
        for line in text.lines() {
            let item = line.trim();
            if item.is_empty() {
                continue;
            }
            if is_url(item) {
                urls.push(item.to_string());
            } else {
                specs.push(item.to_string());
            }
        }
        if !specs.is_empty() {
            let (added, skipped, total) = self.store.import_text(&specs.join("\n"));
            self.screen.reload(true);
            self.say(format!("added {}, skipped {}, total {}", added, skipped, total));
        }
        for url in urls {
            self.fetch(&url);
        }
        None
    }

    fn fetch(&mut self, url: &str) {
        let shown: String = url.chars().take(48).collect();
        self.say(format!("downloading {}...", shown));
        let store = self.store.clone();
        let notice = std::sync::Arc::clone(&self.screen.notice);
        let url = url.to_string();
        std::thread::spawn(move || {
            let message = match download(&url, 20.0) {
                Err(e) => format!("download failed: {}", e),
                Ok(raw) => {
                    let (added, skipped, total) = store.import_text(&raw);
                    format!("list: added {}, skipped {}, total {}", added, skipped, total)
                }
            };
            *notice.lock().unwrap_or_else(|e| e.into_inner()) = message;
        });
    }

    fn open_attach(&mut self) {
        let (entries, note) = self.process_entries();
        self.modal = Some(Modal::Attach(AttachModal { entries, selected: 0, manual: String::new(), note }));
    }

    fn process_entries(&self) -> (Vec<(u32, String)>, String) {
        let lines = match backend::list_processes(&self.paths) {
            Ok((_rc, lines)) => lines,
            Err(e) => return (Vec::new(), e.to_string()),
        };
        let mut entries = Vec::new();
        for line in lines {
            let parts = split_whitespace_n(&line, 3);
            if parts.len() < 3 || !parts[0].bytes().all(|b| b.is_ascii_digit()) {
                continue;
            }
            if let Ok(pid) = parts[0].parse::<u32>() {
                entries.push((pid, parts[2].trim().to_string()));
            }
        }
        entries.truncate(200);
        (entries, String::new())
    }

    pub fn attach_key(&mut self, mut modal: AttachModal, key: Key) -> Option<Modal> {
        match key {
            Key::Esc => {
                if !modal.manual.is_empty() {
                    modal.manual.clear();
                    modal.note.clear();
                    return Some(Modal::Attach(modal));
                }
                return None;
            }
            Key::Up => modal.selected = modal.selected.saturating_sub(1),
            Key::Down => modal.selected = (modal.selected + 1).min(modal.entries.len().saturating_sub(1)),
            Key::Backspace => {
                modal.manual.pop();
            }
            Key::Enter => return self.attach(modal),
            Key::Char(c) => {
                if (c == 'x' || c == 'ч') && !modal.entries.is_empty() && modal.manual.is_empty() {
                    self.kill(&mut modal);
                } else {
                    modal.manual.push(c);
                }
            }
            _ => {}
        }
        Some(Modal::Attach(modal))
    }

    fn attach(&mut self, mut modal: AttachModal) -> Option<Modal> {
        let mut target = modal.manual.trim().to_string();
        if target.is_empty() && !modal.entries.is_empty() {
            let pick = modal.selected.min(modal.entries.len() - 1);
            target = modal.entries[pick].0.to_string();
        }
        if target.is_empty() {
            modal.note = "type a pid or an exe name first".to_string();
            return Some(Modal::Attach(modal));
        }
        let result = match target.parse::<u32>() {
            Ok(pid) if target.bytes().all(|b| b.is_ascii_digit()) => backend::inject_pid(&self.paths, pid),
            _ => backend::inject_name(&self.paths, &target),
        };
        match result {
            Err(e) => self.say(format!("attach: {}", e)),
            Ok((_rc, lines)) => {
                let message = if lines.is_empty() { format!("attached to {}", target) } else { last_two(&lines) };
                self.say(message);
                self.screen.reload(true);
            }
        }
        None
    }

    fn kill(&mut self, modal: &mut AttachModal) {
        if modal.entries.is_empty() {
            return;
        }
        let pick = modal.selected.min(modal.entries.len() - 1);
        let (pid, label) = modal.entries[pick].clone();
        let ok = processes::kill_process(pid);
        processes::clean_dead(&self.paths);
        let outcome = if ok { "terminated" } else { "not running or access denied" };
        modal.note = format!("{}: {}", label, outcome);
        let (entries, _note) = self.process_entries();
        modal.selected = modal.selected.min(entries.len().saturating_sub(1));
        modal.entries = entries;
    }

    fn input_key(&mut self, mut modal: InputModal, key: Key) -> Option<Modal> {
        match key {
            Key::Esc => return None,
            Key::Enter => return self.submit(modal),
            Key::Backspace => {
                modal.value.pop();
            }
            Key::Char(c) => modal.value.push(c),
            _ => {}
        }
        Some(Modal::Input(modal))
    }

    fn submit(&mut self, mut modal: InputModal) -> Option<Modal> {
        let value = modal.value.trim().to_string();
        match modal.kind {
            InputKind::Rotate => {
                let text = if value.is_empty() { "0" } else { value.as_str() };
                match text.parse::<i64>() {
                    Ok(seconds) => {
                        let message = self.screen.set_rotate(seconds);
                        self.say(message);
                        None
                    }
                    Err(_) => {
                        modal.hint = "whole seconds, for example 120 (0 = off)".to_string();
                        Some(Modal::Input(modal))
                    }
                }
            }
            InputKind::Launch => {
                if value.is_empty() {
                    modal.hint = "a command is required".to_string();
                    return Some(Modal::Input(modal));
                }
                match backend::launch(&self.paths, &value) {
                    Err(e) => self.say(format!("launch: {}", e)),
                    Ok((_rc, lines)) => {
                        let message = if lines.is_empty() { format!("launched {}", value) } else { last_two(&lines) };
                        self.say(message);
                        self.screen.reload(true);
                    }
                }
                None
            }
            InputKind::Search => {
                self.screen.set_query(&value);
                let message = if value.is_empty() { "search cleared".to_string() } else { format!("search: \"{}\"", value) };
                self.say(message);
                None
            }
        }
    }

    fn ask_delete(&mut self) {
        let Some(row) = self.screen.selected_row() else {
            self.say("nothing to delete".to_string());
            return;
        };
        let (index, display) = (row.index, row.proxy.display());
        self.modal = Some(Modal::Confirm(ConfirmModal {
            title: "d: delete proxy".to_string(),
            body: vec![format!("#{}  {}", index, display), String::new(), "this removes the proxy from the database".to_string()],
        }));
        self.pending_delete = Some(index);
    }

    fn confirm_key(&mut self, modal: ConfirmModal, key: Key) -> Option<Modal> {
        let yes = key == Key::Enter || matches!(key, Key::Char(c) if "yднY".contains(c));
        let no = key == Key::Esc || matches!(key, Key::Char(c) if "nлN".contains(c));
        if !yes && !no {
            return Some(Modal::Confirm(modal));
        }
        let pending = self.pending_delete.take();
        if yes {
            if let Some(index) = pending {
                let removed = self.store.remove_indices(&[index as i64]);
                self.screen.reload(true);
                self.say(format!("removed {} proxy line(s)", removed));
            }
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tui::model::tests::FakeNet;
    use std::io;
    use std::sync::Arc;

    pub struct FakeTerminal {
        pub keys: Vec<Key>,
        pub drawn: Vec<String>,
        pub opened: bool,
        pub closed: bool,
    }

    impl Term for FakeTerminal {
        fn open(&mut self) -> io::Result<()> {
            self.opened = true;
            Ok(())
        }
        fn close(&mut self) {
            self.closed = true;
        }
        fn is_tty(&self) -> bool {
            true
        }
        fn update_size(&mut self) {}
        fn size(&self) -> (usize, usize) {
            (100, 30)
        }
        fn draw(&mut self, text: &str) {
            self.drawn.push(text.to_string());
        }
        fn write(&mut self, text: &str) {
            self.drawn.push(text.to_string());
        }
        fn read_key(&mut self, _timeout: Duration) -> Option<Key> {
            if self.keys.is_empty() { Some(Key::CtrlC) } else { Some(self.keys.remove(0)) }
        }
    }

    fn make_app(name: &str, keys: Vec<Key>) -> App<FakeTerminal> {
        let dir = std::env::temp_dir().join(format!("ppp-app-{}-{}", name, std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let paths = Paths::new(dir);
        std::fs::write(&paths.config, "enabled = 1\ncurrent = 0\nrotate_sec = 0\n").unwrap();
        Store::new(paths.clone()).add_specs(&["1.1.1.1:80".to_string(), "2.2.2.2:80".to_string(), "3.3.3.3:80".to_string()]);
        let screen = Screen::with_net(paths, Arc::new(FakeNet));
        let term = FakeTerminal { keys, drawn: Vec::new(), opened: false, closed: false };
        let mut app = App::new(term, screen);
        app.color = false;
        app
    }

    fn ch(c: char) -> Key {
        Key::Char(c)
    }

    fn drawn(app: &App<FakeTerminal>) -> String {
        app.term.drawn.join("\n")
    }

    #[test]
    fn quit_closes_the_terminal() {
        let mut app = make_app("quit", vec![ch('q')]);
        assert_eq!(app.run(), 0);
        assert!(app.term.opened && app.term.closed);
        assert!(!app.term.drawn.is_empty());
    }

    #[test]
    fn ctrl_c_quits_cleanly() {
        let mut app = make_app("ctrlc", vec![Key::CtrlC]);
        assert_eq!(app.run(), 0);
        assert!(app.term.closed);
        assert!(drawn(&app).contains("PerProcessProxy"));
    }

    #[test]
    fn paste_modal_renders_and_closes() {
        let mut app = make_app("paste", vec![ch('v'), ch('1'), ch('2'), Key::Esc, ch('q')]);
        app.run();
        assert!(drawn(&app).contains("paste proxies"));
        assert!(app.modal.is_none());
    }

    #[test]
    fn filter_key_cycles() {
        let mut app = make_app("filter", vec![ch('f'), ch('q')]);
        app.run();
        assert_eq!(app.screen.filter, "working");
    }

    #[test]
    fn navigation_moves_selection() {
        let mut app = make_app("nav", vec![Key::Down, Key::Down, ch('q')]);
        app.run();
        assert_eq!(app.screen.selected, 2);
    }

    #[test]
    fn search_is_reachable() {
        let mut app = make_app("search", vec![ch('/'), ch('2'), Key::Enter, ch('q')]);
        app.run();
        assert_eq!(app.screen.query, "2");
    }

    #[test]
    fn unknown_keys_do_not_crash() {
        let mut app = make_app("unknown", vec![Key::Unknown, Key::F(3), ch('z'), Key::Esc, ch('q')]);
        assert_eq!(app.run(), 0);
    }

    #[test]
    fn hotkeys_work_in_cyrillic_layout() {
        let mut app = make_app("cyr", vec![ch('а'), ch('й')]);
        app.run();
        assert_eq!(app.screen.filter, "working");
    }

    #[test]
    fn enter_uses_selected_proxy_and_writes_config() {
        let mut app = make_app("use", vec![Key::Down, Key::Enter, ch('q')]);
        app.run();
        assert_eq!(app.store.current_index(), 1);
    }

    #[test]
    fn power_toggle_writes_config() {
        let mut app = make_app("power", vec![ch('o'), ch('q')]);
        app.run();
        assert_eq!(app.store.read_config().get("enabled").map(String::as_str), Some("0"));
    }

    #[test]
    fn paste_adds_proxies_with_f2() {
        let keys = "9.9.9.9:8080".chars().map(Key::Char).collect::<Vec<_>>();
        let mut all = vec![ch('v')];
        all.extend(keys);
        all.push(Key::F(2));
        all.push(ch('q'));
        let mut app = make_app("pasteadd", all);
        app.run();
        assert_eq!(app.store.proxies().len(), 4);
    }

    #[test]
    fn delete_asks_before_removing() {
        let mut app = make_app("delete", vec![ch('d'), Key::Esc, ch('q')]);
        app.run();
        assert_eq!(app.store.proxies().len(), 3);
        let mut app = make_app("delete2", vec![ch('d'), ch('y'), ch('q')]);
        app.run();
        assert_eq!(app.store.proxies().len(), 2);
    }

    #[test]
    fn paste_editing_keys() {
        let mut app = make_app("edit", Vec::new());
        let mut modal = PasteModal::default();
        for c in "ab".chars() {
            modal = match app.paste_key(modal, Key::Char(c)) {
                Some(Modal::Paste(m)) => m,
                _ => unreachable!(),
            };
        }
        for key in [Key::Left, Key::Enter, Key::Backspace, Key::End] {
            modal = match app.paste_key(modal, key) {
                Some(Modal::Paste(m)) => m,
                _ => unreachable!(),
            };
        }
        assert_eq!(modal.lines, vec!["ab".to_string()]);
        assert_eq!((modal.row, modal.col), (0, 2));
    }

    #[test]
    fn first_escape_clears_manual_input_second_closes() {
        let mut app = make_app("attachesc", Vec::new());
        let modal = AttachModal { entries: vec![(1, "a.exe".to_string())], selected: 0, manual: "abc".to_string(), note: "x".to_string() };
        let Some(Modal::Attach(modal)) = app.attach_key(modal, Key::Esc) else { panic!("modal closed too early") };
        assert_eq!((modal.manual.as_str(), modal.note.as_str()), ("", ""));
        assert!(app.attach_key(modal, Key::Esc).is_none());
    }
}
