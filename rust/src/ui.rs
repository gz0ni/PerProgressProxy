use std::io::{self, IsTerminal, Write};
use std::time::Duration;

use crate::backend;
use crate::cli;
use crate::netcheck;
use crate::paths::Paths;
use crate::processes;
use crate::store::Store;
use crate::tui::term::enable_output_vt;
use crate::tui::view::fmt_age;

const RULES: &str = "------------------------------------------------------------------------------";

struct Colors {
    on: bool,
}

impl Colors {
    fn detect() -> Colors {
        let on = std::env::var_os("NO_COLOR").is_none() && io::stdout().is_terminal();
        if on {
            enable_output_vt();
        }
        Colors { on }
    }

    fn paint(&self, text: &str, code: &str) -> String {
        if self.on { format!("\x1b[{}m{}\x1b[0m", code, text) } else { text.to_string() }
    }

    fn green(&self, text: &str) -> String {
        self.paint(text, "32")
    }

    fn red(&self, text: &str) -> String {
        self.paint(text, "31")
    }

    fn yellow(&self, text: &str) -> String {
        self.paint(text, "33")
    }

    fn dim(&self, text: &str) -> String {
        self.paint(text, "2")
    }

    fn cyan(&self, text: &str) -> String {
        self.paint(text, "36")
    }
}

fn health_cell(c: &Colors, paths: &Paths, label: &str) -> String {
    match netcheck::load_health(paths).get(label) {
        None => c.dim("untested"),
        Some(entry) if !entry.ok => {
            let note = if entry.note.is_empty() { "failed" } else { entry.note.as_str() };
            let note: String = note.chars().take(22).collect();
            c.red(&format!("FAIL {}", note))
        }
        Some(entry) => format!("{} {}", c.green(&format!("ok {}ms", entry.ms)), c.dim(&entry.ip)),
    }
}

fn render(c: &Colors, paths: &Paths) {
    let store = Store::new(paths.clone());
    let cfg = store.read_config();
    let rows = store.proxies();
    let cur = store.current_index();
    let states = processes::read_states(paths, false);

    let enabled = cfg.get("enabled").map(String::as_str).unwrap_or("1") == "1";
    let power = if enabled { c.green("PROXY: ON ") } else { c.yellow("PROXY: OFF") };
    let current = if rows.is_empty() {
        "no proxies".to_string()
    } else {
        let row = if cur >= 0 && (cur as usize) < rows.len() { &rows[cur as usize] } else { &rows[0] };
        format!("current #{} {}", row.index, row.proxy.display())
    };
    let rotate = cfg.get("rotate_sec").cloned().unwrap_or_else(|| "0".to_string());
    let rotate_text = if rotate != "0" && !rotate.is_empty() { format!("rotate {}s", rotate) } else { "rotate manual".to_string() };

    println!();
    println!("  {}   {}   {}   {}", c.cyan("PerProcessProxy"), power, current, c.dim(&rotate_text));
    println!("  {}", c.dim(RULES));

    println!("  {}  {}", c.dim("PROXIES"), c.dim(&format!("{} entries, newest import from proxy/", rows.len())));
    if rows.is_empty() {
        println!("      {}", c.dim("none - drop a list into proxy/ and run: import"));
    } else {
        for row in &rows {
            let mark = if row.index as i64 == cur { c.cyan("*") } else { " ".to_string() };
            println!("  {} {:>3}  {:<46} {}", mark, row.index, row.proxy.display(), health_cell(c, paths, &row.proxy.label()));
        }
    }

    let alive = states.iter().filter(|st| st.alive()).count();
    println!("  {}", c.dim(RULES));
    println!("  {}  {}", c.dim("PROCESSES"), c.dim(&format!("{} alive / {} recorded", alive, states.len())));
    if states.is_empty() {
        println!("      {}", c.dim("none - use: launch \"C:\\path\\app.exe\" or attach <pid|name>"));
    } else {
        let mut sorted: Vec<_> = states.iter().map(|st| (st, st.alive())).collect();
        sorted.sort_by_key(|(st, alive)| (!*alive, st.pid));
        for (st, alive) in sorted {
            let tag = if alive { c.green("alive") } else { c.red("gone ") };
            let proxy_text = if alive { format!("#{}", st.current) } else { "-".to_string() };
            let exe: String = st.exe.chars().take(24).collect();
            println!("      {:>7}  {:<24} {}  {:<4} seen {} ago", st.pid, exe, tag, proxy_text, fmt_age(st.age));
        }
    }

    println!("  {}", c.dim(RULES));
    println!("  {}", c.dim("3 | next | on | off | test [--prune] | import | attach <pid|name>"));
    println!("  {}", c.dim("launch \"app.exe\" | stop [pid] | kill <pid> | clean | rm <n...> | add <spec>"));
    println!("  {}", c.dim("status | list | doctor | help | quit"));
}

