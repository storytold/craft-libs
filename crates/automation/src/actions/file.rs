//! [`ActionsFile`]: the `actions.json` form, compatible with PhotoCraft's.

use super::model::Action;

/// The actions file (PhotoCraft's `actions.json` in its preset store): `{"version": 1,
/// "actions": [...]}`, plus `sets` with the names of empty sets, left out when there are none
/// and optional on load, so PhotoCraft's files load and save unchanged. Unknown fields are
/// ignored.
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct ActionsFile {
    /// The file version, [`ActionsFile::VERSION`].
    pub version: u32,
    /// The actions in list order.
    pub actions: Vec<Action>,
    /// Sets that hold no action.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub sets: Vec<String>,
}

impl ActionsFile {
    /// The version this crate writes (PhotoCraft's).
    // pc: crates/engine/src/preset_store.rs:1112.
    pub const VERSION: u32 = 1;

    /// Reads a file's JSON text.
    pub fn from_json(text: &str) -> Result<ActionsFile, serde_json::Error> {
        serde_json::from_str(text)
    }

    /// The file as pretty JSON text.
    pub fn to_json(&self) -> Result<String, serde_json::Error> {
        serde_json::to_string_pretty(self)
    }
}
