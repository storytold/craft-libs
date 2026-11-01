//! Network policy for printer URLs, adopted from the first line of code.
//!
//! * A printer URL is `http://<numeric address>[:port]/path`. The address is a literal IPv4 or
//!   IPv6 address that is loopback, link-local, private (RFC 1918) or unique-local (fc00::/7);
//!   host names, public addresses and the other odd forms are refused. The rules and the
//!   [`NetworkLimits`] numbers are the WSD core's; [`PrinterUrl`] is the print-flavoured wrapper
//!   around its `DeviceUrl`.
//! * Consent is the registry's business: a [`crate::BackendClass::Network`] backend only receives
//!   requests that carry a [`crate::UserConfirmed`].

use std::net::IpAddr;

pub use super::adapter::NetworkLimits;
use super::adapter::{DeviceUrl, parse_device_url};
use super::error::Result;

/// A validated printer endpoint. Only the [`DeviceUrl`] inside it reaches a transport.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PrinterUrl(DeviceUrl);

impl PrinterUrl {
    /// Parse and validate. The error says what is allowed.
    pub fn parse(text: &str) -> Result<Self> {
        Ok(Self(parse_device_url(text, "printer")?))
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

    /// The canonical URL text (also the `wsa:To` of requests, and the printer's native id).
    pub fn as_url(&self) -> String {
        self.0.as_url()
    }

    /// The underlying generic endpoint.
    pub(crate) fn as_device(&self) -> &DeviceUrl {
        &self.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::wsd::error::WsdError;

    #[test]
    fn only_numeric_local_addresses_are_printer_urls() {
        for ok in ["http://127.0.0.1:5357/print", "http://10.1.2.3:80/p", "http://[::1]:5357/print"] {
            assert!(PrinterUrl::parse(ok).is_ok(), "{ok}");
        }
        for no in [
            "https://127.0.0.1/x",
            "http://printer.local/x",
            "http://localhost/x",
            "http://8.8.8.8/x",
            "http://192.0.2.7/x",
            "http://198.51.100.9:5357/print",
            "http://198.51.100.9@127.0.0.1/x",
            "ftp://127.0.0.1/x",
            "127.0.0.1",
            "",
        ] {
            assert!(matches!(PrinterUrl::parse(no), Err(WsdError::Policy(_))), "{no}");
        }
        let u = PrinterUrl::parse("http://127.0.0.1:5357/print").unwrap();
        assert_eq!((u.port(), u.path(), u.as_url().as_str()), (5357, "/print", "http://127.0.0.1:5357/print"));
    }
}