fn read_line(prompt: &str) -> String {
    print!("{}", prompt);
    let _ = io::stdout().flush();
    let mut line = String::new();
    match io::stdin().read_line(&mut line) {
        Ok(0) | Err(_) => "quit".to_string(),
        Ok(_) => line.trim().to_string(),
    }
}

fn print_indented(lines: &[String]) {
    for line in lines {
        println!("  {}", line);
    }
}

fn cmd_test(c: &Colors, paths: &Paths, rest: &[&str]) {
    let prune = rest.contains(&"--prune");
    let store = Store::new(paths.clone());
    let rows = store.proxies();
    if rows.is_empty() {
        println!("  no proxies to test");
        return;
    }
    println!("  testing {} proxies in parallel, this may take ~12s...", rows.len());
    let direct = netcheck::direct_ip(6.0);
    let entries = rows.iter().map(|r| (r.index, r.proxy.clone())).collect();
    let results = netcheck::test_all(entries, 24, 12.0, netcheck::real_tester());
    netcheck::save_health(paths, &results);
    let same = |r: &netcheck::TestResult| r.ok && matches!(&direct, Some(d) if !d.is_empty() && &r.exit_ip == d);
    let mut good = 0;
    for r in &results {
        let mark = if r.ok && !same(r) {
            good += 1;
            c.green("ok  ")
        } else if r.ok {
            c.yellow("same")
        } else {
            c.red("FAIL")
        };
        let ip = if r.exit_ip.is_empty() { "-" } else { r.exit_ip.as_str() };
        println!("  {} #{:<3} {:<46} {:>5}ms  {:<16} {}", mark, r.idx, r.proxy.display(), r.ms, ip, r.note);
    }
    println!("  {} usable, {} failed", good, results.len() - good);
    if prune {
        let keep_labels: std::collections::HashSet<String> =
            results.iter().filter(|r| r.ok && !same(r)).map(|r| r.proxy.label()).collect();
        let keep: Vec<String> =
            rows.iter().filter(|row| keep_labels.contains(&row.proxy.label())).map(|row| row.text.clone()).collect();
        let _ = store.write_proxy_lines(&keep);
        println!("  pruned the list to {} working entries", keep.len());
    }
}

fn cmd_stop(paths: &Paths, rest: &[&str]) {
    let states = processes::read_states(paths, false);
    let mut targets: Vec<_> = states.into_iter().filter(|st| st.alive()).collect();
    if let Some(arg) = rest.first() {
        let Ok(pid) = arg.parse::<u32>() else {
            println!("  usage: stop [pid]");
            return;
        };
        targets.retain(|st| st.pid == pid);
    }
    if targets.is_empty() {
        println!("  nothing to stop");
        return;
    }
    if rest.is_empty() && targets.len() > 1 {
        let answer = read_line(&format!("  terminate {} process(es)? [y/N] ", targets.len())).to_lowercase();
        if !["y", "yes", "д", "да"].contains(&answer.as_str()) {
            println!("  cancelled");
            return;
        }
    }
    for st in &targets {
        let ok = processes::kill_process(st.pid);
        println!("  pid {} {}: {}", st.pid, st.exe, if ok { "terminated" } else { "failed" });
    }
    processes::clean_dead(paths);
}

fn cmd_attach(paths: &Paths, rest: &[&str]) {
    let Some(target) = rest.first() else {
        println!("  usage: attach <pid|exe name>");
        return;
    };
    let outcome = match target.parse::<u32>() {
        Ok(pid) if target.bytes().all(|b| b.is_ascii_digit()) => backend::inject_pid(paths, pid),
        _ => backend::inject_name(paths, target),
    };
    match outcome {
        Ok((_rc, lines)) => print_indented(&lines),
        Err(e) => {
            println!("  {}", e);
            return;
        }
    }
    std::thread::sleep(Duration::from_millis(700));
}

fn cmd_launch(paths: &Paths, raw: &str) {
    let mut command = raw.trim();
    let chars: Vec<char> = command.chars().collect();
    if chars.len() >= 2 && chars[0] == chars[chars.len() - 1] && (chars[0] == '"' || chars[0] == '\'') {
        command = &command[1..command.len() - 1];
    }
    if command.is_empty() {
        println!("  usage: launch \"C:\\path\\app.exe\"");
        return;
    }
    match backend::launch(paths, command) {
        Ok((_rc, lines)) => print_indented(&lines),
        Err(e) => {
            println!("  {}", e);
            return;
        }
    }
    std::thread::sleep(Duration::from_millis(700));
}

