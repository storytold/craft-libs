//! Network policy, adopted from the first line of code.
//!
//! * A device URL is `http://<numeric address>[:port]/path`. The address is a literal IPv4 or
//!   IPv6 address that is loopback, link-local, private (RFC 1918) or unique-local (fc00::/7).
//!   Host names are refused (no DNS lookup can redirect a job), as are public addresses,
//!   multicast, unspecified, carrier-grade NAT (100.64/10), IPv4-mapped IPv6 addresses, user
//!   information, queries and fragments. Only `http` is accepted: a numeric address has no
//!   certificate to check, so TLS would add nothing.
//! * Transports must not use proxies, must not follow redirects, must cap response sizes and
//!   must time out; [`NetworkLimits`] carries the numbers.

use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};
use std::time::Duration;

use crate::error::{Error, Result};

const MAX_URL: usize = 512;
const MAX_PATH: usize = 256;

/// A validated device endpoint. Only this type reaches a [`crate::Transport`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DeviceUrl {
    host: IpAddr,
    port: u16,
    path: String,
}

fn refuse<T>(why: impl Into<String>) -> Result<T> {
    Err(Error::Policy(why.into()))
}

/// Is `ip` an address a device URL may name?
pub fn address_allowed(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(a) => v4_allowed(a),
        IpAddr::V6(a) => v6_allowed(a),
    }
}

fn v4_allowed(a: Ipv4Addr) -> bool {
    // is_private covers 10/8, 172.16/12 and 192.168/16.
    !(a.is_unspecified() || a.is_multicast() || a.is_broadcast()) && (a.is_loopback() || a.is_link_local() || a.is_private())
}

fn v6_allowed(a: Ipv6Addr) -> bool {
    if a.is_unspecified() || a.is_multicast() || a.to_ipv4_mapped().is_some() {
        return false;
    }
    let first = a.segments().first().copied().unwrap_or(0);
    a.is_loopback() || (first & 0xffc0) == 0xfe80 || (first & 0xfe00) == 0xfc00
}

impl DeviceUrl {
    /// Parse and validate. The error says what is allowed.
    pub fn parse(text: &str) -> Result<Self> {
        Self::parse_as(text, "device")
    }

    /// Like [`Self::parse`], but error messages call the endpoint `noun` ("the printer URL ...").
    pub fn parse_as(text: &str, noun: &str) -> Result<Self> {
        if text.len() > MAX_URL {
            return refuse(format!("the {noun} URL is too long"));
        }
        if text.chars().any(|c| c.is_control() || c.is_whitespace() || !c.is_ascii()) {
            return refuse(format!("the {noun} URL must be plain ASCII without spaces or control characters"));
        }
        let Some(rest) = strip_prefix_ci(text, "http://") else {
            return refuse(format!("the {noun} URL must start with http:// (https and other schemes are not used for local {noun}s)"));
        };
        let (authority, path) = match rest.find('/') {
            Some(i) => (rest.get(..i).unwrap_or(""), rest.get(i..).unwrap_or("/")),
            None => (rest, "/"),
        };
        if authority.contains('@') {
            return refuse(format!("the {noun} URL must not carry user information"));
        }
        if path.contains(['?', '#']) {
            return refuse(format!("the {noun} URL must not have a query or fragment"));
        }
        if path.len() > MAX_PATH || path.starts_with("//") {
            return refuse(format!("the {noun} URL path is not acceptable"));
        }
        let (host_text, port_text) = split_authority(authority, noun)?;
        let host: IpAddr = host_text.parse().map_err(|_| {
            Error::Policy(format!("{host_text:?} is not a numeric address: use an IP address (loopback, link-local or private), not a host name"))
        })?;
        if !address_allowed(host) {
            return refuse(format!("{host} is not a loopback, link-local or private address"));
        }
        let port = match port_text {
            None => 80,
            Some(p) => match p.parse::<u16>() {
                Ok(n) if n != 0 => n,
                _ => return refuse(format!("{p:?} is not a valid port")),
            },
        };
        Ok(Self { host, port, path: path.to_owned() })
    }

    pub fn host(&self) -> IpAddr {
        self.host
    }

    pub fn port(&self) -> u16 {
        self.port
    }

    pub fn path(&self) -> &str {
        &self.path
    }

    /// The canonical URL text (also the `wsa:To` of requests).
    pub fn as_url(&self) -> String {
        match self.host {
            IpAddr::V4(a) => format!("http://{a}:{}{}", self.port, self.path),
            IpAddr::V6(a) => format!("http://[{a}]:{}{}", self.port, self.path),
        }
    }
}

