//! Small JSON records used by StencilCraft's local library and recent-files list.
//!
//! These records are separate from a document, but their on-disk field names, defaults and
//! JSON encoding are part of StencilCraft's supported persisted data.

use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::DocError;

/// One item in the local library recycle bin.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LibraryTrashEntry {
    /// File name inside the bin.
    pub name: String,
    /// Where it was, for restoring.
    pub original: String,
}

/// The `.library.json` record stored at the root of a StencilCraft library.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct LibraryIndex {
    #[serde(default)]
    pub favourites: Vec<String>,
    #[serde(default)]
    pub trash: Vec<LibraryTrashEntry>,
}

impl LibraryIndex {
    /// Parse the library index, applying defaults for fields absent in older indexes.
    pub fn from_json(json: &str) -> Result<Self, serde_json::Error> {
        serde_json::from_str(json)
    }

    /// Serialize the library index using the canonical pretty JSON representation.
    pub fn to_json_pretty(&self) -> Result<String, serde_json::Error> {
        serde_json::to_string_pretty(self)
    }
}

/// Maximum number of paths retained in the recent-files record.
pub const MAX_RECENT_FILES: usize = 10;

/// Most recently used document paths, newest first, without duplicates.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct RecentFiles {
    #[serde(default)]
    pub paths: Vec<String>,
}

impl RecentFiles {
    /// Parse recent files from their persisted JSON representation.
    pub fn from_json(json: &str) -> Result<Self, serde_json::Error> {
        serde_json::from_str(json)
    }

    /// Serialize recent files using the canonical pretty JSON representation.
    pub fn to_json_pretty(&self) -> Result<String, serde_json::Error> {
        serde_json::to_string_pretty(self)
    }

    /// Moves `path` to the front, keeping at most [`MAX_RECENT_FILES`].
    pub fn push(&mut self, path: &str) {
        self.paths.retain(|p| p != path);
        self.paths.insert(0, path.to_string());
        self.paths.truncate(MAX_RECENT_FILES);
    }

    pub fn remove(&mut self, path: &str) {
        self.paths.retain(|p| p != path);
    }

    /// Reads the list; a missing or damaged file gives an empty list.
    pub fn load(file: &Path) -> Self {
        let mut list = std::fs::read_to_string(file).ok().and_then(|text| Self::from_json(&text).ok()).unwrap_or_default();
        list.paths.truncate(MAX_RECENT_FILES);
        list
    }

    pub fn save(&self, file: &Path) -> Result<(), DocError> {
        let json = self.to_json_pretty().map_err(|e| DocError::Structure { path: file.display().to_string(), message: e.to_string() })?;
        craft_atomic::atomic_write(file, json.as_bytes()).map_err(|source| DocError::Io { path: file.display().to_string(), source })
    }
}
