//! Network policy for printer URIs.
//!
//! * A printer URI is `ipp://<numeric address>[:port]/path` or `http://...` (both are carried over
//!   plain HTTP; the port is 631 when the URI names none). The address is a literal IPv4 or IPv6
//!   address that is loopback, link-local, private (RFC 1918) or unique-local; host names, public
//!   addresses, user information, queries and fragments are refused, and so are redirects and
//!   proxies (the transport's rules, see `craft-wsd`).
//! * `ipps://` and `https://` are refused with [`IppError::Tls`]: there is no TLS yet, and a numeric
//!   address has no certificate to check anyway.
//! * Consent is the registry's business: a [`crate::BackendClass::Network`] backend only receives
//!   requests that carry a [`crate::UserConfirmed`].

use std::net::IpAddr;

pub use super::adapter::NetworkLimits;
use super::adapter::{DeviceUrl, parse_device_url};
use super::error::{IppError, Result};

/// The IPP port, used when a URI names none.
pub const DEFAULT_PORT: u16 = 631;

/// A validated printer endpoint. Only the [`DeviceUrl`] inside it reaches a transport.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PrinterUri(DeviceUrl);

impl PrinterUri {
    /// Parse and validate. The error says what is allowed.
    pub fn parse(text: &str) -> Result<Self> {
        let trimmed = text.trim();
        let Some((scheme, rest)) = trimmed.split_once("://") else {
            return Err(IppError::Policy("the printer URI must start with ipp:// or http://".into()));
        };
        match scheme.to_ascii_lowercase().as_str() {
            "ipp" | "http" => {}
            "ipps" | "https" => {
                return Err(IppError::Tls(format!(
                    "{scheme}:// needs TLS, which this backend does not implement yet; use ipp:// or http:// on the local network"
                )));
            }
            _ => return Err(IppError::Policy("the printer URI must start with ipp:// or http://".into())),
        }
        let (authority, path) = match rest.find('/') {
            Some(i) => (rest.get(..i).unwrap_or(""), rest.get(i..).unwrap_or("/")),
            None => (rest, "/"),
        };
        let has_port = match authority.strip_prefix('[') {
            Some(inner) => inner.split_once(']').is_some_and(|(_, after)| after.starts_with(':')),
            None => authority.contains(':'),
        };
        let authority = if has_port || authority.contains('@') { authority.to_owned() } else { format!("{authority}:{DEFAULT_PORT}") };
        Ok(Self(parse_device_url(&format!("http://{authority}{path}"), "printer")?))
    }

    pub fn host(&self) -> IpAddr {
        self.0.host()
    }

    pub fn port(&self) -> u16 {
        self.0.port()
    }

    pub fn path(&self) -> &str {
        self.0.path()
    }

    /// The canonical URI (`ipp://host:port/path`, the port always written): the printer's native id
    /// and the `printer-uri` of requests.
    pub fn canonical(&self) -> String {
        match self.0.host() {
            IpAddr::V4(a) => format!("ipp://{a}:{}{}", self.0.port(), self.0.path()),
            IpAddr::V6(a) => format!("ipp://[{a}]:{}{}", self.0.port(), self.0.path()),
        }
    }

    /// The underlying generic endpoint.
    pub(crate) fn as_device(&self) -> &DeviceUrl {
        &self.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_numeric_local_ipp_and_http_uris_are_printer_uris() {
        for (ok, canonical) in [
            ("ipp://127.0.0.1:631/ipp/print", "ipp://127.0.0.1:631/ipp/print"),
            ("ipp://127.0.0.1/ipp/print", "ipp://127.0.0.1:631/ipp/print"),
            ("http://127.0.0.1:8631/d/x/ipp/print", "ipp://127.0.0.1:8631/d/x/ipp/print"),
            ("HTTP://10.1.2.3/p", "ipp://10.1.2.3:631/p"),
            ("ipp://[::1]/ipp", "ipp://[::1]:631/ipp"),
            ("ipp://[fe80::1]:700/x", "ipp://[fe80::1]:700/x"),
            ("ipp://192.168.0.9", "ipp://192.168.0.9:631/"),
            ("  ipp://172.16.0.1:631/ipp/print ", "ipp://172.16.0.1:631/ipp/print"),
        ] {
            assert_eq!(PrinterUri::parse(ok).unwrap().canonical(), canonical, "{ok}");
        }
        // The two schemes name the same printer.
        assert_eq!(PrinterUri::parse("http://127.0.0.1:631/a").unwrap(), PrinterUri::parse("ipp://127.0.0.1/a").unwrap());
        for no in [
            "ipp://printer.local/ipp/print",
            "ipp://localhost/",
            "ipp://8.8.8.8/",
            "ipp://192.0.2.7/x",
            "ipp://user@127.0.0.1/",
            "ipp://127.0.0.1:0/",
            "ipp://127.0.0.1/a?b=c",
            "ipp://127.0.0.1/a#f",
            "ftp://127.0.0.1/",
            "127.0.0.1:631",
            "",
        ] {
            assert!(matches!(PrinterUri::parse(no), Err(IppError::Policy(_))), "{no}");
        }
    }

    #[test]
    fn tls_schemes_are_refused_with_their_own_error() {
        for tls in ["ipps://127.0.0.1/ipp/print", "https://127.0.0.1:443/ipp/print", "IPPS://10.0.0.1/"] {
            let e = PrinterUri::parse(tls).unwrap_err();
            assert!(matches!(e, IppError::Tls(_)), "{tls}");
            assert!(e.to_string().contains("TLS"));
        }
    }
}
