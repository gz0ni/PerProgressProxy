use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::time::Duration;

use rusqlite::Connection;

use crate::paths::Paths;
use crate::specs::{parse_spec, split_tokens, Proxy};

pub type Config = HashMap<String, String>;

#[derive(Debug, Clone)]
pub struct ProxyLine {
    pub text: String,
    pub proxy: Option<Proxy>,
    pub error: String,
}

#[derive(Debug, Clone)]
pub struct ProxyRow {
    pub index: usize,
    pub text: String,
    pub proxy: Proxy,
}

#[derive(Debug, Clone, Default)]
pub struct SyncResult {
    pub files: usize,
    pub added: usize,
    pub skipped: usize,
    pub total: usize,
}

pub fn read_text_lossy(path: &Path) -> Option<String> {
    fs::read(path).ok().map(|bytes| String::from_utf8_lossy(&bytes).into_owned())
}

fn is_comment_line(line: &str) -> bool {
    matches!(line.chars().next(), None | Some('#') | Some(';'))
}

#[derive(Clone, Debug)]
pub struct Store {
    pub paths: Paths,
}

impl Store {
    pub fn new(paths: Paths) -> Store {
        Store { paths }
    }

    pub fn read_config(&self) -> Config {
        let mut out = Config::new();
        let Some(text) = read_text_lossy(&self.paths.config) else {
            return out;
        };
        for line in text.lines() {
            let line = line.trim();
            if is_comment_line(line) || line.starts_with('=') {
                continue;
            }
            if let Some((key, val)) = line.split_once('=') {
                out.insert(key.trim().to_lowercase(), val.trim().to_string());
            }
        }
        out
    }

    pub fn set_config(&self, pairs: &[(&str, String)]) -> std::io::Result<()> {
        let text = read_text_lossy(&self.paths.config).unwrap_or_default();
        let mut lines: Vec<String> = text.lines().map(str::to_string).collect();
        for (key, val) in pairs {
            let wanted = key.to_lowercase();
            let hit = lines.iter().position(|line| {
                line.trim_start()
                    .to_lowercase()
                    .strip_prefix(&wanted)
                    .map(|tail| tail.trim_start().starts_with('='))
                    .unwrap_or(false)
            });
            let replacement = format!("{} = {}", key, val);
            match hit {
                Some(i) => lines[i] = replacement,
                None => lines.push(replacement),
            }
        }
        fs::write(&self.paths.config, lines.join("\n") + "\n")
    }

    pub fn default_scheme(&self) -> String {
        self.read_config().get("default_scheme").cloned().unwrap_or_else(|| "auto".to_string())
    }

