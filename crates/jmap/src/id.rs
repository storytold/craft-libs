//! JMAP ids (RFC 8620 §1.2).
//!
//! An id is 1 to [`MAX_ID_LEN`] octets of the URL-safe base64 alphabet of RFC 4648 §5 without padding:
//! `A-Z a-z 0-9 - _`. Every id received from a server goes through [`Id::new`] before it is used as a JSON
//! key, in an address or in a later request. The maximum is the specification's own: RFC 8620 §1.2 allows
//! 255 octets and nothing in a mail client needs to refuse a conforming server's ids.

use std::fmt;
use std::str::FromStr;

/// The longest id accepted, in octets (RFC 8620 §1.2).
pub const MAX_ID_LEN: usize = 255;

/// Why a text is not a valid id.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum IdError {
    /// The id is empty.
    Empty,
    /// The id is longer than [`MAX_ID_LEN`] octets.
    TooLong,
    /// The id holds a character outside `A-Z a-z 0-9 - _`.
    InvalidCharacter,
}

impl fmt::Display for IdError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Empty => "the id is empty, but a JMAP id has 1 to 255 characters",
            Self::TooLong => "the id is longer than 255 octets, the most a JMAP id may have",
            Self::InvalidCharacter => "the id contains a character other than letters, digits, - and _, the only ones a JMAP id may hold",
        })
    }
}

impl std::error::Error for IdError {}

/// A checked JMAP id.
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Id(String);

impl Id {
    /// Checks `text` and wraps it.
    pub fn new(text: &str) -> Result<Self, IdError> {
        if text.is_empty() {
            return Err(IdError::Empty);
        }
        if text.len() > MAX_ID_LEN {
            return Err(IdError::TooLong);
        }
        if !text.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_') {
            return Err(IdError::InvalidCharacter);
        }
        Ok(Self(text.to_owned()))
    }

    /// The id as text.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl FromStr for Id {
    type Err = IdError;

    fn from_str(text: &str) -> Result<Self, IdError> {
        Self::new(text)
    }
}

impl TryFrom<&str> for Id {
    type Error = IdError;

    fn try_from(text: &str) -> Result<Self, IdError> {
        Self::new(text)
    }
}

impl AsRef<str> for Id {
    fn as_ref(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for Id {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl fmt::Debug for Id {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Id({})", self.0)
    }
}

#[cfg(test)]
mod tests;
