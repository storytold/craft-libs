//! Helpers for the optional `serde` feature: milliseconds for durations, and `&'static str` fields
//! (`NativeCode::domain`, `NativeCode::name`, `IndicatorKey::Other`) read back from text.
//!
//! A `&'static str` cannot be borrowed from the input, so text read from the wire is interned: the
//! well-known domains map to string literals, and anything else is leaked **once** into a small table.
//! The table holds at most [`MAX_INTERNED`] strings of at most [`MAX_LEN`] bytes, so hostile input cannot
//! grow memory without bound. Beyond the limits a domain reads back as `"other"`, a name as `None`, and
//! an indicator key as `Other("other")`.

use std::sync::{Mutex, PoisonError};
use std::time::Duration;

use serde::{Deserialize, Deserializer};

/// The most distinct strings the table holds.
pub(crate) const MAX_INTERNED: usize = 256;
/// The longest string the table accepts, in bytes.
pub(crate) const MAX_LEN: usize = 64;

const KNOWN: [&str; 9] = ["hresult", "win32", "sane", "http", "ipp", "wsd-fault", "cups", "nserror", "other"];

static TABLE: Mutex<Vec<&'static str>> = Mutex::new(Vec::new());

fn intern(text: &str) -> Option<&'static str> {
    if let Some(known) = KNOWN.iter().find(|k| **k == text) {
        return Some(known);
    }
    if text.len() > MAX_LEN {
        return None;
    }
    let mut table = TABLE.lock().unwrap_or_else(PoisonError::into_inner);
    if let Some(found) = table.iter().find(|k| **k == text) {
        return Some(found);
    }
    if table.len() >= MAX_INTERNED {
        return None;
    }
    let leaked: &'static str = Box::leak(text.to_owned().into_boxed_str());
    table.push(leaked);
    Some(leaked)
}

pub(crate) fn intern_or_other(text: &str) -> &'static str {
    intern(text).unwrap_or("other")
}

/// `NativeCode` as read: owned strings, interned on the way in.
#[derive(Deserialize)]
struct NativeCodeWire {
    domain: String,
    code: i64,
    #[serde(default)]
    name: Option<String>,
}

impl<'de> Deserialize<'de> for crate::native::NativeCode {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let wire = NativeCodeWire::deserialize(deserializer)?;
        Ok(Self { domain: intern_or_other(&wire.domain), code: wire.code, name: wire.name.and_then(|n| intern(&n)) })
    }
}

pub(crate) fn default_window() -> Duration {
    crate::tag::Fingerprint::DEFAULT_WINDOW
}

/// A [`Duration`] as whole milliseconds (`u64`); finer precision is dropped.
pub(crate) mod duration_ms {
    use std::time::Duration;

    use serde::{Deserialize, Deserializer, Serializer};

    pub(crate) fn serialize<S: Serializer>(value: &Duration, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_u64(u64::try_from(value.as_millis()).unwrap_or(u64::MAX))
    }

    pub(crate) fn deserialize<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Duration, D::Error> {
        u64::deserialize(deserializer).map(Duration::from_millis)
    }
}
