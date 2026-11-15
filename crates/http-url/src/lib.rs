//! Bounded HTTP/HTTPS URL parsing and redirect resolution, extracted from LightCraft.
//!
//! This is syntax handling, not a network permission or SSRF policy. It permits local/private
//! hosts and performs no DNS, socket or TLS operations. Callers own destination allowlists,
//! credential forwarding, redirect counts, connection deadlines and response limits.
//! Only parsed values have validated fields; constructing public fields bypasses parsing.

#![forbid(unsafe_code)]
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

/// A URL or redirect cannot be represented by this bounded HTTP URL syntax.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UrlError(pub String);

impl std::fmt::Display for UrlError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}
impl std::error::Error for UrlError {}

/// A parsed `http://` or `https://` URL.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Url {
    pub tls: bool,
    pub host: String,
    pub port: u16,
    /// Path and query, starting with `/`.
    pub path: String,
}

/// Remove `.` and `..` segments from the path part of `path` (RFC 3986 section 5.2.4); the query is kept as is.
fn remove_dot_segments(path: &str) -> String {
    let (p, query) = match path.find('?') {
        Some(i) => (path.get(..i).unwrap_or_default(), path.get(i..).unwrap_or_default()),
        None => (path, ""),
    };
    let mut out: Vec<&str> = Vec::new();
    let mut segments = p.split('/').skip(1).peekable();
    let mut trailing_slash = false;
    while let Some(seg) = segments.next() {
        let last = segments.peek().is_none();
        match seg {
            "." => trailing_slash = last,
            ".." => {
                out.pop();
                trailing_slash = last;
            }
            _ => {
                out.push(seg);
                trailing_slash = false;
            }
        }
    }
    let mut result = format!("/{}", out.join("/"));
    if trailing_slash && !result.ends_with('/') {
        result.push('/');
    }
    result.push_str(query);
    result
}

/// `s` without the leading `prefix`, matched ASCII case-insensitively (RFC 3986 §3.1).
fn strip_scheme<'a>(s: &'a str, prefix: &str) -> Option<&'a str> {
    let head = s.get(..prefix.len())?;
    head.eq_ignore_ascii_case(prefix).then(|| s.get(prefix.len()..).unwrap_or_default())
}