    fn open_db(&self) -> rusqlite::Result<Connection> {
        let conn = Connection::open(&self.paths.db)?;
        conn.busy_timeout(Duration::from_secs(3))?;
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS proxies (id INTEGER PRIMARY KEY AUTOINCREMENT, text TEXT NOT NULL);
             CREATE TABLE IF NOT EXISTS meta (key TEXT PRIMARY KEY, value TEXT NOT NULL);",
        )?;
        Ok(conn)
    }

    fn meta_get(conn: &Connection, key: &str) -> Option<String> {
        conn.query_row("SELECT value FROM meta WHERE key = ?1", [key], |row| row.get(0)).ok()
    }

    fn db_texts(conn: &Connection) -> rusqlite::Result<Vec<String>> {
        let mut stmt = conn.prepare("SELECT text FROM proxies ORDER BY id")?;
        let rows = stmt.query_map([], |row| row.get::<_, String>(0))?;
        rows.collect()
    }

    fn replace_rows(conn: &Connection, texts: &[String]) -> rusqlite::Result<()> {
        let tx = conn.unchecked_transaction()?;
        tx.execute("DELETE FROM proxies", [])?;
        {
            let mut stmt = tx.prepare("INSERT INTO proxies (text) VALUES (?1)")?;
            for text in texts {
                stmt.execute([text])?;
            }
        }
        tx.commit()
    }

    fn legacy_texts(&self) -> Vec<String> {
        read_text_lossy(&self.paths.proxies)
            .map(|text| text.lines().map(str::trim).filter(|l| !is_comment_line(l)).map(str::to_string).collect())
            .unwrap_or_default()
    }

    fn migrate_legacy(&self, conn: &Connection) {
        let legacy = self.legacy_texts();
        if Self::replace_rows(conn, &legacy).is_err() {
            return;
        }
        let stored = Self::db_texts(conn).map(|rows| rows.len()).unwrap_or(usize::MAX);
        if stored == legacy.len()
            && conn.execute("INSERT OR REPLACE INTO meta (key, value) VALUES ('migrated', '1')", []).is_ok()
        {
            let _ = fs::remove_file(&self.paths.proxies);
        }
    }

    fn load_texts(&self) -> Vec<String> {
        let Ok(conn) = self.open_db() else {
            return Vec::new();
        };
        if Self::meta_get(&conn, "migrated").is_none() {
            self.migrate_legacy(&conn);
        }
        Self::db_texts(&conn).unwrap_or_default()
    }

    pub fn db_row_count(&self) -> Option<usize> {
        let conn = self.open_db().ok()?;
        conn.query_row("SELECT COUNT(*) FROM proxies", [], |row| row.get::<_, i64>(0)).ok().map(|n| n as usize)
    }

    pub fn read_proxy_lines(&self) -> Vec<ProxyLine> {
        let default = self.default_scheme();
        let mut rows = Vec::new();
        for line in self.load_texts() {
            let s = line.trim();
            if is_comment_line(s) {
                continue;
            }
            match parse_spec(s, &default) {
                Ok(proxy) => rows.push(ProxyLine { text: s.to_string(), proxy: Some(proxy), error: String::new() }),
                Err(e) => rows.push(ProxyLine { text: s.to_string(), proxy: None, error: e.to_string() }),
            }
        }
        rows
    }

    pub fn write_proxy_lines(&self, texts: &[String]) -> std::io::Result<()> {
        let conn = self.open_db().map_err(std::io::Error::other)?;
        Self::replace_rows(&conn, texts).map_err(std::io::Error::other)
    }

    pub fn proxies(&self) -> Vec<ProxyRow> {
        self.read_proxy_lines()
            .into_iter()
            .enumerate()
            .filter_map(|(index, line)| line.proxy.map(|proxy| ProxyRow { index, text: line.text, proxy }))
            .collect()
    }

    pub fn current_index(&self) -> i64 {
        self.read_config().get("current").and_then(|v| v.trim().parse().ok()).unwrap_or(0)
    }

    fn existing_keys(&self, default: &str) -> (Vec<String>, HashSet<String>) {
        let texts: Vec<String> = self.read_proxy_lines().into_iter().map(|l| l.text.trim().to_string()).collect();
        let keys = texts
            .iter()
            .map(|text| match parse_spec(text, default) {
                Ok(p) => p.label(),
                Err(_) => text.clone(),
            })
            .collect();
        (texts, keys)
    }

    pub fn add_specs(&self, specs: &[String]) -> (usize, Vec<String>) {
        let default = self.default_scheme();
        let (mut texts, mut keys) = self.existing_keys(&default);
        let mut added = 0;
        let mut notes = Vec::new();
        for spec in specs {
            let parsed = match parse_spec(spec, &default) {
                Ok(p) => p,
                Err(e) => {
                    notes.push(format!("skip '{}': {}", spec, e));
                    continue;
                }
            };
            if keys.contains(&parsed.label()) {
                notes.push(format!("dup  {}", parsed.display()));
                continue;
            }
            texts.push(spec.trim().to_string());
            keys.insert(parsed.label());
            notes.push(format!("add  {}", parsed.display()));
            added += 1;
        }
        if added > 0 {
            let _ = self.write_proxy_lines(&texts);
        }
        (added, notes)
    }

    pub fn import_file(&self, src: &Path) -> std::io::Result<(usize, usize, usize)> {
        let bytes = fs::read(src)?;
        Ok(self.import_text(&String::from_utf8_lossy(&bytes)))
    }

    pub fn import_text(&self, raw: &str) -> (usize, usize, usize) {
        let default = self.default_scheme();
        let (mut texts, mut keys) = self.existing_keys(&default);
        let mut added = 0;
        let mut skipped = 0;
        for token in split_tokens(raw) {
            let Ok(parsed) = parse_spec(&token, &default) else {
                skipped += 1;
                continue;
            };
            if keys.contains(&parsed.label()) {
                skipped += 1;
                continue;
            }
            keys.insert(parsed.label());
            texts.push(token);
            added += 1;
        }
        if added > 0 {
            let _ = self.write_proxy_lines(&texts);
        }
        (added, skipped, texts.len())
    }

    pub fn proxy_sources(&self) -> Vec<PathBuf> {
        let Ok(entries) = fs::read_dir(&self.paths.proxy_dir) else {
            return Vec::new();
        };
        let mut files: Vec<PathBuf> = entries
            .filter_map(Result::ok)
            .map(|e| e.path())
            .filter(|p| {
                p.is_file()
                    && p.extension()
                        .and_then(|e| e.to_str())
                        .map(|e| matches!(e.to_lowercase().as_str(), "txt" | "csv" | "list"))
                        .unwrap_or(false)
            })
            .collect();
        files.sort();
        files
    }

    pub fn sync_proxy_dir(&self) -> SyncResult {
        let files = self.proxy_sources();
        let default = self.default_scheme();
        let (mut texts, mut keys) = self.existing_keys(&default);
        let mut added = 0;
        let mut skipped = 0;
        for src in &files {
            let raw = read_text_lossy(src).unwrap_or_default();
            for token in split_tokens(&raw) {
                let Ok(parsed) = parse_spec(&token, &default) else {
                    skipped += 1;
                    continue;
                };
                if keys.contains(&parsed.label()) {
                    continue;
                }
                keys.insert(parsed.label());
                texts.push(token);
                added += 1;
            }
        }
        if added > 0 {
            let _ = self.write_proxy_lines(&texts);
        }
        SyncResult { files: files.len(), added, skipped, total: texts.len() }
    }

    pub fn remove_indices(&self, indices: &[i64]) -> usize {
        let rows = self.read_proxy_lines();
        let before = rows.len();
        let keep: Vec<String> = rows
            .into_iter()
            .enumerate()
            .filter(|(i, _)| !indices.contains(&(*i as i64)))
            .map(|(_, line)| line.text)
            .collect();
        let _ = self.write_proxy_lines(&keep);
        before - keep.len()
    }
}

