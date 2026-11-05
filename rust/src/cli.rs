use std::collections::HashSet;
use std::net::{SocketAddr, TcpStream, ToSocketAddrs};
use std::path::PathBuf;
use std::time::Duration;

use clap::{Parser, Subcommand};

use crate::backend;
use crate::netcheck::{self, TestResult};
use crate::paths::{Paths, IS_WINDOWS};
use crate::processes;
use crate::store::Store;
use crate::tui::view::fmt_age;
use crate::util;

#[derive(Parser)]
#[command(name = "rot", version, about = "Control panel for proxyinject.dll. No arguments opens the interactive console.")]
pub struct Cli {
    #[command(subcommand)]
    pub cmd: Option<Cmd>,
}

#[derive(Subcommand)]
pub enum Cmd {
    /// interactive console with hotkeys (default)
    Ui,
    /// plain-text console driven by typed commands
    Console,
    /// settings, current proxy, injected processes
    Status,
    /// list proxies in the database
    List,
    /// check the environment
    Doctor,
    /// merge proxy/*.txt into the database
    Sync,
    /// drop state files of dead processes
    Clean,
    /// append proxies
    Add {
        #[arg(required = true)]
        specs: Vec<String>,
    },
    /// import a file, or sync the proxy/ folder
    Import { file: Option<String> },
    /// delete proxies by index
    Rm {
        #[arg(required = true, allow_negative_numbers = true)]
        indices: Vec<i64>,
    },
    /// switch to proxy #N now
    Use {
        #[arg(allow_negative_numbers = true)]
        index: i64,
    },
    /// switch to the next proxy
    Next,
    /// rotate every N seconds (0 = off)
    Auto {
        #[arg(allow_negative_numbers = true)]
        seconds: i64,
    },
    /// enable proxying
    On,
    /// disable proxying (pass through)
    Off,
    /// check every proxy and print its exit IP
    Test {
        #[arg(long, default_value_t = 24)]
        concurrency: usize,
        #[arg(long, default_value_t = 12.0)]
        timeout: f64,
        /// delete the entries that failed
        #[arg(long)]
        prune: bool,
    },
    /// inject every new instance of an exe
    Watch {
        #[arg(value_name = "EXE")]
        name: String,
        #[arg(long, default_value_t = 2.0)]
        interval: f64,
    },
    /// inject the DLL into a process
    Inject {
        #[arg(long, value_name = "CMD")]
        launch: Option<String>,
        #[arg(long, value_name = "EXE")]
        name: Option<String>,
        #[arg(long)]
        pid: Vec<u32>,
        #[arg(long)]
        list: bool,
    },
    /// start a program already injected
    Launch {
        #[arg(value_name = "CMD")]
        command: String,
    },
    /// inject into a running process (pid or exe name)
    Attach {
        #[arg(value_name = "PID|EXE")]
        target: String,
    },
    /// terminate an injected process
    Kill { pid: u32 },
    /// print the raw spec (with credentials) of a proxy, for test scripts
    #[command(hide = true)]
    Raw {
        #[arg(long)]
        credentialed: bool,
    },
    /// show every setting (stored in the database)
    Config,
    /// change one setting, e.g. `rot set fail_closed 0`
    Set { key: String, value: String },
}

fn cmd_config(paths: &Paths) -> i32 {
    let cfg = Store::new(paths.clone()).read_config();
    let mut keys: Vec<&String> = cfg.keys().collect();
    keys.sort();
    for key in keys {
        println!("{:<16} {}", key, cfg[key]);
    }
    0
}

fn cmd_set(paths: &Paths, key: &str, value: &str) -> i32 {
    let key = key.to_lowercase();
    if !crate::store::is_known_setting(&key) {
        let names: Vec<&str> = crate::store::DEFAULT_SETTINGS.iter().map(|(name, _)| *name).collect();
        eprintln!("unknown setting '{}'; known: {}", key, names.join(", "));
        return 1;
    }
    if value.trim().is_empty() || value.contains('\n') {
        eprintln!("bad value");
        return 1;
    }
    match Store::new(paths.clone()).set_config(&[(key.as_str(), value.trim().to_string())]) {
        Ok(()) => {
            println!("{} = {}", key, value.trim());
            0
        }
        Err(e) => {
            eprintln!("cannot save: {}", e);
            1
        }
    }
}