impl Url {
    pub fn parse(s: &str) -> Result<Url, UrlError> {
        let bad = |why: &str| UrlError(why.to_string());
        if s.len() > 16 * 1024 {
            return Err(bad("URL exceeds 16 KiB"));
        }
        let s = s.trim();
        if s.chars().any(|c| c.is_control() || c == ' ') {
            return Err(bad("contains spaces or control characters"));
        }
        let (tls, rest) = if let Some(r) = strip_scheme(s, "https://") {
            (true, r)
        } else if let Some(r) = strip_scheme(s, "http://") {
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
        let path = remove_dot_segments(&path);
        if authority.contains('@') {
            return Err(bad("user names and passwords in URLs are not supported"));
        }
        let (host, port) = if let Some(v6) = authority.strip_prefix('[') {
            let end = v6.find(']').ok_or_else(|| bad("unterminated IPv6 address"))?;
            let host = v6.get(..end).unwrap_or_default();
            let after = v6.get(end + 1..).unwrap_or_default();
            if !after.is_empty() && !after.starts_with(':') {
                return Err(bad("unexpected text after IPv6 address"));
            }
            (host.to_string(), after.strip_prefix(':'))
        } else {
            match authority.rsplit_once(':') {
                Some((h, p)) => (h.to_string(), Some(p)),
                None => (authority.to_string(), None),
            }
        };
        let port = match port {
            Some(p) if p.bytes().all(|b| b.is_ascii_digit()) => p.parse::<u16>().ok().filter(|p| *p != 0).ok_or_else(|| bad("bad port"))?,
            Some(_) => return Err(bad("bad port")),
            None if tls => 443,
            None => 80,
        };
        if host.is_empty() || !host.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '.' | ':' | '_')) {
            return Err(bad("bad host name"));
        }
        Ok(Url { tls, host: host.to_ascii_lowercase(), port, path })
    }

    /// `Location` of a redirect, resolved against this URL.
    pub fn join(&self, location: &str) -> Result<Url, UrlError> {
        // Reject before concatenating a hostile Location with the base URL.
        if location.len() > 16 * 1024 || self.host.len().saturating_add(self.path.len()) > 16 * 1024 {
            return Err(UrlError("URL exceeds 16 KiB".into()));
        }
        let l = location.trim();
        if strip_scheme(l, "http://").is_some() || strip_scheme(l, "https://").is_some() {
            let url = Url::parse(l)?;
            if self.tls && !url.tls {
                return Err(UrlError("redirect from https to http refused".into()));
            }
            return Ok(url);
        }
        // another scheme (`ftp:`, `file:`, `data:`…)
        if let Some((scheme, _)) = l.split_once(':')
            && !scheme.is_empty()
            && scheme.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '-' | '.'))
        {
            return Err(UrlError(format!("redirect to an unsupported location `{scheme}:`")));
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
        // empty or fragment-only: the same document
        if l.is_empty() || l.starts_with('#') {
            return Ok(self.clone());
        }
        // relative to the current directory
        let dir = self.path.split('?').next().unwrap_or("/");
        // a query-only reference keeps the whole path, file name included (RFC 3986 §5.2.2)
        if l.starts_with('?') {
            return Url::parse(&format!("{base}{dir}{l}"));
        }
        let dir = dir.rsplit_once('/').map_or("", |(d, _)| d);
        Url::parse(&format!("{base}{dir}/{l}"))
    }

    /// Authority formatted for an HTTP Host field.
    pub fn host_header(&self) -> String {
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
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
    use super::{Url, UrlError};

    #[test]
    fn fragment_only_or_empty_location_keeps_file_name_and_query() {
        let base = Url::parse("https://a.example/x/y/file?q").unwrap();
        for l in ["#top", "", "  "] {
            let u = base.join(l).unwrap();
            assert_eq!((u.tls, u.host.as_str(), u.port, u.path.as_str()), (true, "a.example", 443, "/x/y/file?q"), "{l:?}");
        }
    }

    #[test]
    fn rejects_oversized_and_control_character_urls() {
        let base = Url::parse("https://example.com/path").unwrap();
        assert!(base.join(&format!("#{}", "x".repeat(16 * 1024))).is_err());
        assert!(base.join(&"x".repeat(16 * 1024 + 1)).is_err());
        assert!(Url::parse(&format!("https://example.com/{}", "x".repeat(16 * 1024))).is_err());
        assert!(Url::parse("https://example.com/x?secret=abc\r\nInjected: 1").is_err());
    }

    #[test]
    fn join_query_only_keeps_file_name() {
        let u = Url::parse("https://a.example/x/y/file?q=1").unwrap();
        assert_eq!(u.join("?page=2").unwrap().path, "/x/y/file?page=2");
    }

    #[test]
    fn join_removes_dot_segments() {
        let base = Url::parse("https://a.example/x/y/file").unwrap();
        for (loc, want) in
            [("../z", "/x/z"), ("./z", "/x/y/z"), ("../../../z", "/z"), ("/a/./b/../c", "/a/c"), ("../z?q=1", "/x/z?q=1"), ("..", "/x/")]
        {
            assert_eq!(base.join(loc).unwrap().path, want, "{loc}");
        }
    }

    #[test]
    fn url_scheme_is_matched_case_insensitively() {
        let u = Url::parse("HTTPS://A.example/z").unwrap();
        assert_eq!((u.tls, u.host.as_str(), u.port, u.path.as_str()), (true, "a.example", 443, "/z"));
        assert!(Url::parse("Http://a.example/").is_ok());
        let j = Url::parse("https://a.example/start").unwrap().join("HTTPS://cdn.example/file").unwrap();
        assert_eq!((j.tls, j.host.as_str(), j.path.as_str()), (true, "cdn.example", "/file"));
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
    fn rejects_signed_ports() {
        for bad in ["http://example.com:+80/x", "http://example.com:-80/x", "http://[::1]:+80/x"] {
            assert!(Url::parse(bad).is_err(), "{bad}");
        }
        assert!(Url::parse("http://a.test/").unwrap().join("http://example.com:+80/x").is_err());
        assert_eq!(Url::parse("http://example.com:80/x").unwrap().port, 80);
    }

    #[test]
    fn rejects_text_after_a_bracketed_ipv6_host() {
        for bad in ["http://[::1]junk/x", "https://[::1]junk/x", "http://[::1]8080/x", "http://[::1]8080"] {
            assert!(matches!(Url::parse(bad), Err(UrlError(_))), "{bad}");
        }
        let u = Url::parse("http://[::1]:8080/x").unwrap();
        assert_eq!((u.host.as_str(), u.port), ("::1", 8080));
        let u = Url::parse("http://[::1]/x").unwrap();
        assert_eq!((u.host.as_str(), u.port), ("::1", 80));
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
