//! What a document keeps when it uses a plugin contribution.
//!
//! The document stores a [`MissingPluginRef`] for every use of a plugin contribution, such as an
//! effect instance. When the plugin is absent the host leaves the record exactly as it is on
//! save and shows a "missing plugin" marker. Nothing here rewrites `params`, so installing the
//! plugin again restores the effect with its settings.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// `{plugin_id, contribution_id, params, plugin_version}`. Fields this release does not know
/// survive a load and a save in `extra`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct MissingPluginRef {
    pub plugin_id: String,
    pub contribution_id: String,
    /// Opaque to the host while the plugin is absent.
    pub params: Value,
    pub plugin_version: String,
    #[serde(flatten)]
    pub extra: BTreeMap<String, Value>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RefStatus {
    /// The plugin is active and has the contribution.
    Available { installed_version: String, version_differs: bool },
    /// No active plugin has this id: not installed, disabled, or faulted.
    MissingPlugin,
    /// The plugin is active and has no contribution with this id.
    MissingContribution,
    /// The plugin has the contribution, and its kind is newer than this release.
    UnsupportedKind { installed_version: String },
}

impl RefStatus {
    /// True when the document shows the "missing plugin" marker.
    pub fn is_missing(&self) -> bool {
        !matches!(self, RefStatus::Available { .. })
    }
}

impl MissingPluginRef {
    /// The text of the marker.
    pub const MARKER: &'static str = "missing plugin";

    /// Judge the reference. `plugin` is the installed version of the active plugin with this id,
    /// or `None`. `contribution` is `None` when that plugin has no contribution with this id, and
    /// `Some(supported)` otherwise.
    pub fn status(&self, plugin: Option<&str>, contribution: Option<bool>) -> RefStatus {
        let Some(version) = plugin else {
            return RefStatus::MissingPlugin;
        };
        match contribution {
            None => RefStatus::MissingContribution,
            Some(false) => RefStatus::UnsupportedKind { installed_version: version.to_owned() },
            Some(true) => RefStatus::Available { installed_version: version.to_owned(), version_differs: version != self.plugin_version },
        }
    }

    /// The text the document shows for this reference, if it is missing.
    pub fn marker(&self, status: &RefStatus) -> Option<String> {
        match status {
            RefStatus::Available { .. } => None,
            RefStatus::MissingPlugin => Some(format!("{}: {} {}", Self::MARKER, self.plugin_id, self.plugin_version)),
            RefStatus::MissingContribution => Some(format!("{}: {} has no {}", Self::MARKER, self.plugin_id, self.contribution_id)),
            RefStatus::UnsupportedKind { .. } => Some(format!("{}: {} needs a newer host", Self::MARKER, self.contribution_id)),
        }
    }
}
