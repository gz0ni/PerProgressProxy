use std::io::Read;
use std::sync::mpsc::{channel, Receiver, TryRecvError};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use crate::netcheck::{self, Health, HealthEntry, TestFn, TestResult};
use crate::paths::Paths;
use crate::processes::{self, ProcessState};
use crate::specs::Proxy;
use crate::store::{Config, ProxyRow, Store};

pub const REFRESH: Duration = Duration::from_millis(500);
pub const FILTERS: [&str; 5] = ["all", "working", "untested", "failed", "same"];
pub const MAX_DOWNLOAD: usize = 5 * 1024 * 1024;
const USER_AGENT: &str = "ppp-tui/1";

pub fn is_url(text: &str) -> bool {
    let lower = text.trim().to_lowercase();
    lower.starts_with("http://") || lower.starts_with("https://")
}

pub fn download(url: &str, timeout: f64) -> Result<String, String> {
    let agent = ureq::AgentBuilder::new()
        .timeout(Duration::from_secs_f64(timeout))
        .user_agent(USER_AGENT)
        .try_proxy_from_env(true)
        .build();
    let response = agent.get(url).call().map_err(|e| e.to_string())?;
    let mut raw = Vec::new();
    response
        .into_reader()
        .take(MAX_DOWNLOAD as u64 + 1)
        .read_to_end(&mut raw)
        .map_err(|e| e.to_string())?;
    if raw.len() > MAX_DOWNLOAD {
        return Err("list is larger than 5 MB".to_string());
    }
    Ok(String::from_utf8_lossy(&raw).into_owned())
}

pub trait Net: Send + Sync {
    fn direct_ip(&self) -> Option<String>;
    fn tester(&self) -> TestFn;
    fn save_health(&self, results: &[TestResult]);
}

pub struct RealNet {
    pub paths: Paths,
}

impl Net for RealNet {
    fn direct_ip(&self) -> Option<String> {
        netcheck::direct_ip(6.0)
    }

    fn tester(&self) -> TestFn {
        netcheck::real_tester()
    }

    fn save_health(&self, results: &[TestResult]) {
        netcheck::save_health(&self.paths, results);
    }
}

enum Message {
    Direct(String),
    Result(TestResult),
    Done,
}

pub struct TestRun {
    pub total: usize,
    pub done: usize,
    pub good: usize,
    pub direct: String,
    pub finished: bool,
    rx: Receiver<Message>,
}

impl TestRun {
    pub fn start(rows: Vec<(usize, Proxy)>, concurrency: usize, timeout: f64, net: Arc<dyn Net>) -> TestRun {
        let (tx, rx) = channel();
        let total = rows.len();
        std::thread::spawn(move || {
            let direct = net.direct_ip().unwrap_or_default();
            let _ = tx.send(Message::Direct(direct));
            let (rtx, rrx) = channel();
            let handles = netcheck::spawn_pool(rows, concurrency, timeout, net.tester(), rtx);
            let mut collected = Vec::new();
            for result in rrx {
                collected.push(result.clone());
                let _ = tx.send(Message::Result(result));
            }
            for handle in handles {
                let _ = handle.join();
            }
            if !collected.is_empty() {
                net.save_health(&collected);
            }
            let _ = tx.send(Message::Done);
        });
        TestRun { total, done: 0, good: 0, direct: String::new(), finished: false, rx }
    }

    pub fn drain(&mut self) -> Vec<TestResult> {
        let mut out = Vec::new();
        loop {
            match self.rx.try_recv() {
                Ok(Message::Done) => self.finished = true,
                Ok(Message::Direct(ip)) => self.direct = ip,
                Ok(Message::Result(result)) => {
                    self.done += 1;
                    if result.ok && !(!self.direct.is_empty() && result.exit_ip == self.direct) {
                        self.good += 1;
                    }
                    out.push(result);
                }
                Err(TryRecvError::Empty) => return out,
                Err(TryRecvError::Disconnected) => {
                    self.finished = true;
                    return out;
                }
            }
        }
    }
}