fn cmd_raw(paths: &Paths, credentialed: bool) -> i32 {
    let rows = Store::new(paths.clone()).proxies();
    let pick = rows.iter().find(|row| !credentialed || !row.proxy.user.is_empty());
    match pick {
        Some(row) => {
            println!("{}", row.proxy.label());
            0
        }
        None => 1,
    }
}

pub fn health_text(paths: &Paths, label: &str) -> String {
    match netcheck::load_health(paths).get(label) {
        None => "untested".to_string(),
        Some(entry) if !entry.ok => {
            let note: String = entry.note.chars().take(28).collect();
            format!("FAIL {}", note)
        }
        Some(entry) => format!("ok {}ms", entry.ms),
    }
}

fn print_lines(lines: &[String]) {
    for line in lines {
        println!("{}", line);
    }
}

pub fn cmd_status(paths: &Paths) -> i32 {
    let store = Store::new(paths.clone());
    let cfg = store.read_config();
    let get = |key: &str, default: &str| cfg.get(key).cloned().unwrap_or_else(|| default.to_string());
    let rows = store.proxies();
    let cur = store.current_index();
    println!("folder        {}", paths.root.display());
    if IS_WINDOWS {
        println!(
            "dll           {}   inject.exe {}",
            if paths.dll().exists() { "ok" } else { "MISSING" },
            if paths.inject().exists() { "ok" } else { "MISSING" }
        );
    } else {
        match backend::shim_path(paths) {
            Some(shim) => println!("shim          {}", shim.display()),
            None => println!("shim          MISSING (build src/proxyinject_posix.c)"),
        }
    }
    println!("proxying      {}", if get("enabled", "1") == "1" { "ON" } else { "OFF (pass-through)" });
    println!("fail closed   {}", get("fail_closed", "1"));
    println!("udp           {}", if get("block_udp", "1") == "1" { "blocked" } else { "allowed (leaks)" });
    let rot = get("rotate_sec", "0");
    println!("auto rotate   {}", if rot != "0" && !rot.is_empty() { format!("every {}s", rot) } else { "off".to_string() });
    println!("proxies       {}  (database: {}, source folder: proxy/)", rows.len(), paths.db.file_name().and_then(|n| n.to_str()).unwrap_or("proxies.db"));
    if !rows.is_empty() {
        let row = if cur >= 0 && (cur as usize) < rows.len() { &rows[cur as usize] } else { &rows[0] };
        println!("current       #{}  {}  [{}]", cur, row.proxy.display(), health_text(paths, &row.proxy.label()));
    } else {
        println!("current       -    (no proxies, traffic goes direct)");
    }

    let injected = processes::read_states(paths, false);
    if !injected.is_empty() {
        let alive = injected.iter().filter(|st| st.alive()).count();
        println!("injected      {} alive / {} recorded", alive, injected.len());
        for st in &injected {
            let state = if st.alive() { "alive" } else { "gone " };
            println!("  pid {:>6}  {}  {:<24} proxy #{}  seen {} ago", st.pid, state, st.exe, st.current, fmt_age(st.age));
        }
    } else {
        println!("injected      none");
    }
    let stale = injected.iter().filter(|st| !st.alive()).count();
    if stale > 0 {
        println!("stale state   {} (run: rot clean)", stale);
    }
    0
}

pub fn cmd_list(paths: &Paths) -> i32 {
    let store = Store::new(paths.clone());
    let rows = store.read_proxy_lines();
    if rows.is_empty() {
        println!("the proxy list is empty - drop a list into proxy/ or use: rot add host:port");
        return 0;
    }
    let cur = store.current_index();
    let mut n: i64 = 0;
    for line in rows {
        match line.proxy {
            None => {
                let text: String = line.text.chars().take(50).collect();
                println!("    !  {}   [invalid: {}]", text, line.error);
            }
            Some(proxy) => {
                let mark = if n == cur { "*" } else { " " };
                println!("{} {:>3}  {:<46} {}", mark, n, proxy.display(), health_text(paths, &proxy.label()));
                n += 1;
            }
        }
    }
    0
}

pub fn cmd_add(paths: &Paths, specs: &[String]) -> i32 {
    let store = Store::new(paths.clone());
    let (added, notes) = store.add_specs(specs);
    print_lines(&notes);
    let total = store.read_proxy_lines().len();
    println!("{} added, {} total", added, total);
    if added > 0 || total > 0 { 0 } else { 1 }
}

