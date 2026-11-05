use std::fs;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use crate::paths::Paths;
use crate::store::read_text_lossy;

#[derive(Debug, Clone)]
pub struct ProcessState {
    pub pid: u32,
    pub current: i64,
    pub proxies: i64,
    pub enabled: bool,
    pub rotated_ms: i64,
    pub exe: String,
    pub age: f64,
    pub path: PathBuf,
}

impl ProcessState {
    pub fn alive(&self) -> bool {
        process_alive(self.pid)
    }
}

pub fn ensure_state_dir(paths: &Paths) -> PathBuf {
    let _ = fs::create_dir_all(&paths.state_dir);
    paths.state_dir.clone()
}

fn legacy_files(paths: &Paths) -> Vec<PathBuf> {
    let Ok(entries) = fs::read_dir(&paths.root) else {
        return Vec::new();
    };
    let mut files: Vec<PathBuf> = entries
        .filter_map(Result::ok)
        .map(|e| e.path())
        .filter(|p| {
            p.is_file()
                && p.file_name()
                    .and_then(|n| n.to_str())
                    .map(|n| n.starts_with("state_") && n.ends_with(".txt"))
                    .unwrap_or(false)
        })
        .collect();
    files.sort();
    files
}

pub fn migrate_legacy(paths: &Paths) -> usize {
    if !paths.root.exists() {
        return 0;
    }
    ensure_state_dir(paths);
    let mut moved = 0;
    for old in legacy_files(paths) {
        let name = old.file_name().and_then(|n| n.to_str()).unwrap_or("").replacen("state_", "", 1);
        let dest = paths.state_dir.join(name);
        if !dest.exists() {
            if fs::rename(&old, &dest).is_ok() {
                moved += 1;
            }
        } else {
            let _ = fs::remove_file(&old);
        }
    }
    moved
}

#[cfg(windows)]
mod sys {
    use std::path::Path;

    use windows_sys::Win32::Foundation::{CloseHandle, HANDLE};
    use windows_sys::Win32::System::Threading::{
        GetExitCodeProcess, OpenProcess, QueryFullProcessImageNameW, TerminateProcess,
        PROCESS_QUERY_LIMITED_INFORMATION, PROCESS_TERMINATE,
    };

    const STILL_ACTIVE: u32 = 259;

    fn open(pid: u32, access: u32) -> Option<HANDLE> {
        let handle = unsafe { OpenProcess(access, 0, pid) };
        if handle.is_null() { None } else { Some(handle) }
    }

    pub fn alive(pid: u32) -> bool {
        let Some(handle) = open(pid, PROCESS_QUERY_LIMITED_INFORMATION) else {
            return false;
        };
        let mut code = 0u32;
        let ok = unsafe { GetExitCodeProcess(handle, &mut code) };
        unsafe { CloseHandle(handle) };
        ok != 0 && code == STILL_ACTIVE
    }

    pub fn name(pid: u32) -> String {
        let Some(handle) = open(pid, PROCESS_QUERY_LIMITED_INFORMATION) else {
            return String::new();
        };
        let mut buf = [0u16; 1024];
        let mut size = buf.len() as u32;
        let ok = unsafe { QueryFullProcessImageNameW(handle, 0, buf.as_mut_ptr(), &mut size) };
        unsafe { CloseHandle(handle) };
        if ok == 0 {
            return String::new();
        }
        let full = String::from_utf16_lossy(&buf[..size as usize]);
        Path::new(&full).file_name().and_then(|n| n.to_str()).unwrap_or("").to_string()
    }

    pub fn kill(pid: u32) -> bool {
        let Some(handle) = open(pid, PROCESS_TERMINATE) else {
            return false;
        };
        let ok = unsafe { TerminateProcess(handle, 1) };
        unsafe { CloseHandle(handle) };
        ok != 0
    }
}

#[cfg(unix)]
mod sys {
    use std::path::Path;
    use std::process::Command;

    pub fn alive(pid: u32) -> bool {
        unsafe { libc::kill(pid as libc::pid_t, 0) == 0 }
    }

    pub fn name(pid: u32) -> String {
        if let Ok(text) = std::fs::read_to_string(format!("/proc/{}/comm", pid)) {
            let text = text.trim();
            if !text.is_empty() {
                return text.to_string();
            }
        }
        match Command::new("ps").args(["-o", "comm=", "-p", &pid.to_string()]).output() {
            Ok(out) => {
                let text = String::from_utf8_lossy(&out.stdout).trim().to_string();
                if text.is_empty() {
                    format!("pid {}", pid)
                } else {
                    Path::new(&text).file_name().and_then(|n| n.to_str()).unwrap_or(&text).to_string()
                }
            }
            Err(_) => format!("pid {}", pid),
        }
    }

    pub fn kill(pid: u32) -> bool {
        unsafe { libc::kill(pid as libc::pid_t, libc::SIGTERM) == 0 }
    }
}

pub fn process_alive(pid: u32) -> bool {
    pid > 0 && sys::alive(pid)
}