#[cfg(test)]
pub mod tests {
    use super::*;

    pub fn temp_store(name: &str) -> Store {
        let dir = std::env::temp_dir().join(format!("ppp-test-{}-{}", name, std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(dir.join("proxy")).unwrap();
        Store::new(Paths::new(dir))
    }

    #[test]
    fn sync_imports_and_dedups() {
        let store = temp_store("sync");
        fs::write(store.paths.proxy_dir.join("a.txt"), "1.2.3.4:8080\n5.6.7.8:1080:u:p\n").unwrap();
        assert_eq!(store.sync_proxy_dir().added, 2);
        assert_eq!(store.sync_proxy_dir().added, 0);
        assert_eq!(store.proxies().len(), 2);
    }

    #[test]
    fn add_specs_dedups() {
        let store = temp_store("add");
        let (added, _) = store.add_specs(&["1.2.3.4:8080".to_string(), "http://1.2.3.4:8080".to_string()]);
        assert_eq!(added, 1);
    }

    #[test]
    fn set_config_updates_in_place_and_appends() {
        let store = temp_store("cfg");
        fs::write(&store.paths.config, "# note\nEnabled = 0\ncurrent = 2\n").unwrap();
        store.set_config(&[("enabled", "1".to_string()), ("rotate_sec", "30".to_string())]).unwrap();
        let text = fs::read_to_string(&store.paths.config).unwrap();
        assert_eq!(text, "# note\nenabled = 1\ncurrent = 2\nrotate_sec = 30\n");
        assert_eq!(store.read_config().get("rotate_sec").map(String::as_str), Some("30"));
    }

    #[test]
    fn remove_indices_drops_lines() {
        let store = temp_store("rm");
        store.add_specs(&["1.1.1.1:80".to_string(), "2.2.2.2:80".to_string(), "3.3.3.3:80".to_string()]);
        assert_eq!(store.remove_indices(&[1]), 1);
        let left: Vec<String> = store.proxies().into_iter().map(|r| r.text).collect();
        assert_eq!(left, vec!["1.1.1.1:80", "3.3.3.3:80"]);
    }

    #[test]
    fn database_is_created_on_first_use_and_no_txt_is_written() {
        let store = temp_store("dbcreate");
        assert!(!store.paths.db.exists());
        store.add_specs(&["1.1.1.1:80".to_string()]);
        assert!(store.paths.db.exists());
        assert_eq!(store.db_row_count(), Some(1));
        assert!(!store.paths.proxies.exists());
    }

    #[test]
    fn deleted_database_is_recreated_empty() {
        let store = temp_store("dbdelete");
        store.add_specs(&["1.1.1.1:80".to_string(), "2.2.2.2:80".to_string()]);
        fs::remove_file(&store.paths.db).unwrap();
        assert!(store.proxies().is_empty());
        assert!(store.paths.db.exists());
        store.add_specs(&["3.3.3.3:80".to_string()]);
        assert_eq!(store.proxies().len(), 1);
    }

    #[test]
    fn legacy_txt_is_imported_once_and_removed() {
        let store = temp_store("dbmigrate");
        fs::write(&store.paths.proxies, "# header
1.1.1.1:80

2.2.2.2:1080:u:p
").unwrap();
        let texts: Vec<String> = store.proxies().into_iter().map(|r| r.text).collect();
        assert_eq!(texts, vec!["1.1.1.1:80", "2.2.2.2:1080:u:p"]);
        assert!(!store.paths.proxies.exists());
        fs::write(&store.paths.proxies, "9.9.9.9:80
").unwrap();
        assert_eq!(store.proxies().len(), 2);
        assert!(store.paths.proxies.exists());
    }

    #[test]
    fn order_survives_removals() {
        let store = temp_store("dborder");
        store.add_specs(&["1.1.1.1:80".to_string(), "2.2.2.2:80".to_string(), "3.3.3.3:80".to_string()]);
        store.remove_indices(&[0]);
        store.add_specs(&["4.4.4.4:80".to_string()]);
        let texts: Vec<String> = store.proxies().into_iter().map(|r| r.text).collect();
        assert_eq!(texts, vec!["2.2.2.2:80", "3.3.3.3:80", "4.4.4.4:80"]);
    }

    #[test]
    fn import_text_counts_skipped() {
        let store = temp_store("imp");
        let (added, skipped, total) = store.import_text("1.1.1.1:80 garbage 1.1.1.1:80 2.2.2.2:80");
        assert_eq!((added, skipped, total), (2, 2, 2));
    }
}
