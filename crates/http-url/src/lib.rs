/// A parsed `http://` or `https://` URL.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Url {
    pub tls: bool,
    pub host: String,
    pub port: u16,
    /// Path and query, starting with `/`.
    pub path: String,
}

impl Url {
    pub fn parse(s: &str) -> Result<Url, HttpError> {
        let bad = |why: &str| HttpError::BadUrl(why.to_string());
        if s.len() > 16 * 1024 {
            return Err(bad("URL exceeds 16 KiB"));
        }
        let s = s.trim();
        if s.chars().any(|c| c.is_control() || c == ' ') {
            return Err(bad("contains spaces or control characters"));
        }
        let (tls, rest) = if let Some(r) = s.strip_prefix("https://") {
            (true, r)
        } else if let Some(r) = s.strip_prefix("http://") {
            (false, r)
        } else {
            return Err(bad("only http:// and https:// are supported"));
        };
        let rest = rest.split('#').next().unwrap_or_default();
        let (authority, path) = match rest.find(['/', '?']) {
            Some(i) => (rest.get(..i).unwrap_or_default(), rest.get(i..).unwrap_or_default()),
            None => (rest, ""),
        };
        let path = if path.starts_with('/') { path.to_string() } else { format!("/{path}") };
        if authority.contains('@') {
            return Err(bad("user names and passwords in URLs are not supported"));
        }
        let (host, port) = if let Some(v6) = authority.strip_prefix('[') {
            let end = v6.find(']').ok_or_else(|| bad("unterminated IPv6 address"))?;
            let host = v6.get(..end).unwrap_or_default();
            let after = v6.get(end + 1..).unwrap_or_default();
            (host.to_string(), after.strip_prefix(':'))
        } else {
            match authority.rsplit_once(':') {
                Some((h, p)) => (h.to_string(), Some(p)),
                None => (authority.to_string(), None),
            }
        };
        let port = match port {
            Some(p) => p.parse::<u16>().ok().filter(|p| *p != 0).ok_or_else(|| bad("bad port"))?,
            None if tls => 443,
            None => 80,
        };
        if host.is_empty() || !host.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '.' | ':' | '_')) {
            return Err(bad("bad host name"));
        }
        Ok(Url { tls, host: host.to_ascii_lowercase(), port, path })
    }

    /// `Location` of a redirect, resolved against this URL.
    pub fn join(&self, location: &str) -> Result<Url, HttpError> {
        let l = location.trim();
        if l.starts_with("http://") || l.starts_with("https://") {
            let url = Url::parse(l)?;
            if self.tls && !url.tls {
                return Err(HttpError::BadUrl("redirect from https to http refused".into()));
            }
            return Ok(url);
        }
        // another scheme (`ftp:`, `file:`, `data:`…)
        if let Some((scheme, _)) = l.split_once(':')
            && !scheme.is_empty()
            && scheme.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '-' | '.'))
        {
            return Err(HttpError::BadUrl(format!("redirect to an unsupported location `{scheme}:`")));
        }
        let scheme = if self.tls { "https" } else { "http" };
        let host = if self.host.contains(':') { format!("[{}]", self.host) } else { self.host.clone() };
        let base = format!("{scheme}://{host}:{}", self.port);
        if let Some(rest) = l.strip_prefix("//") {
            return Url::parse(&format!("{scheme}://{rest}"));
        }
        if l.starts_with('/') {
            return Url::parse(&format!("{base}{l}"));
        }
        // relative to the current directory
        let dir = self.path.split('?').next().unwrap_or("/");
        let dir = dir.rsplit_once('/').map_or("", |(d, _)| d);
        Url::parse(&format!("{base}{dir}/{l}"))
    }

    fn host_header(&self) -> String {
        let host = if self.host.contains(':') { format!("[{}]", self.host) } else { self.host.clone() };
        let default = if self.tls { 443 } else { 80 };
        if self.port == default { host } else { format!("{host}:{}", self.port) }
    }

    /// The URL without its query (for messages: a query may hold a signed token).
    pub fn display(&self) -> String {
        let scheme = if self.tls { "https" } else { "http" };
        format!("{scheme}://{}{}", self.host_header(), self.path.split('?').next().unwrap_or("/"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_oversized_and_control_character_urls() {
        assert!(Url::parse(&format!("https://example.com/{}", "x".repeat(16 * 1024))).is_err());
        assert!(Url::parse("https://example.com/x?secret=abc\r\nInjected: 1").is_err());
    }

    #[test]
    fn parses_urls() {
        let u = Url::parse("https://cdn.example.com/models/sam3/model.safetensors?sig=1#x").unwrap();
        assert_eq!((u.tls, u.host.as_str(), u.port, u.path.as_str()), (true, "cdn.example.com", 443, "/models/sam3/model.safetensors?sig=1"));
        assert_eq!(u.display(), "https://cdn.example.com/models/sam3/model.safetensors");
        let u = Url::parse("http://127.0.0.1:8080").unwrap();
        assert_eq!((u.tls, u.port, u.path.as_str()), (false, 8080, "/"));
        let u = Url::parse("http://[::1]:9/a").unwrap();
        assert_eq!((u.host.as_str(), u.port), ("::1", 9));
        for bad in ["ftp://x/y", "https://", "https://a b/", "https://u:p@h/", "http://h:0/", "http://h:99999/", "https://h/\r\nX: y", "file:///etc"]
        {
            assert!(Url::parse(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn resolves_redirects() {
        let u = Url::parse("https://a.example/x/y/file?q").unwrap();
        assert_eq!(u.join("https://b.example/z").unwrap().host, "b.example");
        assert_eq!(u.join("/root").unwrap().path, "/root");
        assert_eq!(u.join("other").unwrap().path, "/x/y/other");
        assert_eq!(u.join("//c.example/p").unwrap().host, "c.example");
        assert!(u.join("ftp://nope").is_err());
        // no downgrade from https to http, however the redirect is written
        assert!(u.join("http://a.example/z").is_err());
        let plain = Url::parse("http://127.0.0.1:8080/a").unwrap();
        assert!(!plain.join("http://127.0.0.1:9/b").unwrap().tls);
        assert!(plain.join("https://b.example/z").unwrap().tls);
    }

}