fn cmd_rm(paths: &Paths, rest: &[&str]) {
    let parsed: Result<Vec<i64>, _> = rest.iter().map(|item| item.parse::<i64>()).collect();
    match parsed {
        Ok(indices) => {
            let removed = Store::new(paths.clone()).remove_indices(&indices);
            println!("  removed {} entries", removed);
        }
        Err(_) => println!("  usage: rm <n> [n...]"),
    }
}

fn use_index(paths: &Paths, index: i64) {
    let store = Store::new(paths.clone());
    let rows = store.proxies();
    if rows.is_empty() {
        println!("  no proxies");
        return;
    }
    let index = index.rem_euclid(rows.len() as i64);
    let _ = store.set_config(&[("current", index.to_string())]);
    println!("  current #{}  {}", index, rows[index as usize].proxy.display());
}

fn dispatch(c: &Colors, paths: &Paths, line: &str) -> bool {
    if line.is_empty() {
        return true;
    }
    let parts: Vec<&str> = line.split_whitespace().collect();
    let cmd = parts[0].to_lowercase();
    let rest = &parts[1..];
    let raw_rest = line[parts[0].len()..].trim();
    let store = Store::new(paths.clone());

    match cmd.as_str() {
        "quit" | "exit" | "q" => return false,
        "help" | "?" => print_indented(&[
            "<number>        use that proxy".to_string(),
            "next            next proxy in the list".to_string(),
            "on / off        proxy / pass everything through".to_string(),
            "test [--prune]  check every proxy, optionally delete the failures".to_string(),
            "import          merge proxy/*.txt into the database".to_string(),
            "attach <pid|exe> inject into a running process".to_string(),
            "launch \"app\"    start a program already injected".to_string(),
            "stop [pid]      terminate injected process(es)".to_string(),
            "kill <pid>      same as stop <pid>".to_string(),
            "clean           drop state files of dead processes".to_string(),
            "rm <n...>       remove proxies by index".to_string(),
            "add <spec...>   append proxies".to_string(),
            "status/list/doctor  detailed views".to_string(),
        ]),
        "next" | "n" => {
            let current = store.current_index();
            use_index(paths, current + 1);
        }
        "on" | "off" => {
            let on = cmd == "on";
            let _ = store.set_config(&[("enabled", if on { "1" } else { "0" }.to_string())]);
            println!("  proxying {}", if on { "ON" } else { "OFF - traffic passes straight through" });
        }
        "test" => cmd_test(c, paths, rest),
        "import" | "sync" => {
            let result = store.sync_proxy_dir();
            if result.files == 0 {
                println!("  proxy/ is empty");
            } else {
                println!("  scanned {} file(s): added {}, unusable {}, total {}", result.files, result.added, result.skipped, result.total);
            }
        }
        "attach" | "inject" => cmd_attach(paths, rest),
        "launch" => cmd_launch(paths, raw_rest),
        "stop" | "kill" => cmd_stop(paths, rest),
        "clean" => println!("  removed {} stale state file(s)", processes::clean_dead(paths)),
        "rm" => cmd_rm(paths, rest),
        "add" => {
            if rest.is_empty() {
                println!("  usage: add host:port or host:port:user:pass");
            } else {
                let specs: Vec<String> = rest.iter().map(|s| s.to_string()).collect();
                let (added, notes) = store.add_specs(&specs);
                print_indented(&notes);
                println!("  {} added", added);
            }
        }
        "status" => {
            cli::cmd_status(paths);
        }
        "list" => {
            cli::cmd_list(paths);
        }
        "doctor" => {
            cli::cmd_doctor(paths);
        }
        other => {
            if !other.is_empty() && other.bytes().all(|b| b.is_ascii_digit()) {
                match other.parse::<i64>() {
                    Ok(index) => use_index(paths, index),
                    Err(_) => println!("  error: number too large"),
                }
            } else {
                println!("  unknown command: {} (try help)", parts[0]);
            }
        }
    }
    true
}

pub fn run(paths: &Paths) -> i32 {
    let c = Colors::detect();
    Store::new(paths.clone()).sync_proxy_dir();
    processes::migrate_legacy(paths);
    println!("  {}", c.dim("PerProcessProxy console. Type help for commands, quit to exit."));
    loop {
        render(&c, paths);
        let line = read_line("\n  > ");
        if !dispatch(&c, paths, &line) {
            println!("  bye");
            return 0;
        }
    }
}
