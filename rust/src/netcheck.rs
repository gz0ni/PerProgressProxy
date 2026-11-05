use std::collections::BTreeMap;
use std::io::{self, Read, Write};
use std::net::{TcpStream, ToSocketAddrs};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc::Sender;
use std::sync::Arc;
use std::thread::JoinHandle;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

use crate::paths::Paths;
use crate::specs::Proxy;

pub const ECHO_HOST: &str = "checkip.amazonaws.com";
pub const ECHO_PORT: u16 = 80;
const ECHO_PATH: &str = "/";
const MAX_HEAD: usize = 64 * 1024;

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct HealthEntry {
    #[serde(default)]
    pub ok: bool,
    #[serde(default)]
    pub ms: u64,
    #[serde(default)]
    pub ip: String,
    #[serde(default)]
    pub note: String,
    #[serde(default)]
    pub ts: u64,
}

pub type Health = BTreeMap<String, HealthEntry>;

#[derive(Debug, Clone)]
pub struct TestResult {
    pub idx: usize,
    pub proxy: Proxy,
    pub ok: bool,
    pub exit_ip: String,
    pub ms: u64,
    pub note: String,
}

pub type TestFn = Arc<dyn Fn(usize, &Proxy, f64) -> TestResult + Send + Sync>;

pub fn is_ip(text: &str) -> bool {
    let parts: Vec<&str> = text.split('.').collect();
    parts.len() == 4 && parts.iter().all(|p| !p.is_empty() && p.len() <= 3 && p.bytes().all(|b| b.is_ascii_digit()))
}

fn io_note(error: &io::Error) -> String {
    match error.kind() {
        io::ErrorKind::TimedOut | io::ErrorKind::WouldBlock => "timed out".to_string(),
        io::ErrorKind::ConnectionRefused => "connection refused".to_string(),
        io::ErrorKind::ConnectionReset => "connection reset".to_string(),
        io::ErrorKind::UnexpectedEof => "closed".to_string(),
        _ => {
            let text = error.to_string();
            match text.find(" (os error") {
                Some(pos) => text[..pos].to_string(),
                None => text,
            }
        }
    }
}

struct NetError(String);

impl From<io::Error> for NetError {
    fn from(error: io::Error) -> NetError {
        NetError(io_note(&error))
    }
}

fn net_err<T>(message: impl Into<String>) -> Result<T, NetError> {
    Err(NetError(message.into()))
}

fn secs(value: f64) -> Duration {
    Duration::from_secs_f64(value.max(0.001))
}

fn connect(host: &str, port: u16, timeout: f64) -> io::Result<TcpStream> {
    let addrs = (host, port).to_socket_addrs()?;
    let mut last = io::Error::new(io::ErrorKind::NotFound, "no addresses");
    for addr in addrs {
        match TcpStream::connect_timeout(&addr, secs(timeout)) {
            Ok(stream) => {
                stream.set_read_timeout(Some(secs(timeout)))?;
                stream.set_write_timeout(Some(secs(timeout)))?;
                return Ok(stream);
            }
            Err(e) => last = e,
        }
    }
    Err(last)
}

fn recv_exact(stream: &mut TcpStream, n: usize) -> Result<Vec<u8>, NetError> {
    let mut buf = vec![0u8; n];
    stream.read_exact(&mut buf)?;
    Ok(buf)
}

fn base64(data: &[u8]) -> String {
    const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::new();
    for chunk in data.chunks(3) {
        let b = [chunk[0], *chunk.get(1).unwrap_or(&0), *chunk.get(2).unwrap_or(&0)];
        let n = (u32::from(b[0]) << 16) | (u32::from(b[1]) << 8) | u32::from(b[2]);
        out.push(TABLE[(n >> 18) as usize & 63] as char);
        out.push(TABLE[(n >> 12) as usize & 63] as char);
        out.push(if chunk.len() > 1 { TABLE[(n >> 6) as usize & 63] as char } else { '=' });
        out.push(if chunk.len() > 2 { TABLE[n as usize & 63] as char } else { '=' });
    }
    out
}

fn clip(bytes: &[u8], max: usize) -> &[u8] {
    &bytes[..bytes.len().min(max)]
}

