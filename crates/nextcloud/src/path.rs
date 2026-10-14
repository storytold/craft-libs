use std::fmt;
use std::str::FromStr;

use percent_encoding::{AsciiSet, NON_ALPHANUMERIC, utf8_percent_encode};

use crate::Error;

/// Maximum number of segments in a [`RemotePath`].
pub const MAX_SEGMENTS: usize = 64;

const SCHEME: &str = "nextcloud://";
const UNRESERVED: &AsciiSet = &NON_ALPHANUMERIC.remove(b'-').remove(b'.').remove(b'_').remove(b'~');

/// A location on a Nextcloud server: `nextcloud://<user>@<host>[:port]/<seg>/<seg>`.
///
/// Segments are raw (decoded) names; they are percent-encoded only when a URL is built.
#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub struct RemotePath {
    user: String,
    host: String,
    segments: Vec<String>,
}

fn malformed(what: &str) -> Error {
    Error::Malformed(what.to_string())
}

/// Splits `host[:port]` (IPv6 bracketed) into the lowercased host and the port; rejects empty hosts and bad ports.
pub(crate) fn split_host_port(authority: &str) -> Result<(String, Option<u16>), Error> {
    let (host, port) = match authority.strip_prefix('[') {
        Some(rest) => {
            let (inner, after) = rest.split_once(']').ok_or_else(|| malformed("unterminated IPv6 host"))?;
            (format!("[{inner}]"), after.strip_prefix(':').or(if after.is_empty() { None } else { Some("!") }))
        }
        None => match authority.split_once(':') {
            Some((h, p)) => (h.to_string(), Some(p)),
            None => (authority.to_string(), None),
        },
    };
    if host.is_empty() || host == "[]" || host.chars().any(|c| c.is_control() || c.is_whitespace() || matches!(c, '\\' | '/' | '@')) {
        return Err(malformed("bad host"));
    }
    let port = match port {
        None => None,
        Some(p) if !p.is_empty() && p.bytes().all(|b| b.is_ascii_digit()) => Some(p.parse::<u16>().map_err(|_| malformed("bad port"))?),
        Some(_) => return Err(malformed("bad port")),
    };
    Ok((host.to_lowercase(), port))
}

pub(crate) fn check_name(name: &str) -> Result<(), Error> {
    if name.is_empty() || name == "." || name == ".." || name.contains('\\') || name.contains('/') || name.chars().any(char::is_control) {
        return Err(malformed("bad path segment"));
    }
    Ok(())
}

impl RemotePath {
    /// Whether `s` looks like a remote path; a cheap prefix check for routing away from `std::fs`.
    pub fn is_remote(s: &str) -> bool {
        s.starts_with(SCHEME)
    }

    /// Parses `nextcloud://<user>@<host>[:port]/<seg>/...`. A trailing `/` on a non-root path is accepted and ignored.
    pub fn parse(s: &str) -> Result<RemotePath, Error> {
        let rest = s.strip_prefix(SCHEME).ok_or_else(|| malformed("not a nextcloud:// path"))?;
        let (authority, path) = rest.split_once('/').unwrap_or((rest, ""));
        let (user, hostport) = authority.rsplit_once('@').ok_or_else(|| malformed("missing user"))?;
        if user.is_empty() || user.chars().any(char::is_control) {
            return Err(malformed("missing user"));
        }
        let (host, port) = split_host_port(hostport)?;
        let host = match port {
            Some(p) => format!("{host}:{p}"),
            None => host,
        };
        let path = path.strip_suffix('/').unwrap_or(path);
        let segments: Vec<String> = if path.is_empty() { Vec::new() } else { path.split('/').map(str::to_string).collect() };
        if segments.len() > MAX_SEGMENTS {
            return Err(malformed("too many path segments"));
        }
        segments.iter().try_for_each(|s| check_name(s))?;
        Ok(RemotePath { user: user.to_string(), host, segments })
    }

    /// The root of `user`'s files on `host` (`host[:port]`, already normalized).
    pub(crate) fn root(user: &str, host: &str) -> RemotePath {
        RemotePath { user: user.to_string(), host: host.to_string(), segments: Vec::new() }
    }

    /// `user@host[:port]`, identifying the account.
    pub fn account_key(&self) -> String {
        format!("{}@{}", self.user, self.host)
    }

    /// The names below the user's files root; empty for the root.
    pub fn segments(&self) -> &[String] {
        &self.segments
    }

    /// The last segment, or `None` for the root.
    pub fn name(&self) -> Option<&str> {
        self.segments.last().map(String::as_str)
    }

    /// The containing folder, or `None` for the root.
    pub fn parent(&self) -> Option<RemotePath> {
        let (_, rest) = self.segments.split_last()?;
        Some(RemotePath { segments: rest.to_vec(), ..self.clone() })
    }

    /// Appends one name; fails for empty names, `.`, `..`, separators and control characters.
    pub fn join(&self, name: &str) -> Result<RemotePath, Error> {
        check_name(name)?;
        if self.segments.len() >= MAX_SEGMENTS {
            return Err(malformed("too many path segments"));
        }
        let mut p = self.clone();
        p.segments.push(name.to_string());
        Ok(p)
    }
}

