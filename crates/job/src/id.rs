//! Typed `<backend>:<native>` ids for jobs and devices.
//!
//! The text form is the one craft-print's job ids and craft-scan's device ids already use:
//! `"<backend>:<native>"`. The backend part is a registry name (lowercase ASCII, digits, `-`, `_`,
//! not empty); the native part is the backend's own handle, may contain colons and anything else,
//! and must not be empty. Only the first colon separates the two.

use std::fmt;
use std::str::FromStr;

/// `"<backend>:<native id>"`, without validation.
pub fn namespaced_id(backend: &str, native_id: &str) -> String {
    format!("{backend}:{native_id}")
}

/// Split a namespaced id at its first colon, without validation.
pub fn split_id(id: &str) -> Option<(&str, &str)> {
    id.split_once(':')
}

/// Whether `name` is non-empty and made only of lowercase ASCII letters, digits, `-` and `_`.
fn valid_backend_name(name: &str) -> bool {
    !name.is_empty() && name.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-' || c == '_')
}

/// Why text is not an id.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum ParseIdErrorKind {
    /// No colon: not `"<backend>:<native>"`.
    MissingSeparator,
    /// Nothing before the first colon.
    EmptyBackend,
    /// The backend part has a character other than lowercase ASCII, digits, `-` and `_`.
    InvalidBackend,
    /// Nothing after the first colon.
    EmptyNative,
}

/// The typed failure of parsing a [`BackendName`], [`JobId`] or [`DeviceId`]; see [`Self::kind`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct ParseIdError {
    kind: ParseIdErrorKind,
}

impl ParseIdError {
    /// Wraps `kind`; only this module builds the error.
    const fn new(kind: ParseIdErrorKind) -> Self {
        Self { kind }
    }

    /// Which part of the text was wrong.
    pub const fn kind(&self) -> ParseIdErrorKind {
        self.kind
    }
}

impl fmt::Display for ParseIdError {
    /// Writes a one-sentence description of the problem; the text is stable and does not include the input.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self.kind {
            ParseIdErrorKind::MissingSeparator => "an id looks like <backend>:<native id>, and this has no colon",
            ParseIdErrorKind::EmptyBackend => "the id has no backend name before its colon",
            ParseIdErrorKind::InvalidBackend => "the backend name of the id has a character other than a-z, 0-9, '-' and '_'",
            ParseIdErrorKind::EmptyNative => "the id has nothing after its colon",
        })
    }
}

impl std::error::Error for ParseIdError {}

/// A validated backend registry name: one or more of `a-z`, `0-9`, `-`, `_`.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(into = "String", try_from = "String"))]
pub struct BackendName(String);

impl BackendName {
    /// Validates `name`: an empty name is [`ParseIdErrorKind::EmptyBackend`], a bad character is
    /// [`ParseIdErrorKind::InvalidBackend`].
    pub fn new(name: impl Into<String>) -> Result<Self, ParseIdError> {
        let name = name.into();
        if name.is_empty() {
            Err(ParseIdError::new(ParseIdErrorKind::EmptyBackend))
        } else if !valid_backend_name(&name) {
            Err(ParseIdError::new(ParseIdErrorKind::InvalidBackend))
        } else {
            Ok(Self(name))
        }
    }

    /// The name as text.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[cfg(feature = "serde")]
impl From<BackendName> for String {
    /// The name as a `String`, for serialisation.
    fn from(value: BackendName) -> String {
        value.0
    }
}

#[cfg(feature = "serde")]
impl TryFrom<String> for BackendName {
    type Error = ParseIdError;

    /// Parses like [`BackendName::new`].
    fn try_from(text: String) -> Result<Self, Self::Error> {
        Self::new(text)
    }
}

impl fmt::Display for BackendName {
    /// Writes the name unchanged.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl FromStr for BackendName {
    type Err = ParseIdError;

