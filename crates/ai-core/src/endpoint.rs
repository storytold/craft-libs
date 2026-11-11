//! Endpoint parsing, only as far as consent and locality need.
//!
//! The question this answers is "does this connection send data off this machine?". It is a
//! security decision, so the rules are strict: only `http` and `https`; no user-info in the
//! authority (the `http://127.0.0.1@evil.example/` trick); plain `http` only for loopback hosts;
//! loopback means `localhost`, `127.0.0.0/8` or `::1`. No DNS, no I/O: a name that is not
//! literally `localhost` is treated as remote.

use std::net::{Ipv4Addr, Ipv6Addr};

use crate::ids::IdError;

const MAX_ENDPOINT_LEN: usize = 2048;

/// A parsed endpoint.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Endpoint {
    /// True for `https`, false for `http`.
    pub https: bool,
    /// Host name or address, without port.
    pub host: String,
    /// Explicit port, or `None` for the scheme default.
    pub port: Option<u16>,
}

impl Endpoint {
    /// Parses `http(s)://host[:port][/path]`. Rejects anything surprising.
    pub fn parse(input: &str) -> Result<Self, IdError> {
        if input.is_empty() {
            return Err(IdError::Empty);
        }
        if input.len() > MAX_ENDPOINT_LEN {
            return Err(IdError::TooLong { len: input.len(), max: MAX_ENDPOINT_LEN });
        }
        if let Some(c) = input.chars().find(|c| c.is_control() || c.is_whitespace() || *c == '\\') {
            return Err(IdError::BadChar(c));
        }
        let (https, rest) = if let Some(r) = input.strip_prefix("https://") {
            (true, r)
        } else if let Some(r) = input.strip_prefix("http://") {
            (false, r)
        } else {
            return Err(IdError::Malformed("endpoint must start with http:// or https://"));
        };
        let authority = rest.split(['/', '?', '#']).next().unwrap_or("");
        if authority.contains('@') {
            return Err(IdError::Malformed("user-info in an endpoint is not allowed"));
        }
        let (host, port) = split_host_port(authority)?;
        if host.is_empty() {
            return Err(IdError::Malformed("endpoint has no host"));
        }
        let endpoint = Self { https, host: host.to_ascii_lowercase(), port };
        if !endpoint.https && !endpoint.is_loopback() {
            return Err(IdError::Malformed("plain http is only allowed for loopback hosts"));
        }
        Ok(endpoint)
    }

    /// True for `localhost`, `127.0.0.0/8` and `::1`. Nothing else, and no name resolution.
    pub fn is_loopback(&self) -> bool {
        if self.host == "localhost" {
            return true;
        }
        if let Ok(v4) = self.host.parse::<Ipv4Addr>() {
            return v4.is_loopback();
        }
        self.host.parse::<Ipv6Addr>().map(|v6| v6.is_loopback()).unwrap_or(false)
    }

    /// `scheme://host[:port]`. Consent is bound to this, so changing the host invalidates it.
    pub fn fingerprint(&self) -> String {
        let scheme = if self.https { "https" } else { "http" };
        let host = if self.host.contains(':') { format!("[{}]", self.host) } else { self.host.clone() };
        match self.port {
            Some(p) => format!("{scheme}://{host}:{p}"),
            None => format!("{scheme}://{host}"),
        }
    }
}

fn split_host_port(authority: &str) -> Result<(&str, Option<u16>), IdError> {
    if let Some(rest) = authority.strip_prefix('[') {
        let (host, after) = rest.split_once(']').ok_or(IdError::Malformed("unclosed [ in host"))?;
        let port = match after.strip_prefix(':') {
            Some(p) => Some(parse_port(p)?),
            None if after.is_empty() => None,
            None => return Err(IdError::Malformed("junk after ] in host")),
        };
        return Ok((host, port));
    }
    match authority.rsplit_once(':') {
        Some((host, p)) => Ok((host, Some(parse_port(p)?))),
        None => Ok((authority, None)),
    }
}

fn parse_port(p: &str) -> Result<u16, IdError> {
    p.parse::<u16>().map_err(|_| IdError::Malformed("port is not a number from 0 to 65535"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn userinfo_trick_is_refused() {
        assert!(Endpoint::parse("http://127.0.0.1@evil.example/").is_err());
        assert!(Endpoint::parse("https://localhost:x@evil.example/").is_err());
        // Only the user-info rule catches this one: without it the "host" would be accepted as
        // `user@api.example.com` while an HTTP client connects to `api.example.com`.
        assert!(Endpoint::parse("https://user@api.example.com").is_err());
    }

    #[test]
    fn localhost_lookalikes_are_remote() {
        for host in ["https://localhost.evil.example", "https://127.0.0.1.evil.example", "https://0x7f.0.0.1"] {
            assert!(!Endpoint::parse(host).unwrap().is_loopback(), "{host}");
        }
    }
}