pub fn cmd_import(paths: &Paths, file: Option<&str>) -> i32 {
    let Some(file) = file else {
        return cmd_sync(paths);
    };
    let store = Store::new(paths.clone());
    let src = PathBuf::from(file);
    match store.import_file(&src) {
        Ok((added, skipped, total)) => {
            println!("imported {}, skipped {}, total {}", added, skipped, total);
            0
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            println!("no such file: {}", src.display());
            1
        }
        Err(e) => {
            println!("cannot read {}: {}", src.display(), e);
            1
        }
    }
}

pub fn cmd_sync(paths: &Paths) -> i32 {
    let result = Store::new(paths.clone()).sync_proxy_dir();
    if result.files == 0 {
        println!("proxy/ is empty - drop proxy lists there and rerun");
        return 0;
    }
    println!(
        "scanned {} file(s) in proxy/: added {}, unusable {}, total {}",
        result.files, result.added, result.skipped, result.total
    );
    0
}

fn set_current(paths: &Paths, index: i64) -> i32 {
    let store = Store::new(paths.clone());
    let rows = store.proxies();
    if rows.is_empty() {
        eprintln!("the proxy list is empty");
        return 1;
    }
    let index = index.rem_euclid(rows.len() as i64);
    let _ = store.set_config(&[("current", index.to_string())]);
    println!("current = #{}  {}", index, rows[index as usize].proxy.display());
    0
}

pub fn cmd_next(paths: &Paths) -> i32 {
    let current = Store::new(paths.clone()).current_index();
    set_current(paths, current + 1)
}

pub fn cmd_auto(paths: &Paths, seconds: i64) -> i32 {
    let _ = Store::new(paths.clone()).set_config(&[("rotate_sec", seconds.to_string())]);
    if seconds != 0 {
        println!("rotating every {}s (live sockets are dropped on each rotation)", seconds);
    } else {
        println!("automatic rotation off");
    }
    0
}

pub fn cmd_power(paths: &Paths, on: bool) -> i32 {
    let _ = Store::new(paths.clone()).set_config(&[("enabled", if on { "1" } else { "0" }.to_string())]);
    println!("{}", if on { "proxying ON" } else { "proxying OFF - traffic passes straight through" });
    0
}

pub fn cmd_rm(paths: &Paths, indices: &[i64]) -> i32 {
    let store = Store::new(paths.clone());
    let removed = store.remove_indices(indices);
    println!("removed {}, {} left", removed, store.read_proxy_lines().len());
    0
}

fn is_same(result: &TestResult, direct: &Option<String>) -> bool {
    result.ok && matches!(direct, Some(d) if !d.is_empty() && &result.exit_ip == d)
}

pub fn run_tests(paths: &Paths, concurrency: usize, timeout: f64) -> (Vec<TestResult>, Option<String>) {
    let store = Store::new(paths.clone());
    let rows = store.proxies();
    if rows.is_empty() {
        println!("no proxies to test");
        return (Vec::new(), None);
    }
    println!("finding this machine's direct IP...");
    let direct = netcheck::direct_ip(6.0);
    println!("direct IP: {}\n", direct.clone().unwrap_or_else(|| "unknown (offline?)".to_string()));
    let entries = rows.iter().map(|r| (r.index, r.proxy.clone())).collect();
    let results = netcheck::test_all(entries, concurrency, timeout, netcheck::real_tester());
    netcheck::save_health(paths, &results);
    let mut good = 0;
    for r in &results {
        let mut mark = "ok  ";
        let mut note = r.note.clone();
        if is_same(r, &direct) {
            mark = "same";
            note = "exit IP equals your direct IP - useless for hiding".to_string();
        } else if !r.ok {
            mark = "FAIL";
        } else {
            good += 1;
        }
        let ip = if r.exit_ip.is_empty() { "-" } else { r.exit_ip.as_str() };
        println!("{} #{:<3} {:<46} {:>5}ms  {:<16} {}", mark, r.idx, r.proxy.display(), r.ms, ip, note);
    }
    println!("\n{} usable, {} failed (out of {})", good, results.len() - good, results.len());
    (results, direct)
}