    /// Parses like [`BackendName::new`].
    fn from_str(text: &str) -> Result<Self, Self::Err> {
        Self::new(text)
    }
}

/// The checks shared by parsing and `try_new`, in order: backend present, backend valid, native present.
fn check_parts(backend: &str, native: &str) -> Result<(), ParseIdError> {
    if backend.is_empty() {
        return Err(ParseIdError::new(ParseIdErrorKind::EmptyBackend));
    }
    if !valid_backend_name(backend) {
        return Err(ParseIdError::new(ParseIdErrorKind::InvalidBackend));
    }
    if native.is_empty() {
        return Err(ParseIdError::new(ParseIdErrorKind::EmptyNative));
    }
    Ok(())
}

macro_rules! namespaced_type {
    ($(#[$doc:meta])* $name:ident) => {
        $(#[$doc])*
        #[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
        #[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
        #[cfg_attr(feature = "serde", serde(into = "String", try_from = "String"))]
        pub struct $name {
            backend: String,
            native: String,
        }

        impl $name {
            /// With `backend` and its native handle. Does **not** validate: use [`str::parse`] or
            /// [`Self::try_new`] for text from outside. A value built here from an invalid backend
            /// name or an empty native part does not round-trip through its text form.
            pub fn new(backend: impl Into<String>, native: impl Into<String>) -> Self {
                Self { backend: backend.into(), native: native.into() }
            }

            /// [`Self::new`] that checks what parsing checks, so the result always round-trips.
            pub fn try_new(backend: impl Into<String>, native: impl Into<String>) -> Result<Self, ParseIdError> {
                let (backend, native) = (backend.into(), native.into());
                check_parts(&backend, &native)?;
                Ok(Self { backend, native })
            }

            /// The owning backend's registry name.
            pub fn backend(&self) -> &str {
                &self.backend
            }

            /// The backend's own handle.
            pub fn native(&self) -> &str {
                &self.native
            }

            /// The text form, `"<backend>:<native>"`.
            pub fn to_id_string(&self) -> String {
                namespaced_id(&self.backend, &self.native)
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                write!(f, "{}:{}", self.backend, self.native)
            }
        }

        #[cfg(feature = "serde")]
        impl From<$name> for String {
            fn from(value: $name) -> String {
                value.to_string()
            }
        }

        #[cfg(feature = "serde")]
        impl TryFrom<String> for $name {
            type Error = ParseIdError;

            fn try_from(text: String) -> Result<Self, Self::Error> {
                text.parse()
            }
        }

        impl FromStr for $name {
            type Err = ParseIdError;

            fn from_str(text: &str) -> Result<Self, Self::Err> {
                let (backend, native) = split_id(text).ok_or(ParseIdError::new(ParseIdErrorKind::MissingSeparator))?;
                Self::try_new(backend, native)
            }
        }
    };
}

namespaced_type!(
    /// The typed id of a job a device has accepted: the owning backend's registry name and that
    /// backend's own handle for the job. Text form `"<backend>:<native>"`; `Display` and
    /// `FromStr` round-trip it.
    JobId
);

namespaced_type!(
    /// The typed id of a printer or scanner, in the same `"<backend>:<native>"` shape as [`JobId`].
    DeviceId
);

#[cfg(test)]
mod tests {
    use super::*;

    fn kind(text: &str) -> ParseIdErrorKind {
        text.parse::<JobId>().map(|_| ()).expect_err("must not parse").kind()
    }

    #[test]
    fn round_trips() {
        for text in [
            "wsd:job-1",
            "ipp:http://10.0.0.2:631/ipp/print#7",
            "sane:pixma:usb:04a9:1234",
            "a:b",
            "x_y-9:\u{00e9}\u{4e2d}",
            "w:  spaced  ",
            "w:a:",
            "w::",
        ] {
            let id: JobId = text.parse().expect(text);
            assert_eq!(id.to_string(), text);
            assert_eq!(id.to_id_string(), text);
            let dev: DeviceId = text.parse().expect(text);
            assert_eq!(dev.to_string(), text);
        }
    }

    #[test]
    fn only_the_first_colon_separates() {
        let id: JobId = "wsd:urn:uuid:1:2".parse().expect("parses");
        assert_eq!((id.backend(), id.native()), ("wsd", "urn:uuid:1:2"));
        let id: JobId = "a::b".parse().expect("parses");
        assert_eq!(id.native(), ":b");
    }

    #[test]
    fn hostile_ids_are_rejected_with_the_right_kind() {
        assert_eq!(kind(""), ParseIdErrorKind::MissingSeparator);
        assert_eq!(kind("nocolon"), ParseIdErrorKind::MissingSeparator);
        assert_eq!(kind(":native"), ParseIdErrorKind::EmptyBackend);
        assert_eq!(kind(":"), ParseIdErrorKind::EmptyBackend);
        assert_eq!(kind("wsd:"), ParseIdErrorKind::EmptyNative);
        assert_eq!(kind("WSD:1"), ParseIdErrorKind::InvalidBackend);
        assert_eq!(kind("w sd:1"), ParseIdErrorKind::InvalidBackend);
        assert_eq!(kind(" wsd:1"), ParseIdErrorKind::InvalidBackend);
        assert_eq!(kind("ws.d:1"), ParseIdErrorKind::InvalidBackend);
        assert_eq!(kind("w/sd:1"), ParseIdErrorKind::InvalidBackend);
        assert_eq!(kind("w\u{0}sd:1"), ParseIdErrorKind::InvalidBackend);
        assert_eq!(kind("w\nsd:1"), ParseIdErrorKind::InvalidBackend);
        assert_eq!(kind("w\u{ff53}d:1"), ParseIdErrorKind::InvalidBackend, "fullwidth letters are not ASCII");
        assert_eq!(kind("\u{e9}:1"), ParseIdErrorKind::InvalidBackend);
        let long = format!("{}:", "a".repeat(100_000));
        assert_eq!(kind(&long), ParseIdErrorKind::EmptyNative, "length is not a limit, emptiness is");
    }

    #[test]
    fn try_new_checks_like_parsing_and_new_does_not() {
        assert_eq!(JobId::try_new("wsd", "1").expect("ok").to_string(), "wsd:1");
        assert_eq!(JobId::try_new("", "1").map(|_| ()), Err(ParseIdError::new(ParseIdErrorKind::EmptyBackend)));
        assert_eq!(JobId::try_new("Bad", "1").map(|_| ()), Err(ParseIdError::new(ParseIdErrorKind::InvalidBackend)));
        assert_eq!(JobId::try_new("wsd", "").map(|_| ()), Err(ParseIdError::new(ParseIdErrorKind::EmptyNative)));
        let raw = JobId::new("Bad", "");
        assert_eq!((raw.backend(), raw.native()), ("Bad", ""));
        assert!(raw.to_string().parse::<JobId>().is_err());
    }

    #[test]
    fn backend_name_validation() {
        assert_eq!(BackendName::new("wsd-2_x").expect("ok").as_str(), "wsd-2_x");
        assert_eq!("sane".parse::<BackendName>().expect("ok").to_string(), "sane");
        assert_eq!(BackendName::new("").map(|_| ()), Err(ParseIdError::new(ParseIdErrorKind::EmptyBackend)));
        assert_eq!(BackendName::new("a:b").map(|_| ()), Err(ParseIdError::new(ParseIdErrorKind::InvalidBackend)));
        assert_eq!(BackendName::new("A").map(|_| ()), Err(ParseIdError::new(ParseIdErrorKind::InvalidBackend)));
    }

    #[test]
    fn matches_the_untyped_helpers_the_existing_crates_use() {
        assert_eq!(namespaced_id("escl", "printer/1"), "escl:printer/1");
        assert_eq!(split_id("escl:printer/1"), Some(("escl", "printer/1")));
        assert_eq!(split_id("none"), None);
        assert_eq!(DeviceId::new("escl", "printer/1").to_string(), namespaced_id("escl", "printer/1"));
    }

    #[test]
    fn error_text_names_the_problem() {
        let e = "x".parse::<JobId>().expect_err("bad");
        assert!(e.to_string().contains("no colon"));
        assert!("".parse::<BackendName>().expect_err("bad").to_string().contains("no backend"));
        let boxed: Box<dyn std::error::Error> = Box::new(e);
        assert!(!boxed.to_string().is_empty());
    }

    #[test]
    fn job_and_device_ids_are_distinct_types_with_equal_text() {
        let j = JobId::new("a", "b");
        let d = DeviceId::new("a", "b");
        assert_eq!(j.to_string(), d.to_string());
        let set: std::collections::HashSet<_> = [j.clone(), j].into_iter().collect();
        assert_eq!(set.len(), 1);
    }
}