fn strip_prefix_ci<'a>(text: &'a str, prefix: &str) -> Option<&'a str> {
    let head = text.get(..prefix.len())?;
    if head.eq_ignore_ascii_case(prefix) { text.get(prefix.len()..) } else { None }
}

fn split_authority<'a>(authority: &'a str, noun: &str) -> Result<(&'a str, Option<&'a str>)> {
    if let Some(inner) = authority.strip_prefix('[') {
        let Some(close) = inner.find(']') else { return refuse(format!("unterminated [ in the {noun} URL")) };
        let host = inner.get(..close).unwrap_or("");
        let after = inner.get(close + 1..).unwrap_or("");
        if after.is_empty() {
            return Ok((host, None));
        }
        return match after.strip_prefix(':') {
            Some(p) => Ok((host, Some(p))),
            None => refuse(format!("unexpected text after the address in the {noun} URL")),
        };
    }
    match authority.rsplit_once(':') {
        Some((h, p)) => Ok((h, Some(p))),
        None => Ok((authority, None)),
    }
}

/// Numbers every transport honours.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct NetworkLimits {
    /// Connect plus the whole exchange, per request.
    pub timeout: Duration,
    /// The largest response body a transport may return.
    pub max_response: usize,
    /// The largest request body the client will send (a document included).
    pub max_request: usize,
}

impl Default for NetworkLimits {
    fn default() -> Self {
        Self { timeout: Duration::from_secs(30), max_response: 2 << 20, max_request: 256 << 20 }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ok(s: &str) -> DeviceUrl {
        DeviceUrl::parse(s).unwrap_or_else(|e| panic!("{s}: {e}"))
    }
    fn no(s: &str) {
        assert!(matches!(DeviceUrl::parse(s), Err(Error::Policy(_))), "{s} should be refused");
    }

    #[test]
    fn local_numeric_addresses_are_accepted() {
        for s in [
            "http://127.0.0.1:5357/x",
            "http://127.9.9.9/",
            "http://10.1.2.3:80/p",
            "http://172.16.0.1/",
            "http://172.31.255.255/",
            "http://192.168.1.50:3911/ws",
            "http://169.254.7.7/",
            "http://[::1]:5357/a",
            "http://[fe80::1]/",
            "http://[fd12:3456::9]:631/",
            "HTTP://127.0.0.1/",
        ] {
            ok(s);
        }
        assert_eq!(ok("http://127.0.0.1").path(), "/");
        assert_eq!(ok("http://127.0.0.1").port(), 80);
        assert_eq!(ok("http://[::1]:8/p").as_url(), "http://[::1]:8/p");
    }

    #[test]
    fn names_public_and_odd_addresses_are_refused() {
        for s in [
            "http://localhost:5357/",
            "http://printer.local/",
            "http://example.com/",
            "http://8.8.8.8/",
            "http://172.32.0.1/",
            "http://172.15.0.1/",
            "http://100.64.0.1/",
            "http://0.0.0.0/",
            "http://255.255.255.255/",
            "http://224.0.0.1/",
            "http://[::]/",
            "http://[ff02::1]/",
            "http://[::ffff:8.8.8.8]/",
            "http://[::ffff:127.0.0.1]/",
            "http://[2001:db8::1]/",
            "http://127.1/",
            "http://0x7f.0.0.1/",
            "http://2130706433/",
            "http://127.0.0.1@8.8.8.8/",
            "http://user@127.0.0.1/",
            "https://127.0.0.1/",
            "ftp://127.0.0.1/",
            "127.0.0.1",
            "http://127.0.0.1:0/",
            "http://127.0.0.1:99999/",
            "http://127.0.0.1:/",
            "http://127.0.0.1/a?b=c",
            "http://127.0.0.1/a#f",
            "http://127.0.0.1/a b",
            "http://[::1/",
            "http://[::1]x/",
            "http://127.0.0.1/\u{e9}",
            "",
            "http://",
            "http:///",
        ] {
            no(s);
        }
        no(&format!("http://127.0.0.1/{}", "a".repeat(600)));
    }

    #[test]
    fn the_noun_in_messages_follows_the_caller() {
        let default = DeviceUrl::parse("ftp://127.0.0.1/").unwrap_err().to_string();
        assert!(default.contains("the device URL"), "{default}");
        let print = DeviceUrl::parse_as("ftp://127.0.0.1/", "printer").unwrap_err().to_string();
        assert!(print.contains("the printer URL") && print.contains("local printers"), "{print}");
        assert!(DeviceUrl::parse_as("http://127.0.0.1/x", "scanner").is_ok());
    }
}
