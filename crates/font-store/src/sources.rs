//! `sources.json`: the built-in indexes, each with its size and sha256.

use std::collections::HashSet;

use serde::Deserialize;

use crate::index::{Index, IndexError, MAX_INDEX_BYTES, gunzip};
use crate::source::{MAX_STR, check_path, check_str, check_version, invalid, is_sha256};
use crate::verify::sha256_hex;

/// The largest `sources.json` read.
pub(crate) const MAX_SOURCES_BYTES: usize = 1 << 20;
const MAX_SOURCES: usize = 64;

/// One built-in index of `sources.json`.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
pub struct SourceEntry {
    /// The index's source ID.
    pub source: String,
    /// The source's display name.
    #[serde(default)]
    pub name: String,
    /// Whether the source is curated.
    #[serde(default)]
    pub curated: bool,
    /// The index, relative to `sources.json`.
    pub path: String,
    /// Bytes of the index.
    pub size: u64,
    /// sha256 of the index.
    pub sha256: String,
    /// The gzipped index, relative to `sources.json`.
    #[serde(default)]
    pub gz_path: Option<String>,
    /// Bytes of the gzipped index.
    #[serde(default)]
    pub gz_size: Option<u64>,
    /// sha256 of the gzipped index.
    #[serde(default)]
    pub gz_sha256: Option<String>,
    /// The commit the index was generated from.
    #[serde(default)]
    pub commit: String,
}

impl SourceEntry {
    /// The gzipped copy's path, size and sha256, when `sources.json` lists one.
    pub fn gz(&self) -> Option<(&str, u64, &str)> {
        match (&self.gz_path, self.gz_size, &self.gz_sha256) {
            (Some(p), Some(n), Some(h)) => Some((p, n, h)),
            _ => None,
        }
    }

    /// The index from the bytes of its gzipped copy (when `gz`) or of the index itself, checked
    /// against the sizes and sha256s listed here, and that it is this entry's source.
    pub fn read(&self, bytes: &[u8], gz: bool) -> Result<Index, IndexError> {
        let json = if gz {
            let (_, size, sha256) = self.gz().ok_or(IndexError::Checksum)?;
            if bytes.len() as u64 != size || sha256_hex(bytes) != sha256 {
                return Err(IndexError::Checksum);
            }
            gunzip(bytes, MAX_INDEX_BYTES)?
        } else {
            bytes.to_vec()
        };
        if json.len() as u64 != self.size || sha256_hex(&json) != self.sha256 {
            return Err(IndexError::Checksum);
        }
        let index = Index::from_json(&json)?;
        if index.sources().iter().any(|s| s.id != self.source) {
            return invalid(format!("the index at {} is not of source {}", self.path, self.source));
        }
        Ok(index)
    }
}

/// `sources.json`.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
pub struct SourcesFile {
    /// `"MAJOR.MINOR"`.
    pub version: String,
    /// The built-in indexes, in the order a client lists them.
    pub indexes: Vec<SourceEntry>,
}

impl SourcesFile {
    /// Reads and checks `sources.json`.
    pub fn from_json(bytes: &[u8]) -> Result<Self, IndexError> {
        if bytes.len() > MAX_SOURCES_BYTES {
            return Err(IndexError::TooLarge { max: MAX_SOURCES_BYTES });
        }
        let value: serde_json::Value = serde_json::from_slice(bytes).map_err(|e| IndexError::Json(e.to_string()))?;
        let version = value.get("version").and_then(|v| v.as_str()).ok_or_else(|| IndexError::Version("(none)".into()))?;
        check_version(version)?;
        let file: SourcesFile = serde_json::from_value(value).map_err(|e| IndexError::Json(e.to_string()))?;
        if file.indexes.len() > MAX_SOURCES {
            return invalid("sources.json lists too many indexes");
        }
        let mut ids = HashSet::new();
        for e in &file.indexes {
            check_str("source", &e.source)?;
            check_str("source name", &e.name)?;
            check_str("commit", &e.commit)?;
            if e.source.is_empty() || e.source.len() > MAX_STR || !ids.insert(e.source.as_str()) {
                return invalid(format!("source `{}` is empty or listed twice", e.source));
            }
            check_path(&e.path)?;
            if !is_sha256(&e.sha256) || e.size == 0 || e.size > MAX_INDEX_BYTES as u64 {
                return invalid(format!("the index of `{}` has a bad sha256 or size", e.source));
            }
            if e.gz_path.is_some() || e.gz_size.is_some() || e.gz_sha256.is_some() {
                let Some((p, n, h)) = e.gz() else { return invalid(format!("the gzipped index of `{}` is incomplete", e.source)) };
                check_path(p)?;
                if !is_sha256(h) || n == 0 || n > MAX_INDEX_BYTES as u64 {
                    return invalid(format!("the gzipped index of `{}` has a bad sha256 or size", e.source));
                }
            }
        }
        Ok(file)
    }
}

#[cfg(feature = "bundled-index")]
const BUNDLED_SOURCES: &[u8] = include_bytes!("../index/sources.json");

/// The gzipped indexes compiled in, by their path in `sources.json`.
#[cfg(feature = "bundled-index")]
const BUNDLED: [(&str, &[u8]); 2] = [
    ("indexes/google-fonts.json.gz", include_bytes!("../index/indexes/google-fonts.json.gz")),
    ("indexes/noto.json.gz", include_bytes!("../index/indexes/noto.json.gz")),
];

#[cfg(feature = "bundled-index")]
pub(crate) fn bundled() -> Result<Index, IndexError> {
    if sha256_hex(BUNDLED_SOURCES) != crate::PINNED_INDEX.sha256 {
        return Err(IndexError::Checksum);
    }
    let sources = SourcesFile::from_json(BUNDLED_SOURCES)?;
    let mut parts = Vec::new();
    for entry in &sources.indexes {
        let gz_path = entry.gz().map(|(p, _, _)| p);
        let bytes = BUNDLED.iter().find(|(p, _)| Some(*p) == gz_path).map(|(_, b)| *b);
        let bytes = bytes.ok_or_else(|| IndexError::Invalid(format!("{} is not compiled in", entry.source)))?;
        parts.push(entry.read(bytes, true)?);
    }
    Ok(Index::merge(parts))
}