pub fn cmd_test(paths: &Paths, concurrency: usize, timeout: f64, prune: bool) -> i32 {
    let (results, direct) = run_tests(paths, concurrency, timeout);
    if results.is_empty() {
        return 1;
    }
    let good: Vec<&TestResult> = results.iter().filter(|r| r.ok && !is_same(r, &direct)).collect();
    if prune {
        let keep_labels: HashSet<String> = good.iter().map(|r| r.proxy.label()).collect();
        let store = Store::new(paths.clone());
        let keep: Vec<String> = store
            .proxies()
            .into_iter()
            .filter(|row| keep_labels.contains(&row.proxy.label()))
            .map(|row| row.text)
            .collect();
        let _ = store.write_proxy_lines(&keep);
        println!("pruned the list to {} working entries", keep.len());
    }
    if good.is_empty() { 1 } else { 0 }
}

pub fn cmd_inject(paths: &Paths, launch: Option<&str>, name: Option<&str>, pids: &[u32], list: bool) -> i32 {
    let outcome = if let Some(command) = launch {
        backend::launch(paths, command)
    } else if let Some(name) = name {
        backend::inject_name(paths, name)
    } else if !pids.is_empty() {
        let mut rc = 0;
        let mut lines = Vec::new();
        for pid in pids {
            match backend::inject_pid(paths, *pid) {
                Ok((pid_rc, pid_lines)) => {
                    if rc == 0 {
                        rc = pid_rc;
                    }
                    lines.extend(pid_lines);
                }
                Err(e) => {
                    println!("inject: {}", e);
                    return 1;
                }
            }
        }
        Ok((rc, lines))
    } else if list {
        backend::list_processes(paths)
    } else {
        println!("choose --launch, --name, --pid or --list");
        return 1;
    };
    match outcome {
        Ok((rc, lines)) => {
            print_lines(&lines);
            rc
        }
        Err(e) => {
            println!("inject: {}", e);
            1
        }
    }
}

pub fn cmd_launch(paths: &Paths, command: &str) -> i32 {
    cmd_inject(paths, Some(command), None, &[], false)
}

pub fn cmd_attach(paths: &Paths, target: &str) -> i32 {
    let outcome = match target.parse::<u32>() {
        Ok(pid) if target.bytes().all(|b| b.is_ascii_digit()) => backend::inject_pid(paths, pid),
        _ => backend::inject_name(paths, target),
    };
    match outcome {
        Ok((rc, lines)) => {
            print_lines(&lines);
            rc
        }
        Err(e) => {
            println!("attach: {}", e);
            1
        }
    }
}

pub fn cmd_kill(paths: &Paths, pid: u32) -> i32 {
    let ok = processes::kill_process(pid);
    processes::clean_dead(paths);
    println!("pid {}: {}", pid, if ok { "terminated" } else { "not running or access denied" });
    if ok { 0 } else { 1 }
}

pub fn cmd_clean(paths: &Paths) -> i32 {
    println!("removed {} stale state file(s)", processes::clean_dead(paths));
    0
}

pub fn cmd_watch(paths: &Paths, name: &str, interval: f64) -> i32 {
    println!("watching for {} every {}s - Ctrl+C to stop", name, interval);
    loop {
        match backend::inject_name(paths, name) {
            Ok((_rc, lines)) => {
                for line in lines {
                    if line.contains("injected,") {
                        println!("{} {}", util::local_now().hms(), line.trim());
                    }
                }
            }
            Err(e) => {
                println!("watch: {}", e);
                return 1;
            }
        }
        std::thread::sleep(Duration::from_secs_f64(interval.max(0.5)));
    }
}

#[cfg(windows)]
fn is_admin() -> bool {
    unsafe { windows_sys::Win32::UI::Shell::IsUserAnAdmin() != 0 }
}

#[cfg(unix)]
fn is_admin() -> bool {
    unsafe { libc::geteuid() == 0 }
}

fn exists_or_missing(path: &std::path::Path) -> String {
    if path.exists() { path.display().to_string() } else { format!("MISSING {}", path.display()) }
}

fn present(path: &std::path::Path) -> String {
    if path.exists() { path.display().to_string() } else { "missing".to_string() }
}

