use std::path::{Path, PathBuf};

pub const IS_WINDOWS: bool = cfg!(windows);

#[derive(Clone, Debug)]
pub struct Paths {
    pub root: PathBuf,
    pub config: PathBuf,
    pub proxies: PathBuf,
    pub db: PathBuf,
    pub proxy_dir: PathBuf,
    pub state_dir: PathBuf,
    pub logs_dir: PathBuf,
    pub bin_dir: PathBuf,
}

impl Paths {
    pub fn new(root: PathBuf) -> Paths {
        Paths {
            config: root.join("config.ini"),
            proxies: root.join("proxies.txt"),
            db: root.join("proxies.db"),
            proxy_dir: root.join("proxy"),
            state_dir: root.join("state"),
            logs_dir: root.join("logs"),
            bin_dir: root.join("bin"),
            root,
        }
    }

    pub fn discover() -> Paths {
        if let Some(dir) = std::env::var_os("PPP_ROOT") {
            return Paths::new(PathBuf::from(dir));
        }
        let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
        if cwd.join("config.ini").exists() {
            return Paths::new(cwd);
        }
        if let Ok(exe) = std::env::current_exe() {
            let mut dir = exe.parent().map(Path::to_path_buf);
            while let Some(candidate) = dir {
                if candidate.join("config.ini").exists() {
                    return Paths::new(candidate);
                }
                dir = candidate.parent().map(Path::to_path_buf);
            }
        }
        Paths::new(cwd)
    }

    pub fn binary(&self, name: &str) -> PathBuf {
        let candidate = self.bin_dir.join(name);
        if candidate.exists() {
            candidate
        } else {
            self.root.join(name)
        }
    }

    pub fn dll(&self) -> PathBuf {
        self.binary("proxyinject.dll")
    }

    pub fn inject(&self) -> PathBuf {
        self.binary("inject.exe")
    }

    pub fn health(&self) -> PathBuf {
        self.state_dir.join("health.json")
    }
}