pub fn process_name(pid: u32) -> String {
    if pid == 0 { String::new() } else { sys::name(pid) }
}

pub fn kill_process(pid: u32) -> bool {
    pid > 0 && sys::kill(pid)
}

pub fn state_files(paths: &Paths) -> Vec<PathBuf> {
    let mut files: Vec<PathBuf> = Vec::new();
    if let Ok(entries) = fs::read_dir(&paths.state_dir) {
        let mut found: Vec<PathBuf> = entries
            .filter_map(Result::ok)
            .map(|e| e.path())
            .filter(|p| p.extension().and_then(|e| e.to_str()) == Some("txt"))
            .collect();
        found.sort();
        files.extend(found);
    }
    files.extend(legacy_files(paths));
    files
}

fn field_int(fields: &[(String, String)], key: &str) -> i64 {
    fields
        .iter()
        .find(|(k, _)| k == key)
        .and_then(|(_, v)| v.trim().parse().ok())
        .unwrap_or(0)
}

pub fn read_state(path: &Path) -> Option<ProcessState> {
    let text = read_text_lossy(path)?;
    let fields: Vec<(String, String)> = text
        .lines()
        .filter_map(|line| line.split_once('='))
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect();
    let pid = field_int(&fields, "pid");
    if pid <= 0 {
        return None;
    }
    let pid = pid as u32;
    let mut exe = fields.iter().find(|(k, _)| k == "exe").map(|(_, v)| v.clone()).unwrap_or_default();
    if exe.is_empty() {
        exe = process_name(pid);
        if exe.is_empty() {
            exe = "-".to_string();
        }
    }
    let age = fs::metadata(path)
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| SystemTime::now().duration_since(t).ok())
        .map(|d| d.as_secs_f64())
        .unwrap_or(0.0);
    let enabled = fields.iter().find(|(k, _)| k == "enabled").map(|(_, v)| v.trim() == "1").unwrap_or(false);
    Some(ProcessState {
        pid,
        current: field_int(&fields, "current"),
        proxies: field_int(&fields, "proxies"),
        enabled,
        rotated_ms: field_int(&fields, "rotated_ms"),
        exe,
        age,
        path: path.to_path_buf(),
    })
}

pub fn read_states(paths: &Paths, prune: bool) -> Vec<ProcessState> {
    let mut out = Vec::new();
    for path in state_files(paths) {
        match read_state(&path) {
            None => {
                if prune {
                    unlink(&path);
                }
            }
            Some(state) => {
                if prune && !state.alive() {
                    unlink(&path);
                    continue;
                }
                out.push(state);
            }
        }
    }
    out
}

pub fn clean_dead(paths: &Paths) -> usize {
    let mut removed = 0;
    for path in state_files(paths) {
        let dead = match read_state(&path) {
            None => true,
            Some(state) => !state.alive(),
        };
        if dead && unlink(&path) {
            removed += 1;
        }
    }
    removed
}

fn unlink(path: &Path) -> bool {
    fs::remove_file(path).is_ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_paths(name: &str) -> Paths {
        let dir = std::env::temp_dir().join(format!("ppp-proc-{}-{}", name, std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(dir.join("state")).unwrap();
        Paths::new(dir)
    }

    #[test]
    fn reads_state_file_fields() {
        let paths = temp_paths("read");
        let file = paths.state_dir.join("4242.txt");
        fs::write(&file, "pid=4242\nexe=app.exe\ncurrent=3\nproxies=10\nenabled=1\nrotated_ms=77\n").unwrap();
        let state = read_state(&file).unwrap();
        assert_eq!((state.pid, state.current, state.proxies, state.enabled, state.rotated_ms), (4242, 3, 10, true, 77));
        assert_eq!(state.exe, "app.exe");
    }

    #[test]
    fn rejects_state_without_pid() {
        let paths = temp_paths("nopid");
        let file = paths.state_dir.join("x.txt");
        fs::write(&file, "exe=a.exe\n").unwrap();
        assert!(read_state(&file).is_none());
    }

    #[test]
    fn clean_dead_removes_gone_and_garbage() {
        let paths = temp_paths("clean");
        fs::write(paths.state_dir.join("a.txt"), "pid=4000000\nexe=x.exe\n").unwrap();
        fs::write(paths.state_dir.join("b.txt"), "garbage\n").unwrap();
        fs::write(paths.state_dir.join("health.json"), "{}").unwrap();
        assert_eq!(clean_dead(&paths), 2);
        assert!(paths.state_dir.join("health.json").exists());
    }

    #[test]
    fn own_process_is_alive() {
        assert!(process_alive(std::process::id()));
        assert!(!process_alive(0));
    }

    #[test]
    fn migrates_legacy_state_files() {
        let paths = temp_paths("legacy");
        fs::write(paths.root.join("state_77.txt"), "pid=77\n").unwrap();
        assert_eq!(migrate_legacy(&paths), 1);
        assert!(paths.state_dir.join("77.txt").exists());
        assert!(!paths.root.join("state_77.txt").exists());
    }
}