pub struct Screen {
    pub paths: Paths,
    pub store: Store,
    pub net: Arc<dyn Net>,
    pub cfg: Config,
    pub rows: Vec<ProxyRow>,
    pub health: Health,
    pub states: Vec<ProcessState>,
    pub current: i64,
    pub direct: String,
    pub filter: &'static str,
    pub query: String,
    pub selected: usize,
    pub scroll: usize,
    pub message: String,
    pub notice: Arc<Mutex<String>>,
    pub test: Option<TestRun>,
    last: Option<Instant>,
}

impl Screen {
    pub fn new(paths: Paths) -> Screen {
        Screen::with_net(paths.clone(), Arc::new(RealNet { paths }))
    }

    pub fn with_net(paths: Paths, net: Arc<dyn Net>) -> Screen {
        Screen {
            store: Store::new(paths.clone()),
            paths,
            net,
            cfg: Config::new(),
            rows: Vec::new(),
            health: Health::new(),
            states: Vec::new(),
            current: 0,
            direct: String::new(),
            filter: "all",
            query: String::new(),
            selected: 0,
            scroll: 0,
            message: String::new(),
            notice: Arc::new(Mutex::new(String::new())),
            test: None,
            last: None,
        }
    }

    pub fn reload(&mut self, force: bool) {
        let now = Instant::now();
        if !force {
            if let Some(last) = self.last {
                if now.duration_since(last) < REFRESH {
                    return;
                }
            }
        }
        self.last = Some(now);
        self.cfg = self.store.read_config();
        self.rows = self.store.proxies();
        if self.test.is_none() {
            self.health = netcheck::load_health(&self.paths);
        }
        self.states = processes::read_states(&self.paths, false);
        self.current = self.cfg.get("current").and_then(|v| v.trim().parse().ok()).unwrap_or(0);
        let notice = std::mem::take(&mut *self.notice.lock().unwrap_or_else(|e| e.into_inner()));
        if !notice.is_empty() {
            self.message = notice;
        }
    }

    pub fn enabled(&self) -> bool {
        self.cfg.get("enabled").map(String::as_str).unwrap_or("1") == "1"
    }

    pub fn rotate_sec(&self) -> String {
        self.cfg.get("rotate_sec").cloned().unwrap_or_else(|| "0".to_string())
    }

    pub fn current_display(&self) -> String {
        if let Some(row) = self.rows.iter().find(|r| r.index as i64 == self.current) {
            return row.proxy.display();
        }
        usize::try_from(self.current)
            .ok()
            .and_then(|i| self.rows.get(i))
            .map(|r| r.proxy.display())
            .unwrap_or_default()
    }

    fn matches(&self, health: Option<&HealthEntry>) -> bool {
        let same = |h: &HealthEntry| !self.direct.is_empty() && h.ip == self.direct;
        match self.filter {
            "working" => health.map(|h| h.ok && !same(h)).unwrap_or(false),
            "untested" => health.is_none(),
            "failed" => health.map(|h| !h.ok).unwrap_or(false),
            "same" => health.map(|h| h.ok && same(h)).unwrap_or(false),
            _ => true,
        }
    }

    pub fn visible(&self) -> Vec<&ProxyRow> {
        let needle = self.query.to_lowercase();
        self.rows
            .iter()
            .filter(|row| {
                if !needle.is_empty() {
                    let haystack = format!("{} {}", row.proxy.display(), row.text).to_lowercase();
                    if !haystack.contains(&needle) {
                        return false;
                    }
                }
                self.matches(self.health.get(&row.proxy.label()))
            })
            .collect()
    }

    pub fn cycle_filter(&mut self) {
        let next = FILTERS.iter().position(|f| *f == self.filter).map(|i| (i + 1) % FILTERS.len()).unwrap_or(0);
        self.filter = FILTERS[next];
        self.selected = 0;
        self.scroll = 0;
    }

    pub fn set_query(&mut self, query: &str) {
        self.query = query.to_string();
        self.selected = 0;
        self.scroll = 0;
    }

    pub fn move_by(&mut self, delta: i64) {
        let count = self.visible().len();
        if count == 0 {
            self.selected = 0;
            return;
        }
        self.selected = (self.selected as i64 + delta).clamp(0, count as i64 - 1) as usize;
    }

