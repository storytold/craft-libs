use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

/// SHA-256 fingerprint of a server's leaf certificate (DER).
#[derive(Copy, Clone, PartialEq, Eq, Hash, Debug, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct Fingerprint(pub [u8; 32]);

impl Fingerprint {
    /// Fingerprint of a DER-encoded certificate.
    pub fn of(der: &[u8]) -> Fingerprint {
        Fingerprint(Sha256::digest(der).into())
    }
}

impl fmt::Display for Fingerprint {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for (i, b) in self.0.iter().enumerate() {
            if i > 0 {
                f.write_str(":")?;
            }
            write!(f, "{b:02X}")?;
        }
        Ok(())
    }
}

impl FromStr for Fingerprint {
    type Err = Error;

    /// Accepts upper or lower case hex, with or without `:` separators.
    fn from_str(s: &str) -> Result<Self, Error> {
        let digits: Vec<u8> = s.chars().filter(|c| *c != ':').map(|c| c.to_digit(16).map(|d| d as u8)).collect::<Option<_>>().ok_or_else(bad)?;
        if digits.len() != 64 {
            return Err(bad());
        }
        let mut out = [0u8; 32];
        for (byte, pair) in out.iter_mut().zip(digits.as_chunks::<2>().0) {
            *byte = pair.iter().fold(0, |acc, d| acc << 4 | d);
        }
        Ok(Fingerprint(out))
    }
}

fn bad() -> Error {
    Error::Malformed("not a SHA-256 fingerprint".into())
}

impl From<Fingerprint> for String {
    fn from(f: Fingerprint) -> String {
        f.to_string()
    }
}

impl TryFrom<String> for Fingerprint {
    type Error = Error;

    fn try_from(s: String) -> Result<Self, Error> {
        s.parse()
    }
}

/// Everything that can go wrong talking to a Nextcloud server. The messages are shown to users.
#[derive(Debug)]
pub enum Error {
    Unreachable(String),
    Unauthorized,
    NotFound,
    CertificateChanged,
    UnknownCertificate { fingerprint: Fingerprint },
    RateLimited,
    TooLarge,
    Malformed(String),
    InvalidAddress(String),
    Cancelled,
    Io(std::io::Error),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Unreachable(d) => write!(f, "Can't reach the Nextcloud server ({d}); check the address and your connection"),
            Error::Unauthorized => f.write_str("Nextcloud rejected the app password; sign in again"),
            Error::NotFound => f.write_str("The file or folder is no longer on the Nextcloud server"),
            Error::CertificateChanged => {
                f.write_str("The Nextcloud server's certificate changed since you trusted it; check with the server's admin, then sign in again")
            }
            Error::UnknownCertificate { fingerprint } => write!(
                f,
                "The Nextcloud server's certificate isn't trusted by this computer (SHA-256 {fingerprint}); confirm the fingerprint with the server's admin before trusting it"
            ),
            Error::RateLimited => f.write_str("The Nextcloud server is busy; try again in a minute"),
            Error::TooLarge => f.write_str("The Nextcloud server sent more data than expected"),
            Error::Malformed(d) => write!(f, "The Nextcloud server sent a response that couldn't be read ({d})"),
            Error::InvalidAddress(d) => write!(f, "That isn't a usable Nextcloud address ({d}); check what you typed"),
            Error::Cancelled => f.write_str("Cancelled"),
            Error::Io(e) => write!(f, "Local file error: {e}"),
        }
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Error::Io(e) => Some(e),
            _ => None,
        }
    }
}

impl From<std::io::Error> for Error {
    fn from(e: std::io::Error) -> Error {
        Error::Io(e)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn error_messages_are_actionable() {
        assert_eq!(Error::Unauthorized.to_string(), "Nextcloud rejected the app password; sign in again");
        let e = Error::UnknownCertificate { fingerprint: Fingerprint([0; 32]) };
        assert!(e.to_string().contains("(SHA-256 00:00:"));
        assert_eq!(Error::InvalidAddress("bad port".into()).to_string(), "That isn't a usable Nextcloud address (bad port); check what you typed");
    }

    #[test]
    fn io_error_has_source() {
        let e = Error::from(std::io::Error::other("boom"));
        assert_eq!(e.to_string(), "Local file error: boom");
        assert!(std::error::Error::source(&e).is_some());
    }

    #[test]
    fn fingerprint_display_and_parse() {
        let f = Fingerprint([0xAB; 32]);
        let s = f.to_string();
        assert!(s.starts_with("AB:AB:"));
        assert_eq!(s.len(), 95);
        assert_eq!("ab".repeat(32).parse::<Fingerprint>().unwrap(), f);
        assert_eq!(s.parse::<Fingerprint>().unwrap(), f);
        assert!("ab:cd".parse::<Fingerprint>().is_err());
        assert!("zz".repeat(32).parse::<Fingerprint>().is_err());
    }

    #[test]
    fn fingerprint_of_is_sha256() {
        assert!(Fingerprint::of(b"").to_string().starts_with("E3:B0:C4:42:"));
    }

    #[test]
    fn fingerprint_serde_is_the_display_string() {
        let f = Fingerprint([0x01; 32]);
        let json = serde_json::to_string(&f).unwrap();
        assert_eq!(json, format!("\"{f}\""));
        assert_eq!(serde_json::from_str::<Fingerprint>(&json).unwrap(), f);
        assert!(serde_json::from_str::<Fingerprint>("\"nope\"").is_err());
    }
}
