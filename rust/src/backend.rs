use std::fmt;
use std::path::PathBuf;
use std::process::Command;

use crate::paths::{Paths, IS_WINDOWS};

#[derive(Debug, Clone)]
pub struct BackendError(pub String);

impl fmt::Display for BackendError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for BackendError {}

pub type Outcome = Result<(i32, Vec<String>), BackendError>;

fn fail<T>(message: impl Into<String>) -> Result<T, BackendError> {
    Err(BackendError(message.into()))
}

pub fn shim_path(paths: &Paths) -> Option<PathBuf> {
    for folder in [&paths.bin_dir, &paths.root] {
        for name in ["proxyinject.so", "proxyinject.dylib"] {
            let candidate = folder.join(name);
            if candidate.exists() {
                return Some(candidate);
            }
        }
    }
    None
}

pub fn available(paths: &Paths) -> bool {
    if IS_WINDOWS { paths.inject().exists() } else { shim_path(paths).is_some() }
}

fn nonblank_lines(bytes: &[u8]) -> Vec<String> {
    String::from_utf8_lossy(bytes)
        .lines()
        .filter(|line| !line.trim().is_empty())
        .map(str::to_string)
        .collect()
}

fn run_windows(paths: &Paths, args: &[&str]) -> Outcome {
    let inject = paths.inject();
    if !inject.exists() {
        return fail(inject.file_name().and_then(|n| n.to_str()).unwrap_or("inject.exe"));
    }
    let output = Command::new(&inject)
        .args(args)
        .output()
        .map_err(|e| BackendError(format!("{}: {}", inject.display(), e)))?;
    let mut lines = nonblank_lines(&output.stdout);
    lines.extend(nonblank_lines(&output.stderr));
    Ok((output.status.code().unwrap_or(1), lines))
}

pub fn list_processes(paths: &Paths) -> Outcome {
    if !IS_WINDOWS {
        return fail("process listing uses the Windows injector; use ps for now");
    }
    run_windows(paths, &["--list"])
}

pub fn inject_pid(paths: &Paths, pid: u32) -> Outcome {
    if !IS_WINDOWS {
        return fail(
            "attaching to an already running process needs ptrace and is not implemented; \
             start the app through the shim instead: launch \"...\"",
        );
    }
    run_windows(paths, &["--pid", &pid.to_string()])
}

pub fn inject_name(paths: &Paths, name: &str) -> Outcome {
    if !IS_WINDOWS {
        return fail("attaching by name is Windows-only; start the app through the shim instead");
    }
    run_windows(paths, &["--name", name])
}

pub fn shell_split(text: &str) -> Result<Vec<String>, BackendError> {
    let mut words = Vec::new();
    let mut current = String::new();
    let mut in_word = false;
    let mut chars = text.chars();
    while let Some(c) = chars.next() {
        match c {
            '\'' => {
                in_word = true;
                loop {
                    match chars.next() {
                        Some('\'') => break,
                        Some(ch) => current.push(ch),
                        None => return fail("No closing quotation"),
                    }
                }
            }
            '"' => {
                in_word = true;
                loop {
                    match chars.next() {
                        Some('"') => break,
                        Some('\\') => match chars.next() {
                            Some(ch @ ('"' | '\\' | '$' | '`')) => current.push(ch),
                            Some(ch) => {
                                current.push('\\');
                                current.push(ch);
                            }
                            None => return fail("No closing quotation"),
                        },
                        Some(ch) => current.push(ch),
                        None => return fail("No closing quotation"),
                    }
                }
            }
            '\\' => {
                in_word = true;
                match chars.next() {
                    Some(ch) => current.push(ch),
                    None => return fail("No escaped character"),
                }
            }
            c if c.is_whitespace() => {
                if in_word {
                    words.push(std::mem::take(&mut current));
                    in_word = false;
                }
            }
            c => {
                in_word = true;
                current.push(c);
            }
        }
    }
    if in_word {
        words.push(current);
    }
    Ok(words)
}

pub fn launch(paths: &Paths, command: &str) -> Outcome {
    if IS_WINDOWS {
        return run_windows(paths, &["--launch", command]);
    }
    let argv = shell_split(command)?;
    if argv.is_empty() {
        return fail("empty command");
    }
    let Some(shim) = shim_path(paths) else {
        return fail(
            "no shim built; run: zig cc -target <triple> -O2 -shared -fPIC \
             -o proxyinject.so src/proxyinject_posix.c -ldl",
        );
    };
    let is_dylib = shim.extension().and_then(|e| e.to_str()) == Some("dylib");
    let var = if is_dylib { "DYLD_INSERT_LIBRARIES" } else { "LD_PRELOAD" };
    let existing = std::env::var(var).unwrap_or_default();
    let value = if existing.is_empty() {
        shim.display().to_string()
    } else {
        format!("{}:{}", shim.display(), existing)
    };
    let child = Command::new(&argv[0])
        .args(&argv[1..])
        .env(var, value)
        .env("PROXYINJECT_DIR", &paths.root)
        .current_dir(&paths.root)
        .spawn()
        .map_err(|e| BackendError(format!("cannot start {}: {}", argv[0], e)))?;
    let shim_name = shim.file_name().and_then(|n| n.to_str()).unwrap_or("?").to_string();
    Ok((
        0,
        vec![
            format!("started pid {} under {}", child.id(), shim_name),
            format!("target keeps running; stop it with: kill {}", child.id()),
        ],
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shell_split_handles_quotes() {
        assert_eq!(shell_split("a 'b c' \"d e\" f\\ g").unwrap(), vec!["a", "b c", "d e", "f g"]);
        assert_eq!(shell_split("  ").unwrap(), Vec::<String>::new());
        assert!(shell_split("a 'b").is_err());
    }
}
