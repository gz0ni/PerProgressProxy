use std::fmt;

pub const SCHEMES: [&str; 3] = ["socks5", "socks4", "http"];
const SOCKS_PORTS: [u32; 5] = [1080, 1081, 1082, 9050, 9150];

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SpecError(pub String);

impl fmt::Display for SpecError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for SpecError {}

fn err<T>(message: impl Into<String>) -> Result<T, SpecError> {
    Err(SpecError(message.into()))
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Proxy {
    pub scheme: String,
    pub host: String,
    pub port: u16,
    pub user: String,
    pub pass: String,
}

impl Proxy {
    pub fn label(&self) -> String {
        if self.user.is_empty() {
            format!("{}://{}:{}", self.scheme, self.host, self.port)
        } else {
            format!("{}://{}:{}@{}:{}", self.scheme, self.user, self.pass, self.host, self.port)
        }
    }

    pub fn display(&self) -> String {
        if self.user.is_empty() {
            self.label()
        } else {
            format!("{}://{}:***@{}:{}", self.scheme, self.user, self.host, self.port)
        }
    }
}

fn is_comment_start(c: char) -> bool {
    c == '#' || c == ';'
}

fn ascii_digits(text: &str) -> bool {
    !text.is_empty() && text.bytes().all(|b| b.is_ascii_digit())
}

pub fn parse_spec(text: &str, default_scheme: &str) -> Result<Proxy, SpecError> {
    let raw = text.trim();
    match raw.chars().next() {
        None => return err("empty or comment"),
        Some(c) if is_comment_start(c) => return err("empty or comment"),
        _ => {}
    }

    let lower = raw.to_lowercase();
    let mut scheme = String::new();
    let mut rest = raw;
    if let Some(pos) = raw.find("://") {
        scheme = raw[..pos].to_lowercase();
        rest = &raw[pos + 3..];
        if !SCHEMES.contains(&scheme.as_str()) {
            return err(format!("unknown scheme '{}'", scheme));
        }
    } else if lower.starts_with("socks5:") || lower.starts_with("socks4:") || lower.starts_with("http:") {
        let pos = raw.find(':').unwrap_or(0);
        scheme = raw[..pos].to_lowercase();
        rest = &raw[pos + 1..];
    }

    let mut user = String::new();
    let mut pass = String::new();
    if let Some(pos) = rest.rfind('@') {
        let cred = &rest[..pos];
        rest = &rest[pos + 1..];
        match cred.split_once(':') {
            Some((u, p)) => {
                user = u.to_string();
                pass = p.to_string();
            }
            None => user = cred.to_string(),
        }
    }

    let host;
    let port;
    if let Some(inner) = rest.strip_prefix('[') {
        let (h, tail) = match inner.split_once(']') {
            Some((h, tail)) => (h, tail),
            None => (inner, ""),
        };
        host = h.to_string();
        port = tail.trim_start_matches(':').to_string();
    } else {
        let colons = rest.matches(':').count();
        let mut parts = rest.splitn(4, ':');
        let first = parts.next().unwrap_or("");
        let second = parts.next().unwrap_or("");
        let third = parts.next();
        let fourth = parts.next();
        let credential_form = colons >= 3
            && !first.is_empty()
            && second.len() <= 5
            && ascii_digits(second)
            && matches!(fourth, Some(p) if !p.is_empty());
        if credential_form {
            host = first.to_string();
            port = second.to_string();
            if user.is_empty() {
                user = third.unwrap_or("").to_string();
                pass = fourth.unwrap_or("").to_string();
            }
        } else if colons == 1 {
            host = first.to_string();
            port = second.to_string();
        } else {
            return err("expected host:port or host:port:user:pass");
        }
    }

    let host = host.trim().to_string();
    let port = port.trim().to_string();
    if host.is_empty() || !ascii_digits(&port) {
        return err("bad host/port");
    }
    let port_n: u32 = match port.parse() {
        Ok(n) if n > 0 && n < 65536 => n,
        _ => return err("port out of range"),
    };

    if scheme.is_empty() {
        if SCHEMES.contains(&default_scheme) {
            scheme = default_scheme.to_string();
        } else if SOCKS_PORTS.contains(&port_n) {
            scheme = "socks5".to_string();
        } else {
            scheme = "http".to_string();
        }
    }
    if scheme == "socks4" && (!user.is_empty() || !pass.is_empty()) {
        return err("socks4 cannot carry a username/password");
    }
    Ok(Proxy { scheme, host, port: port_n as u16, user, pass })
}

pub fn split_tokens(raw: &str) -> Vec<String> {
    let mut out = Vec::new();
    for line in raw.split(|c| c == '\r' || c == '\n') {
        let stripped = line.trim();
        match stripped.chars().next() {
            None => continue,
            Some(c) if is_comment_start(c) => continue,
            _ => {}
        }
        for token in stripped.split(|c: char| c.is_whitespace() || c == ',' || c == ';') {
            let token = token.trim().trim_matches(|c| c == '"' || c == '\'');
            if token.is_empty() || token.starts_with('#') {
                continue;
            }
            out.push(token.to_string());
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(text: &str) -> Result<Proxy, SpecError> {
        parse_spec(text, "auto")
    }

    #[test]
    fn webshare_three_colon_form() {
        let p = parse("45.38.107.97:6014:mvcwtmiw:g36umzrtdlw0").unwrap();
        assert_eq!(
            (p.host.as_str(), p.port, p.user.as_str(), p.pass.as_str()),
            ("45.38.107.97", 6014, "mvcwtmiw", "g36umzrtdlw0")
        );
        assert_eq!(p.scheme, "http");
    }

    #[test]
    fn password_with_colon() {
        let p = parse("1.2.3.4:8080:user:pa:ss").unwrap();
        assert_eq!(p.user, "user");
        assert_eq!(p.pass, "pa:ss");
    }

    #[test]
    fn socks_url_with_credentials() {
        let p = parse("socks5://bob:secret@10.0.0.1:1080").unwrap();
        assert_eq!(
            (p.scheme.as_str(), p.host.as_str(), p.port, p.user.as_str(), p.pass.as_str()),
            ("socks5", "10.0.0.1", 1080, "bob", "secret")
        );
    }

    #[test]
    fn host_port_guesses_scheme_like_dll() {
        assert_eq!(parse("10.0.0.1:1080").unwrap().scheme, "socks5");
        assert_eq!(parse("10.0.0.1:9999").unwrap().scheme, "http");
    }

    #[test]
    fn default_scheme_applies() {
        assert_eq!(parse_spec("10.0.0.1:9999", "socks4").unwrap().scheme, "socks4");
    }

    #[test]
    fn bracketed_ipv6() {
        let p = parse("socks5://[::1]:1080").unwrap();
        assert_eq!((p.host.as_str(), p.port), ("::1", 1080));
    }

    #[test]
    fn socks4_rejects_credentials() {
        assert!(parse("socks4://user:pass@1.2.3.4:1080").is_err());
    }

    #[test]
    fn rejects_garbage() {
        for bad in ["", "# comment", "1.2.3.4", "1.2.3.4:notaport", "ftp://1.2.3.4:21", "1.2.3.4:70000", "a:b:c:d"] {
            assert!(parse(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn display_masks_password() {
        let p = parse("socks5://bob:secret@10.0.0.1:1080").unwrap();
        assert!(!p.display().contains("secret"));
        assert!(p.label().contains("secret"));
    }

    #[test]
    fn scheme_prefix_without_slashes() {
        let p = parse("socks5:1.2.3.4:9999").unwrap();
        assert_eq!((p.scheme.as_str(), p.port), ("socks5", 9999));
    }

    #[test]
    fn split_tokens_mixed_file() {
        let raw = "# header\n1.2.3.4:8080\n\n5.6.7.8:1080:u:p  9.9.9.9:80,other:3128\n";
        assert_eq!(split_tokens(raw), vec!["1.2.3.4:8080", "5.6.7.8:1080:u:p", "9.9.9.9:80", "other:3128"]);
    }
}