    pub fn jump(&mut self, home: bool) {
        let count = self.visible().len();
        if count == 0 {
            return;
        }
        self.selected = if home { 0 } else { count - 1 };
    }

    pub fn clamp(&mut self, page: usize) {
        let page = page.max(1);
        let count = self.visible().len();
        self.selected = if count > 0 { self.selected.min(count - 1) } else { 0 };
        if self.selected < self.scroll {
            self.scroll = self.selected;
        } else if self.selected >= self.scroll + page {
            self.scroll = self.selected + 1 - page;
        }
        self.scroll = self.scroll.min(count.saturating_sub(page));
    }

    pub fn selected_row(&self) -> Option<&ProxyRow> {
        let rows = self.visible();
        if rows.is_empty() {
            return None;
        }
        Some(rows[self.selected.min(rows.len() - 1)])
    }

    pub fn use_selected(&mut self) -> String {
        let Some(row) = self.selected_row() else {
            return "no proxy selected".to_string();
        };
        let (index, display) = (row.index, row.proxy.display());
        let _ = self.store.set_config(&[("current", index.to_string())]);
        self.current = index as i64;
        format!("current #{}  {}", index, display)
    }

    pub fn step(&mut self, delta: i64) -> String {
        let count = self.visible().len();
        if count == 0 {
            return "no proxy selected".to_string();
        }
        self.selected = (self.selected as i64 + delta).rem_euclid(count as i64) as usize;
        self.use_selected()
    }

    pub fn toggle_power(&mut self) -> String {
        let enabled = !self.enabled();
        let flag = if enabled { "1" } else { "0" };
        let _ = self.store.set_config(&[("enabled", flag.to_string())]);
        self.cfg.insert("enabled".to_string(), flag.to_string());
        if enabled {
            "proxying ON".to_string()
        } else {
            "proxying OFF - traffic passes straight through".to_string()
        }
    }

    pub fn set_rotate(&mut self, seconds: i64) -> String {
        let seconds = seconds.max(0);
        let _ = self.store.set_config(&[("rotate_sec", seconds.to_string())]);
        self.cfg.insert("rotate_sec".to_string(), seconds.to_string());
        if seconds > 0 {
            format!("rotating every {}s", seconds)
        } else {
            "automatic rotation off".to_string()
        }
    }

    pub fn start_test(&mut self) -> String {
        if self.test.is_some() {
            return "a test is already running".to_string();
        }
        if self.rows.is_empty() {
            return "no proxies to test".to_string();
        }
        let rows: Vec<(usize, Proxy)> = self.rows.iter().map(|r| (r.index, r.proxy.clone())).collect();
        self.test = Some(TestRun::start(rows, 24, 12.0, Arc::clone(&self.net)));
        format!("testing {} proxies in the background", self.rows.len())
    }

    pub fn poll_test(&mut self) {
        let Some(run) = self.test.as_mut() else {
            return;
        };
        for result in run.drain() {
            self.health.insert(
                result.proxy.label(),
                HealthEntry { ok: result.ok, ms: result.ms, ip: result.exit_ip.clone(), note: result.note.clone(), ts: 0 },
            );
        }
        if !run.direct.is_empty() {
            self.direct = run.direct.clone();
        }
        if run.finished {
            self.message = format!("test finished: {} usable / {} checked", run.good, run.total);
            self.test = None;
        }
    }

    pub fn clean(&mut self) -> String {
        format!("removed {} stale state file(s)", processes::clean_dead(&self.paths))
    }
}

#[cfg(test)]
pub mod tests {
    use super::*;
    use crate::specs::parse_spec;

    pub struct FakeNet;

    impl Net for FakeNet {
        fn direct_ip(&self) -> Option<String> {
            Some("9.9.9.9".to_string())
        }

        fn tester(&self) -> TestFn {
            Arc::new(|idx, proxy, _| TestResult {
                idx,
                proxy: proxy.clone(),
                ok: true,
                exit_ip: "8.8.8.8".to_string(),
                ms: 5,
                note: String::new(),
            })
        }

