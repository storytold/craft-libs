//! Validated string types and the macros that build them.
//!
//! Every identifier that crosses a trust boundary (a settings file, a provider response, a
//! command parameter) is a validated newtype, so a malformed value is an error at the edge and
//! never a surprise deeper in. Validation is by character class and length, with no indexing, so
//! hostile input cannot panic. Nothing here touches I/O.

use std::fmt;

/// Longest accepted identifier, in bytes.
pub const MAX_ID_LEN: usize = 128;

/// Why a string was refused as an identifier.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum IdError {
    Empty,
    TooLong { len: usize, max: usize },
    BadChar(char),
    Malformed(&'static str),
}

impl fmt::Display for IdError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Empty => f.write_str("value is empty"),
            Self::TooLong { len, max } => write!(f, "value is {len} bytes, the limit is {max}"),
            Self::BadChar(c) => write!(f, "character {c:?} is not allowed"),
            Self::Malformed(why) => write!(f, "malformed value: {why}"),
        }
    }
}

impl std::error::Error for IdError {}

fn check(s: &str, ok: impl Fn(char) -> bool) -> Result<(), IdError> {
    if s.is_empty() {
        return Err(IdError::Empty);
    }
    if s.len() > MAX_ID_LEN {
        return Err(IdError::TooLong { len: s.len(), max: MAX_ID_LEN });
    }
    match s.chars().find(|c| !ok(*c)) {
        Some(c) => Err(IdError::BadChar(c)),
        None => Ok(()),
    }
}

/// General identifiers: ASCII letters and digits plus `- _ . : /` (model names such as
/// `vendor/model-1.5` are common).
pub fn validate_id(s: &str) -> Result<(), IdError> {
    check(s, |c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.' | ':' | '/'))
}

/// Tokens: ASCII letters and digits plus `- _`. No dot, so `modality.task` splits unambiguously.
pub fn validate_token(s: &str) -> Result<(), IdError> {
    check(s, |c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_'))
}

/// A string newtype whose constructor and deserializer both run `$validator`.
macro_rules! validated_string {
    ($(#[$m:meta])* $name:ident, $validator:path) => {
        $(#[$m])*
        #[derive(Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
        pub struct $name(String);

        impl $name {
            /// Validates and wraps `value`.
            pub fn new(value: impl Into<String>) -> Result<Self, $crate::ids::IdError> {
                let value = value.into();
                $validator(&value)?;
                Ok(Self(value))
            }

            pub fn as_str(&self) -> &str {
                &self.0
            }
        }

        impl std::fmt::Display for $name {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.write_str(&self.0)
            }
        }

        impl std::fmt::Debug for $name {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                write!(f, "{}({:?})", stringify!($name), self.0)
            }
        }

        impl std::str::FromStr for $name {
            type Err = $crate::ids::IdError;
            fn from_str(s: &str) -> Result<Self, Self::Err> {
                Self::new(s)
            }
        }

        impl serde::Serialize for $name {
            fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
                s.serialize_str(&self.0)
            }
        }

        impl<'de> serde::Deserialize<'de> for $name {
            fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
                let raw = <String as serde::Deserialize>::deserialize(d)?;
                Self::new(raw).map_err(serde::de::Error::custom)
            }
        }
    };
}

/// An enum with well-known variants plus `Other(String)`, serialized as a plain string. Unknown
/// but well-formed values survive a round trip, so an app can add its own without a library
/// release. `parse` is the only way to get a canonical value from text.
macro_rules! open_enum {
    ($(#[$m:meta])* $name:ident { $($var:ident => $text:literal),+ $(,)? }) => {
        $(#[$m])*
        #[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
        pub enum $name {
            $($var,)+
            /// A well-formed value this library does not know by name.
            Other(String),
        }

        impl $name {
            pub fn as_str(&self) -> &str {
                match self {
                    $(Self::$var => $text,)+
                    Self::Other(s) => s,
                }
            }

            /// Maps well-known names to their variant and validates anything else.
            pub fn parse(s: &str) -> Result<Self, $crate::ids::IdError> {
                match s {
                    $($text => Ok(Self::$var),)+
                    other => {
                        $crate::ids::validate_token(other)?;
                        Ok(Self::Other(other.to_owned()))
                    }
                }
            }
        }

        impl std::fmt::Display for $name {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.write_str(self.as_str())
            }
        }

        impl serde::Serialize for $name {
            fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
                s.serialize_str(self.as_str())
            }
        }

        impl<'de> serde::Deserialize<'de> for $name {
            fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
                let raw = <String as serde::Deserialize>::deserialize(d)?;
                Self::parse(&raw).map_err(serde::de::Error::custom)
            }
        }
    };
}

pub(crate) use open_enum;
pub(crate) use validated_string;

validated_string!(
    /// Identifies one saved connection (a place to send work). Stable across renames.
    ConnectionId,
    validate_id
);
validated_string!(
    /// A provider's name for a model, such as `gpt-image-1` or `vendor/model-1.5`.
    ModelId,
    validate_id
);
validated_string!(
    /// An app-declared job the user can bind to a model, such as `select-subject`.
    RoleId,
    validate_id
);
validated_string!(
    /// One Craft app, such as `photocraft`.
    AppId,
    validate_id
);
validated_string!(
    /// One in-flight unit of work on a provider.
    JobId,
    validate_id
);
validated_string!(
    /// Names a secret held in a keychain or encrypted file. It locates a secret and is not one.
    SecretRef,
    validate_id
);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_accept_normal_values() {
        assert!(ModelId::new("black-forest-labs/flux-1.1-pro").is_ok());
        assert!(ConnectionId::new("conn_01hx").is_ok());
    }

    #[test]
    fn ids_reject_bad_values_without_panicking() {
        assert_eq!(ModelId::new(""), Err(IdError::Empty));
        assert_eq!(ModelId::new("a b"), Err(IdError::BadChar(' ')));
        assert_eq!(ModelId::new("a\0b"), Err(IdError::BadChar('\0')));
        assert!(matches!(ModelId::new("x".repeat(MAX_ID_LEN + 1)), Err(IdError::TooLong { .. })));
        assert!(ModelId::new("\u{202e}rtl").is_err());
    }

    #[test]
    fn tokens_reject_dots() {
        assert!(validate_token("image").is_ok());
        assert_eq!(validate_token("a.b"), Err(IdError::BadChar('.')));
    }

    #[test]
    fn deserialize_validates() {
        let ok: Result<ModelId, _> = serde_json::from_str("\"gpt-image-1\"");
        assert!(ok.is_ok());
        let bad: Result<ModelId, _> = serde_json::from_str("\"has space\"");
        assert!(bad.is_err());
    }

    #[test]
    fn debug_shows_type_and_value() {
        assert_eq!(format!("{:?}", AppId::new("photocraft").unwrap()), "AppId(\"photocraft\")");
    }
}