impl fmt::Display for RemotePath {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{SCHEME}{}@{}/{}", self.user, self.host, self.segments.join("/"))
    }
}

impl FromStr for RemotePath {
    type Err = Error;

    fn from_str(s: &str) -> Result<Self, Error> {
        RemotePath::parse(s)
    }
}

/// Percent-encodes everything except `A-Za-z0-9-._~`.
pub(crate) fn encode_segment(s: &str) -> String {
    utf8_percent_encode(s, UNRESERVED).to_string()
}

/// `/seg/seg` with each segment encoded; `/` for the root.
pub(crate) fn dav_suffix(p: &RemotePath) -> String {
    if p.segments.is_empty() {
        return "/".to_string();
    }
    p.segments.iter().map(|s| format!("/{}", encode_segment(s))).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_example() {
        let s = "nextcloud://root@cloud.dev-null.me/Sony A7R VI/2026/06/28/_DSC1748.ARW";
        let p = RemotePath::parse(s).unwrap();
        assert_eq!(p.to_string(), s);
        assert_eq!(p.account_key(), "root@cloud.dev-null.me");
        assert_eq!(p.name(), Some("_DSC1748.ARW"));
        assert_eq!(p.segments().len(), 5);
    }

    #[test]
    fn root_and_port() {
        let p: RemotePath = "nextcloud://alice@127.0.0.1:8080/".parse().unwrap();
        assert!(p.segments().is_empty());
        assert_eq!(p.account_key(), "alice@127.0.0.1:8080");
        assert_eq!(p.to_string(), "nextcloud://alice@127.0.0.1:8080/");
        assert_eq!(p.name(), None);
        assert_eq!(p.parent(), None);
    }

    #[test]
    fn ipv6_host_and_lowercasing() {
        let p = RemotePath::parse("nextcloud://u@[::1]:8080/a").unwrap();
        assert_eq!(p.account_key(), "u@[::1]:8080");
        assert_eq!(RemotePath::parse("nextcloud://u@Cloud.EXAMPLE/").unwrap().account_key(), "u@cloud.example");
    }

    #[test]
    fn user_with_at_sign_splits_at_last_at() {
        let p = RemotePath::parse("nextcloud://me@example.com@cloud.host/a").unwrap();
        assert_eq!(p.account_key(), "me@example.com@cloud.host");
    }

    #[test]
    fn rejects_bad_paths() {
        let long = format!("nextcloud://u@h/{}", vec!["a"; 65].join("/"));
        for s in [
            "nextcloud://u@h/a/../b",
            "nextcloud://u@h/./b",
            "nextcloud://u@h/a//b",
            "nextcloud://u@h/a\\b",
            "nextcloud://u@h/a\u{0}b",
            "nextcloud://u@h/a\u{7}b",
            "nextcloud://host/a",
            "nextcloud://@host/a",
            "nextcloud://u@/a",
            "nextcloud://u@h:99999/a",
            "nextcloud://u@h:x/a",
            "nextcloud://u@[::1/a",
            "http://x/a",
            long.as_str(),
        ] {
            assert!(matches!(RemotePath::parse(s), Err(Error::Malformed(_))), "{s:?}");
        }
        let max = format!("nextcloud://u@h/{}", vec!["a"; 64].join("/"));
        assert!(RemotePath::parse(&max).is_ok());
    }

    #[test]
    fn trailing_slash_on_non_root_path_is_ignored() {
        let p = RemotePath::parse("nextcloud://a@b/x/").unwrap();
        assert_eq!(p.to_string(), "nextcloud://a@b/x");
        assert!(RemotePath::parse("nextcloud://a@b/x//").is_err());
    }

    #[test]
    fn is_remote_is_a_prefix_check() {
        assert!(RemotePath::is_remote("nextcloud://a@b/"));
        for s in ["/home/x", "C:\\x", "//host/share"] {
            assert!(!RemotePath::is_remote(s));
        }
    }

    #[test]
    fn join_and_parent() {
        let root = RemotePath::parse("nextcloud://u@h/").unwrap();
        let ab = root.join("a b").unwrap();
        assert_eq!(ab.join("c").unwrap().parent(), Some(ab.clone()));
        assert_eq!(ab.parent(), Some(root.clone()));
        assert!(root.join("..").is_err());
        assert!(root.join("a/b").is_err());
        assert!(root.join("").is_err());
        let deep = RemotePath::parse(&format!("nextcloud://u@h/{}", vec!["a"; 64].join("/"))).unwrap();
        assert!(deep.join("b").is_err());
    }

    #[test]
    fn encode_segment_escapes_reserved() {
        assert_eq!(encode_segment("a b%#?é"), "a%20b%25%23%3F%C3%A9");
        assert_eq!(encode_segment("A-z_0.~"), "A-z_0.~");
    }

    #[test]
    fn dav_suffix_encodes_each_segment() {
        let p = RemotePath::parse("nextcloud://u@h/a b/c").unwrap();
        assert_eq!(dav_suffix(&p), "/a%20b/c");
        assert_eq!(dav_suffix(&RemotePath::parse("nextcloud://u@h/").unwrap()), "/");
    }
}
