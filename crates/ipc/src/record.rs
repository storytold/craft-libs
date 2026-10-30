//! The `craft.link` project item record.
//!
//! Schema: `schemas/link-record.schema.json`. A consumer stores one of these
//! on the linked media item. Records written before the asset fields existed
//! (FilmCraft to EffectCraft) parse and validate unchanged. Unknown JSON fields
//! stay in the `extra` maps and come back out on save.
//!
//! This module uses only `std` and `serde`, so an app can copy it to a wasm build.

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::error::LinkError;
use crate::ids::{valid_app_id, valid_uuid};
use crate::moniker::Moniker;

/// What the linked asset is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AssetKind {
    Still,
    Vector,
    Sequence,
    Comp,
}

impl AssetKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Still => "still",
            Self::Vector => "vector",
            Self::Sequence => "sequence",
            Self::Comp => "comp",
        }
    }
}

/// The linked asset: its kind and its durable id.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AssetRef {
    pub kind: AssetKind,
    /// A moniker string. [`LinkRecord::validate`] parses it.
    pub moniker: String,
    #[serde(default, flatten)]
    pub extra: Map<String, Value>,
}

/// Who wrote the result.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Producer {
    pub app: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub app_version: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub doc_id: Option<String>,
    #[serde(default, flatten)]
    pub extra: Map<String, Value>,
}

/// Size of the linked result. The video fields are absent for stills.
///
/// `rate_num` and `rate_den` come as a pair.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Format {
    pub w: u64,
    pub h: u64,
    pub alpha: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rate_num: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rate_den: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub duration_ticks: Option<u64>,
    #[serde(default, flatten)]
    pub extra: Map<String, Value>,
}

/// One link from a consumer media item to a producer asset.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LinkRecord {
    pub v: u32,
    pub link_id: String,
    pub target_app: String,
    pub target_project: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target_comp: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub nested_sequence: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub asset: Option<AssetRef>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub producer: Option<Producer>,
    pub result_path: String,
    pub result_epoch: u64,
    pub format: Format,
    pub created: String,
    #[serde(default, flatten)]
    pub extra: Map<String, Value>,
}

impl LinkRecord {
    /// Check the record rules. Deserialization can accept a record this rejects.
    pub fn validate(&self) -> Result<(), LinkError> {
        if self.v != 1 {
            return Err(invalid(format!("v is {}, want 1", self.v)));
        }
        if !valid_uuid(&self.link_id) {
            return Err(invalid("link_id is not a uuid".into()));
        }
        validate_app("target_app", &self.target_app)?;
        require_nonempty("target_project", &self.target_project)?;
        require_nonempty("result_path", &self.result_path)?;
        if let Some(comp) = &self.target_comp {
            require_nonempty("target_comp", comp)?;
        }
        if let Some(nest) = &self.nested_sequence {
            require_nonempty("nested_sequence", nest)?;
        }
        if self.target_comp.is_none() && self.asset.is_none() {
            return Err(invalid("a record needs target_comp or asset".into()));
        }
        if let Some(asset) = &self.asset {
            Moniker::parse(&asset.moniker).map_err(|err| invalid(format!("asset.moniker: {err}")))?;
        }
        if let Some(producer) = &self.producer {
            validate_app("producer.app", &producer.app)?;
        }
        self.format.validate()?;
        if !valid_rfc3339(&self.created) {
            return Err(invalid("created is not an RFC 3339 date-time".into()));
        }
        Ok(())
    }
}

impl Format {
    fn validate(&self) -> Result<(), LinkError> {
        require_at_least_one("format.w", self.w)?;
        require_at_least_one("format.h", self.h)?;
        match (self.rate_num, self.rate_den) {
            (Some(num), Some(den)) => {
                require_at_least_one("format.rate_num", num)?;
                require_at_least_one("format.rate_den", den)
            }
            (None, None) => Ok(()),
            _ => Err(invalid("format.rate_num and format.rate_den come as a pair".into())),
        }
    }
}

fn invalid(message: String) -> LinkError {
    LinkError::Invalid(message)
}

fn validate_app(name: &str, app: &str) -> Result<(), LinkError> {
    if valid_app_id(app) { Ok(()) } else { Err(invalid(format!("{name} {app:?} must match [a-z][a-z0-9-]{{1,31}}"))) }
}

fn require_nonempty(name: &str, value: &str) -> Result<(), LinkError> {
    if value.is_empty() { Err(invalid(format!("{name} is empty"))) } else { Ok(()) }
}

fn require_at_least_one(name: &str, value: u64) -> Result<(), LinkError> {
    if value >= 1 { Ok(()) } else { Err(invalid(format!("{name} must be >= 1"))) }
}

/// `YYYY-MM-DDTHH:MM:SS` with an optional fraction and a `Z` or numeric offset.
fn valid_rfc3339(text: &str) -> bool {
    let bytes = text.as_bytes();
    if bytes.len() < 20 {
        return false;
    }
    let (Some(month), Some(day), Some(hour), Some(minute), Some(second)) =
        (two_digits(bytes, 5), two_digits(bytes, 8), two_digits(bytes, 11), two_digits(bytes, 14), two_digits(bytes, 17))
    else {
        return false;
    };
    if !four_digits(bytes, 0)
        || byte_at(bytes, 4) != Some(b'-')
        || byte_at(bytes, 7) != Some(b'-')
        || !matches!(byte_at(bytes, 10), Some(b'T' | b't'))
        || byte_at(bytes, 13) != Some(b':')
        || byte_at(bytes, 16) != Some(b':')
    {
        return false;
    }
    if !(1..=12).contains(&month) || !(1..=31).contains(&day) || hour > 23 || minute > 59 || second > 60 {
        return false;
    }
    let mut index = 19;
    if byte_at(bytes, index) == Some(b'.') {
        index += 1;
        let start = index;
        while byte_at(bytes, index).is_some_and(|byte| byte.is_ascii_digit()) {
            index += 1;
        }
        if index == start {
            return false;
        }
    }
    match byte_at(bytes, index) {
        Some(b'Z' | b'z') => index + 1 == bytes.len(),
        Some(b'+' | b'-') => numeric_offset(bytes, index + 1),
        _ => false,
    }
}

fn numeric_offset(bytes: &[u8], index: usize) -> bool {
    if bytes.len() != index + 5 || byte_at(bytes, index + 2) != Some(b':') {
        return false;
    }
    matches!((two_digits(bytes, index), two_digits(bytes, index + 3)), (Some(hour), Some(minute)) if hour <= 23 && minute <= 59)
}

fn four_digits(bytes: &[u8], index: usize) -> bool {
    bytes.get(index..index + 4).is_some_and(|slice| slice.iter().all(u8::is_ascii_digit))
}

fn two_digits(bytes: &[u8], index: usize) -> Option<u16> {
    let slice = bytes.get(index..index + 2)?;
    let hi = slice.first().copied()?;
    let lo = slice.get(1).copied()?;
    if !hi.is_ascii_digit() || !lo.is_ascii_digit() {
        return None;
    }
    Some(u16::from(hi - b'0') * 10 + u16::from(lo - b'0'))
}

fn byte_at(bytes: &[u8], index: usize) -> Option<u8> {
    bytes.get(index).copied()
}
