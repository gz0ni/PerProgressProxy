mod backend;
mod cli;
mod netcheck;
mod paths;
mod processes;
mod specs;
mod store;
mod tui;
mod ui;
mod util;

use std::io::IsTerminal;
use std::panic::AssertUnwindSafe;
use std::sync::Mutex;

use clap::Parser;

use cli::Cli;
use paths::Paths;

static PANIC_REPORT: Mutex<String> = Mutex::new(String::new());

fn log_crash(paths: &Paths, args: &[String], report: &str) -> Option<String> {
    use std::io::Write;
    std::fs::create_dir_all(&paths.logs_dir).ok()?;
    let path = paths.logs_dir.join("ui-crash.log");
    let mut file = std::fs::OpenOptions::new().create(true).append(true).open(&path).ok()?;
    writeln!(file, "[{}] rot {}", util::local_now().full(), args.join(" ")).ok()?;
    writeln!(file, "{}\n", report).ok()?;
    Some(path.display().to_string())
}

fn run(paths: &Paths) -> i32 {
    processes::migrate_legacy(paths);
    let cli = Cli::parse();
    match cli.cmd {
        Some(cmd) => cli::dispatch(paths, cmd),
        None => {
            if !std::io::stdout().is_terminal() {
                println!("no terminal detected: use a one-shot command instead, e.g. `rot status`");
                return 2;
            }
            cli::dispatch(paths, cli::Cmd::Ui)
        }
    }
}

fn main() {
    util::install_interrupt_handler();
    std::panic::set_hook(Box::new(|info| {
        let backtrace = std::backtrace::Backtrace::force_capture();
        if let Ok(mut report) = PANIC_REPORT.lock() {
            *report = format!("{}\n{}", info, backtrace);
        }
    }));
    let paths = Paths::discover();
    let args: Vec<String> = std::env::args().skip(1).collect();
    let code = match std::panic::catch_unwind(AssertUnwindSafe(|| run(&paths))) {
        Ok(code) => code,
        Err(_) => {
            let report = PANIC_REPORT.lock().map(|r| r.clone()).unwrap_or_default();
            eprintln!("{}", report);
            if let Some(path) = log_crash(&paths, &args, &report) {
                eprintln!("\ncrash logged to {}", path);
            }
            1
        }
    };
    std::process::exit(code);
}
