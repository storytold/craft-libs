use serde::{Deserialize, Serialize};

use crate::path::split_host_port;
use crate::{Error, Fingerprint, RemotePath};

/// A Nextcloud account. Holds no secret; the app password lives in the keyring.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Account {
    /// Normalized server URL, e.g. `https://cloud.example/nc`.
    pub server: String,
    /// The Login Flow login name.
    pub user: String,
    /// SHA-256 of the leaf certificate the user chose to trust, if any.
    pub trust: Option<Fingerprint>,
}

impl Account {
    /// `host[:port]` of the server; the server string as given when it doesn't parse.
    fn authority(&self) -> String {
        match normalize_server(&self.server) {
            Ok(url) => {
                let rest = url.split_once("://").map_or(url.as_str(), |(_, r)| r);
                rest.split('/').next().unwrap_or(rest).to_string()
            }
            Err(_) => self.server.clone(),
        }
    }

    /// `user@host[:port]`, same as [`RemotePath::account_key`] for this account's paths.
    pub fn key(&self) -> String {
        format!("{}@{}", self.user, self.authority())
    }

    /// The root of this account's files.
    pub fn root(&self) -> RemotePath {
        RemotePath::root(&self.user, &self.authority())
    }
}

/// Canonical form of a user-typed server address: `https://host[:port][/prefix]`.
///
/// Adds `https://` when no scheme is given, lowercases the host, drops the scheme's default port and trailing `/`.
/// Rejects userinfo, query, fragment, schemes other than http(s), and `http://` for non-loopback hosts, as
/// `InvalidAddress`.
pub(crate) fn normalize_server(input: &str) -> Result<String, Error> {
    let bad = |what: &str| Error::InvalidAddress(what.to_string());
    let input = input.trim();
    let (scheme, rest) = match input.split_once("://") {
        Some((s, r)) => (s.to_ascii_lowercase(), r),
        None => ("https".to_string(), input),
    };
    if scheme != "https" && scheme != "http" {
        return Err(bad("server must be an https:// address"));
    }
    if rest.contains(['?', '#']) {
        return Err(bad("server address can't have a query or fragment"));
    }
    let (authority, path) = rest.find('/').map_or((rest, ""), |i| rest.split_at(i));
    if authority.contains('@') {
        return Err(bad("server address can't contain a user name"));
    }
    let (host, port) = split_host_port(authority).map_err(|e| match e {
        Error::Malformed(d) => Error::InvalidAddress(d),
        e => e,
    })?;
    let loopback = host == "localhost" || host == "[::1]" || host.parse::<std::net::Ipv4Addr>().is_ok_and(|a| a.is_loopback());
    if scheme == "http" && !loopback {
        return Err(bad("http:// is only allowed for localhost"));
    }
    let default = if scheme == "https" { 443 } else { 80 };
    let port = port.filter(|p| *p != default).map(|p| format!(":{p}")).unwrap_or_default();
    Ok(format!("{scheme}://{host}{port}{}", path.trim_end_matches('/')))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalize_server_cases() {
        let n = |s| normalize_server(s).unwrap();
        assert_eq!(n("cloud.dev-null.me/"), "https://cloud.dev-null.me");
        assert_eq!(n("https://Cloud.Example:443/nc/"), "https://cloud.example/nc");
        assert_eq!(n("  http://127.0.0.1:8080 "), "http://127.0.0.1:8080");
        assert_eq!(n("http://localhost:80"), "http://localhost");
        assert_eq!(n("http://[::1]:8080"), "http://[::1]:8080");
        assert_eq!(n("HTTPS://x.example"), "https://x.example");
        for s in [
            "http://example.com",
            "http://127.example.com",
            "http://127.a.b.c",
            "http://127.999.0.1",
            "ftp://x",
            "https://u:p@x",
            "https://x?q=1",
            "https://x#f",
            "",
            "https://x:99999",
        ] {
            assert!(matches!(normalize_server(s), Err(Error::InvalidAddress(_))), "{s:?}");
        }
    }

    #[test]
    fn account_key_matches_remote_path() {
        let a = Account { server: "https://cloud.example:8443/nc".into(), user: "bob".into(), trust: None };
        assert_eq!(a.key(), "bob@cloud.example:8443");
        assert_eq!(a.key(), a.root().account_key());
        assert_eq!(a.root().to_string(), "nextcloud://bob@cloud.example:8443/");
    }

    #[test]
    fn unparsable_server_does_not_panic() {
        let a = Account { server: "ftp://x".into(), user: "bob".into(), trust: None };
        assert_eq!(a.key(), "bob@ftp://x");
        let _ = a.root();
    }

    #[test]
    fn account_serde_round_trip() {
        let a = Account { server: "https://c.example".into(), user: "bob".into(), trust: Some(Fingerprint([7; 32])) };
        let json = serde_json::to_string(&a).unwrap();
        assert_eq!(serde_json::from_str::<Account>(&json).unwrap(), a);
    }
}