        fn save_health(&self, _results: &[TestResult]) {}
    }

    pub fn make_screen() -> Screen {
        let mut screen = Screen::with_net(Paths::new(std::env::temp_dir()), Arc::new(FakeNet));
        for (i, text) in ["1.1.1.1:80", "2.2.2.2:80", "3.3.3.3:80", "4.4.4.4:80"].iter().enumerate() {
            screen.rows.push(ProxyRow { index: i, text: text.to_string(), proxy: parse_spec(text, "auto").unwrap() });
        }
        screen.direct = "9.9.9.9".to_string();
        let entry = |ok: bool, ip: &str, note: &str| HealthEntry { ok, ms: 10, ip: ip.to_string(), note: note.to_string(), ts: 0 };
        screen.health.insert(screen.rows[0].proxy.label(), entry(true, "8.8.8.8", ""));
        screen.health.insert(screen.rows[1].proxy.label(), entry(false, "", "timeout"));
        screen.health.insert(screen.rows[2].proxy.label(), entry(true, "9.9.9.9", ""));
        screen
    }

    fn indices(screen: &Screen) -> Vec<usize> {
        screen.visible().iter().map(|r| r.index).collect()
    }

    #[test]
    fn filters() {
        let mut screen = make_screen();
        assert_eq!(screen.visible().len(), 4);
        screen.filter = "working";
        assert_eq!(indices(&screen), vec![0]);
        screen.filter = "untested";
        assert_eq!(indices(&screen), vec![3]);
        screen.filter = "failed";
        assert_eq!(indices(&screen), vec![1]);
        screen.filter = "same";
        assert_eq!(indices(&screen), vec![2]);
    }

    #[test]
    fn cycle_filter_returns_to_all() {
        let mut screen = make_screen();
        let mut seen = Vec::new();
        for _ in 0..5 {
            screen.cycle_filter();
            seen.push(screen.filter);
        }
        assert_eq!(screen.filter, "all");
        assert_eq!(seen[0], "working");
    }

    #[test]
    fn query_matches_host_and_scheme() {
        let mut screen = make_screen();
        screen.set_query("2.2.2");
        assert_eq!(indices(&screen), vec![1]);
        screen.set_query("socks4");
        assert!(screen.visible().is_empty());
    }

    #[test]
    fn scroll_clamp_keeps_selection_visible() {
        let mut screen = make_screen();
        screen.selected = 3;
        screen.clamp(2);
        assert_eq!(screen.scroll, 2);
        screen.selected = 0;
        screen.clamp(2);
        assert_eq!(screen.scroll, 0);
    }

    #[test]
    fn move_stays_inside_the_list() {
        let mut screen = make_screen();
        screen.move_by(-5);
        assert_eq!(screen.selected, 0);
        screen.move_by(50);
        assert_eq!(screen.selected, 3);
    }

    #[test]
    fn step_wraps_around() {
        let mut screen = make_screen();
        screen.paths = Paths::new(std::env::temp_dir().join(format!("ppp-step-{}", std::process::id())));
        let _ = std::fs::create_dir_all(&screen.paths.root);
        screen.store = Store::new(screen.paths.clone());
        screen.selected = 3;
        screen.step(1);
        assert_eq!(screen.selected, 0);
        screen.step(-1);
        assert_eq!(screen.selected, 3);
    }

    #[test]
    fn results_stream_and_finish() {
        let rows = vec![(0, parse_spec("1.1.1.1:80", "auto").unwrap()), (1, parse_spec("2.2.2.2:80", "auto").unwrap())];
        let mut run = TestRun::start(rows, 24, 12.0, Arc::new(FakeNet));
        let mut collected = Vec::new();
        for _ in 0..500 {
            collected.extend(run.drain());
            if run.finished {
                break;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        assert!(run.finished);
        assert_eq!(run.done, 2);
        assert_eq!(run.good, 2);
        assert_eq!(collected.len(), 2);
    }

    #[test]
    fn url_detection() {
        assert!(is_url(" HTTPS://example.com/list.txt "));
        assert!(!is_url("1.2.3.4:8080"));
    }
}