pub fn cmd_doctor(paths: &Paths) -> i32 {
    let exe = std::env::current_exe().map(|p| p.display().to_string()).unwrap_or_default();
    println!("rot           {}  ({})", env!("CARGO_PKG_VERSION"), exe);
    println!(
        "admin         {}",
        if is_admin() { "yes" } else { "no (needed only for protected/elevated targets)" }
    );
    println!("dll           {}", exists_or_missing(&paths.dll()));
    println!("injector      {}", exists_or_missing(&paths.inject()));
    println!("database      {}", present(&paths.db));
    println!("proxy folder  {}", present(&paths.proxy_dir));
    println!("state folder  {}", present(&paths.state_dir));
    if backend::available(paths) {
        if IS_WINDOWS {
            println!("backend       windows (inject.exe)");
        } else {
            let name = backend::shim_path(paths)
                .and_then(|p| p.file_name().map(|n| n.to_string_lossy().into_owned()))
                .unwrap_or_default();
            println!("backend       {}", name);
        }
    } else {
        println!("backend       unavailable - see README");
    }

    let ip = netcheck::direct_ip(6.0);
    println!("direct IP     {}", ip.unwrap_or_else(|| "unreachable".to_string()));

    let mut found = false;
    for port in [7897u16, 7890, 7891] {
        let addr: SocketAddr = ([127, 0, 0, 1], port).into();
        if TcpStream::connect_timeout(&addr, Duration::from_millis(400)).is_ok() {
            println!("clash         listening on 127.0.0.1:{}", port);
            found = true;
            break;
        }
    }
    if !found {
        println!("clash         not detected on 7890/7891/7897");
    }

    match ("example.com", 80u16).to_socket_addrs() {
        Ok(addrs) => {
            let mut v4: Vec<String> = addrs.filter(|a| a.is_ipv4()).map(|a| a.ip().to_string()).collect();
            v4.sort();
            v4.dedup();
            let fake: Vec<&String> = v4.iter().filter(|a| a.starts_with("198.18.") || a.starts_with("198.19.")).collect();
            if !fake.is_empty() {
                let list: Vec<&str> = fake.iter().map(|s| s.as_str()).collect();
                println!(
                    "dns           fake-ip detected ({}) - handled: hostnames are sent to the proxy instead of these addresses",
                    list.join(", ")
                );
            } else {
                println!("dns           example.com -> {}", v4.join(", "));
            }
        }
        Err(e) => println!("dns           lookup failed: {}", e),
    }

    let states = processes::read_states(paths, false);
    if !states.is_empty() {
        let alive = states.iter().filter(|st| st.alive()).count();
        println!("injected      {} alive / {} recorded", alive, states.len());
        for st in &states {
            if !st.alive() {
                let file = st.path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                println!("  stale state {} (process gone)", file);
            }
        }
    } else {
        println!("injected      none");
    }
    println!("\nReminder: public proxies are often dead or blocked; `rot test` tells you which ones really work from this machine.");
    0
}

pub fn dispatch(paths: &Paths, cmd: Cmd) -> i32 {
    match cmd {
        Cmd::Ui => crate::tui::run(paths),
        Cmd::Console => crate::ui::run(paths),
        Cmd::Status => cmd_status(paths),
        Cmd::List => cmd_list(paths),
        Cmd::Doctor => cmd_doctor(paths),
        Cmd::Sync => cmd_sync(paths),
        Cmd::Clean => cmd_clean(paths),
        Cmd::Add { specs } => cmd_add(paths, &specs),
        Cmd::Import { file } => cmd_import(paths, file.as_deref()),
        Cmd::Rm { indices } => cmd_rm(paths, &indices),
        Cmd::Use { index } => set_current(paths, index),
        Cmd::Next => cmd_next(paths),
        Cmd::Auto { seconds } => cmd_auto(paths, seconds),
        Cmd::On => cmd_power(paths, true),
        Cmd::Off => cmd_power(paths, false),
        Cmd::Test { concurrency, timeout, prune } => cmd_test(paths, concurrency, timeout, prune),
        Cmd::Watch { name, interval } => cmd_watch(paths, &name, interval),
        Cmd::Inject { launch, name, pid, list } => cmd_inject(paths, launch.as_deref(), name.as_deref(), &pid, list),
        Cmd::Launch { command } => cmd_launch(paths, &command),
        Cmd::Attach { target } => cmd_attach(paths, &target),
        Cmd::Kill { pid } => cmd_kill(paths, pid),
        Cmd::Raw { credentialed } => cmd_raw(paths, credentialed),
        Cmd::Config => cmd_config(paths),
        Cmd::Set { key, value } => cmd_set(paths, &key, &value),
    }
}