fn open_tunnel(proxy: &Proxy, host: &str, port: u16, timeout: f64) -> Result<TcpStream, NetError> {
    let mut s = connect(&proxy.host, proxy.port, timeout)?;
    match proxy.scheme.as_str() {
        "socks5" => {
            let methods: &[u8] = if proxy.user.is_empty() { &[0] } else { &[0, 2] };
            let mut hello = vec![5, methods.len() as u8];
            hello.extend_from_slice(methods);
            s.write_all(&hello)?;
            let reply = recv_exact(&mut s, 2)?;
            if reply[0] != 5 || reply[1] == 0xFF {
                return net_err("no acceptable auth method");
            }
            if reply[1] == 2 {
                let user = clip(proxy.user.as_bytes(), 255);
                let pass = clip(proxy.pass.as_bytes(), 255);
                let mut auth = vec![1, user.len() as u8];
                auth.extend_from_slice(user);
                auth.push(pass.len() as u8);
                auth.extend_from_slice(pass);
                s.write_all(&auth)?;
                let status = recv_exact(&mut s, 2)?;
                if status[1] != 0 {
                    return net_err("auth rejected");
                }
            }
            let name = host.as_bytes();
            if name.len() > 255 {
                return net_err("host name too long");
            }
            let mut request = vec![5, 1, 0, 3, name.len() as u8];
            request.extend_from_slice(name);
            request.extend_from_slice(&port.to_be_bytes());
            s.write_all(&request)?;
            let head = recv_exact(&mut s, 4)?;
            if head[0] != 5 || head[1] != 0 {
                return net_err(format!("socks5 reply {}", head[1]));
            }
            match head[3] {
                1 => {
                    recv_exact(&mut s, 4)?;
                }
                3 => {
                    let len = recv_exact(&mut s, 1)?[0] as usize;
                    recv_exact(&mut s, len)?;
                }
                4 => {
                    recv_exact(&mut s, 16)?;
                }
                _ => {}
            }
            recv_exact(&mut s, 2)?;
        }
        "socks4" => {
            let mut request = vec![4, 1];
            request.extend_from_slice(&port.to_be_bytes());
            request.extend_from_slice(&[0, 0, 0, 1, 0]);
            request.extend_from_slice(host.as_bytes());
            request.push(0);
            s.write_all(&request)?;
            let reply = recv_exact(&mut s, 8)?;
            if reply[1] != 0x5A {
                return net_err(format!("socks4 reply {}", reply[1]));
            }
        }
        _ => {
            let auth = if proxy.user.is_empty() {
                String::new()
            } else {
                format!("Proxy-Authorization: Basic {}\r\n", base64(format!("{}:{}", proxy.user, proxy.pass).as_bytes()))
            };
            let target = format!("{}:{}", host, port);
            s.write_all(format!("CONNECT {t} HTTP/1.1\r\nHost: {t}\r\n{auth}\r\n", t = target, auth = auth).as_bytes())?;
            let mut head: Vec<u8> = Vec::new();
            let mut byte = [0u8; 1];
            while !head.ends_with(b"\r\n\r\n") {
                if s.read(&mut byte)? == 0 {
                    return net_err("proxy closed on CONNECT");
                }
                head.push(byte[0]);
                if head.len() > MAX_HEAD {
                    return net_err("proxy reply too long");
                }
            }
            let first: Vec<u8> = head.split(|b| *b == b'\n').next().unwrap_or(&[]).to_vec();
            let first_line = latin1(&first).trim_end_matches('\r').to_string();
            if !first_line.contains(" 200") {
                return net_err(first_line);
            }
        }
    }
    Ok(s)
}

fn latin1(bytes: &[u8]) -> String {
    bytes.iter().map(|b| *b as char).collect()
}

fn split_response(body: &[u8]) -> (Vec<u8>, Vec<u8>) {
    match body.windows(4).position(|w| w == b"\r\n\r\n") {
        Some(pos) => (body[..pos].to_vec(), body[pos + 4..].to_vec()),
        None => (body.to_vec(), Vec::new()),
    }
}

