//! Reply bodies for `link.resolve`, `asset.describe` and `link.refresh`
//! (`PROTOCOL.md` section 4).
//!
//! `std`, `serde` and the sibling pure modules only.

use serde::Serialize;

use crate::moniker::Moniker;
use crate::record::AssetKind;
use crate::refresh::{FileState, Fingerprint};

/// Answer to `link.resolve`: where the last-known result is and which version it is.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Resolved {
    pub asset: Moniker,
    pub path: String,
    pub epoch: u64,
    pub size: u64,
    pub hash: String,
    pub mtime_ms: u64,
}

impl Resolved {
    /// Build the answer from a file fingerprint.
    pub fn from_fingerprint(asset: Moniker, path: &str, epoch: u64, fp: &Fingerprint) -> Self {
        Self { asset, path: path.to_string(), epoch, size: fp.size, hash: fp.hash.clone(), mtime_ms: fp.mtime_ms }
    }
}

/// Answer to `asset.describe`: enough for a hub to list an asset without opening its document.
/// Unknown values serialize as `null`.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Described {
    pub asset: Moniker,
    pub name: String,
    pub kind: AssetKind,
    pub epoch: u64,
    pub w: Option<u64>,
    pub h: Option<u64>,
    pub hash: Option<String>,
    pub thumb_path: Option<String>,
    pub modified_ms: Option<u64>,
}

/// Answer to `link.refresh`.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Refreshed {
    pub link_id: String,
    pub epoch: u64,
    pub state: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result_path: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hash: Option<String>,
}

impl Refreshed {
    /// Build the answer from a tracker state. `hash` and `result_path` are
    /// dropped when the file is missing.
    pub fn new(link_id: &str, epoch: u64, state: FileState, result_path: &str, seen: Option<&Fingerprint>) -> Self {
        let present = state != FileState::Missing;
        Self {
            link_id: link_id.to_string(),
            epoch,
            state: state.as_str(),
            result_path: present.then(|| result_path.to_string()),
            hash: seen.filter(|_| present).map(|fp| fp.hash.clone()),
        }
    }
}