fn fetch_via(proxy: &Proxy, timeout: f64) -> Result<String, NetError> {
    let mut s = open_tunnel(proxy, ECHO_HOST, ECHO_PORT, timeout)?;
    s.write_all(
        format!("GET {} HTTP/1.0\r\nHost: {}\r\nUser-Agent: ppp\r\nConnection: close\r\n\r\n", ECHO_PATH, ECHO_HOST).as_bytes(),
    )?;
    let mut body = Vec::new();
    s.read_to_end(&mut body)?;
    let (head, payload) = split_response(&body);
    let status_line = head.split(|b| *b == b'\n').next().unwrap_or(&[]).to_vec();
    let status = latin1(&status_line).trim_end_matches('\r').to_string();
    if !status.contains(" 200") {
        return net_err(if status.is_empty() { "empty reply".to_string() } else { status });
    }
    Ok(latin1(&payload).trim().to_string())
}

pub fn direct_ip(timeout: f64) -> Option<String> {
    let mut s = connect(ECHO_HOST, ECHO_PORT, timeout).ok()?;
    s.write_all(format!("GET / HTTP/1.0\r\nHost: {}\r\nConnection: close\r\n\r\n", ECHO_HOST).as_bytes()).ok()?;
    let mut data = Vec::new();
    s.read_to_end(&mut data).ok()?;
    let (_head, payload) = split_response(&data);
    let body = latin1(&payload).trim().to_string();
    if is_ip(&body) {
        Some(body)
    } else {
        let cut: String = body.chars().take(40).collect();
        if cut.is_empty() { None } else { Some(cut) }
    }
}

pub fn test_one(idx: usize, proxy: &Proxy, timeout: f64) -> TestResult {
    let started = Instant::now();
    let elapsed = || started.elapsed().as_millis() as u64;
    match fetch_via(proxy, timeout) {
        Ok(ip) => {
            if !is_ip(&ip) {
                let cut: String = ip.chars().take(32).collect();
                return TestResult { idx, proxy: proxy.clone(), ok: false, exit_ip: cut, ms: elapsed(), note: "bad answer".to_string() };
            }
            TestResult { idx, proxy: proxy.clone(), ok: true, exit_ip: ip, ms: elapsed(), note: String::new() }
        }
        Err(NetError(note)) => {
            let note = if note.is_empty() { "failed".to_string() } else { note };
            TestResult { idx, proxy: proxy.clone(), ok: false, exit_ip: String::new(), ms: elapsed(), note }
        }
    }
}

pub fn real_tester() -> TestFn {
    Arc::new(|idx, proxy, timeout| test_one(idx, proxy, timeout))
}

pub fn load_health(paths: &Paths) -> Health {
    std::fs::read(paths.health())
        .ok()
        .and_then(|bytes| serde_json::from_slice(&bytes).ok())
        .unwrap_or_default()
}

pub fn save_health(paths: &Paths, results: &[TestResult]) {
    let mut data = load_health(paths);
    let now = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
    for r in results {
        data.insert(
            r.proxy.label(),
            HealthEntry { ok: r.ok, ms: r.ms, ip: r.exit_ip.clone(), note: r.note.clone(), ts: now },
        );
    }
    let _ = std::fs::create_dir_all(&paths.state_dir);
    let mut buffer = Vec::new();
    let formatter = serde_json::ser::PrettyFormatter::with_indent(b" ");
    let mut serializer = serde_json::Serializer::with_formatter(&mut buffer, formatter);
    if data.serialize(&mut serializer).is_ok() {
        let _ = std::fs::write(paths.health(), buffer);
    }
}

pub fn spawn_pool(
    entries: Vec<(usize, Proxy)>,
    concurrency: usize,
    timeout: f64,
    tester: TestFn,
    tx: Sender<TestResult>,
) -> Vec<JoinHandle<()>> {
    let entries = Arc::new(entries);
    let next = Arc::new(AtomicUsize::new(0));
    let workers = concurrency.max(1).min(entries.len().max(1));
    (0..workers)
        .map(|_| {
            let entries = Arc::clone(&entries);
            let next = Arc::clone(&next);
            let tester = Arc::clone(&tester);
            let tx = tx.clone();
            std::thread::spawn(move || loop {
                let i = next.fetch_add(1, Ordering::SeqCst);
                let Some((idx, proxy)) = entries.get(i) else {
                    break;
                };
                let result = tester(*idx, proxy, timeout);
                if tx.send(result).is_err() {
                    break;
                }
            })
        })
        .collect()
}

pub fn test_all(entries: Vec<(usize, Proxy)>, concurrency: usize, timeout: f64, tester: TestFn) -> Vec<TestResult> {
    let (tx, rx) = std::sync::mpsc::channel();
    let handles = spawn_pool(entries, concurrency, timeout, tester, tx);
    let mut results: Vec<TestResult> = rx.iter().collect();
    for handle in handles {
        let _ = handle.join();
    }
    results.sort_by_key(|r| r.idx);
    results
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::specs::parse_spec;
    use std::net::TcpListener;

    #[test]
    fn ip_shapes() {
        assert!(is_ip("1.2.3.4"));
        assert!(is_ip("255.255.255.255"));
        assert!(!is_ip("1.2.3"));
        assert!(!is_ip("a.b.c.d"));
        assert!(!is_ip("1.2.3.4.5"));
        assert!(!is_ip("1234.1.1.1"));
    }

    #[test]
    fn base64_matches_reference() {
        assert_eq!(base64(b"user:pass"), "dXNlcjpwYXNz");
        assert_eq!(base64(b"a"), "YQ==");
        assert_eq!(base64(b"ab"), "YWI=");
        assert_eq!(base64(b""), "");
    }

    #[test]
    fn pool_returns_sorted_results() {
        let entries: Vec<(usize, Proxy)> =
            (0..10).map(|i| (i, parse_spec(&format!("10.0.0.{}:8080", i + 1), "auto").unwrap())).collect();
        let tester: TestFn = Arc::new(|idx, proxy, _| TestResult {
            idx,
            proxy: proxy.clone(),
            ok: true,
            exit_ip: "8.8.8.8".to_string(),
            ms: 1,
            note: String::new(),
        });
        let results = test_all(entries, 4, 1.0, tester);
        assert_eq!(results.iter().map(|r| r.idx).collect::<Vec<_>>(), (0..10).collect::<Vec<_>>());
    }

    #[test]
    fn health_roundtrip() {
        let dir = std::env::temp_dir().join(format!("ppp-health-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let paths = Paths::new(dir);
        let proxy = parse_spec("1.2.3.4:8080", "auto").unwrap();
        let result = TestResult { idx: 0, proxy: proxy.clone(), ok: true, exit_ip: "9.9.9.9".to_string(), ms: 42, note: String::new() };
        save_health(&paths, &[result]);
        let loaded = load_health(&paths);
        let entry = loaded.get(&proxy.label()).unwrap();
        assert!(entry.ok);
        assert_eq!((entry.ms, entry.ip.as_str()), (42, "9.9.9.9"));
    }

    fn serve_once(handler: impl FnOnce(TcpStream) + Send + 'static) -> u16 {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        std::thread::spawn(move || {
            if let Ok((stream, _)) = listener.accept() {
                handler(stream);
            }
        });
        port
    }

    #[test]
    fn socks5_tunnel_handshake() {
        let port = serve_once(|mut s| {
            let mut hello = [0u8; 3];
            s.read_exact(&mut hello).unwrap();
            assert_eq!(hello, [5, 1, 0]);
            s.write_all(&[5, 0]).unwrap();
            let mut head = [0u8; 5];
            s.read_exact(&mut head).unwrap();
            assert_eq!(&head[..4], &[5, 1, 0, 3]);
            let mut rest = vec![0u8; head[4] as usize + 2];
            s.read_exact(&mut rest).unwrap();
            s.write_all(&[5, 0, 0, 1, 0, 0, 0, 0, 0, 0]).unwrap();
            let mut request = [0u8; 512];
            let _ = s.read(&mut request).unwrap();
            s.write_all(b"HTTP/1.0 200 OK\r\n\r\n10.9.9.9\n").unwrap();
        });
        let proxy = parse_spec(&format!("socks5://127.0.0.1:{}", port), "auto").unwrap();
        let result = test_one(0, &proxy, 5.0);
        assert!(result.ok, "{}", result.note);
        assert_eq!(result.exit_ip, "10.9.9.9");
    }

    #[test]
    fn http_connect_rejection_is_reported() {
        let port = serve_once(|mut s| {
            let mut buf = [0u8; 512];
            let _ = s.read(&mut buf).unwrap();
            s.write_all(b"HTTP/1.1 407 Proxy Authentication Required\r\n\r\n").unwrap();
        });
        let proxy = parse_spec(&format!("http://127.0.0.1:{}", port), "auto").unwrap();
        let result = test_one(0, &proxy, 5.0);
        assert!(!result.ok);
        assert!(result.note.contains("407"), "{}", result.note);
    }
}
