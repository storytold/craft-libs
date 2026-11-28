#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

//! Shared settings storage for the Craft suite.
//!
//! Each application keeps its own settings type and defaults. This crate decides where the
//! `config.toml` lives, loads it without ever failing, saves it atomically while the
//! application runs, keeps a backup, moves damaged files aside and runs schema migrations.
//!
//! Hand edits survive: the file is kept in memory as a [`toml_edit::DocumentMut`] and saves
//! merge values into it, so comments and keys the application does not know stay in place.
//!
//! ```no_run
//! use craft_settings::{Location, Settings, SettingsStore};
//! use serde::{Deserialize, Serialize};
//!
//! #[derive(Serialize, Deserialize, Default, Clone, PartialEq)]
//! #[serde(default)]
//! struct Prefs {
//!     appearance: Appearance,
//! }
//!
//! #[derive(Serialize, Deserialize, Default, Clone, PartialEq)]
//! #[serde(default)]
//! struct Appearance {
//!     theme: String,
//! }
//!
//! impl Settings for Prefs {
//!     const APP_ID: &'static str = "photocraft";
//!     const SCHEMA_VERSION: u32 = 1;
//! }
//!
//! let (mut store, report) = SettingsStore::<Prefs>::open(Location::Auto);
//! store.update(|p| p.appearance.theme = "dark".into());
//! // Call `store.tick(web_time::Instant::now())` every frame and `store.flush()` on exit.
//! ```
#![warn(missing_docs)]

pub mod document;
pub mod error;
pub mod files;
pub mod migrate;
mod names;
pub mod presets;
pub mod remember;

pub mod paths;
#[cfg(not(target_arch = "wasm32"))]
pub mod profiles;
pub mod recovery;
pub mod store;
#[cfg(all(feature = "testing", not(target_arch = "wasm32")))]
pub mod testing;

pub use document::MergeRule;
pub use error::{LoadReport, MigrateError, ReadOnlyReason, SaveStatus, SectionError, SettingsError, Source, TickResult};
#[cfg(not(target_arch = "wasm32"))]
pub use files::DiskFiles;
#[cfg(target_arch = "wasm32")]
pub use files::LocalStorageFiles;
/// The browser's settings storage under its former name.
#[cfg(target_arch = "wasm32")]
#[deprecated(note = "use `LocalStorageFiles::new(cap)`")]
pub type BrowserFiles = LocalStorageFiles;
pub use files::SettingsFiles;
pub use migrate::LegacyFile;
pub use paths::{Location, Portable, Probe, Resolved, Rule};
#[cfg(not(target_arch = "wasm32"))]
pub use profiles::{ProfileError, Profiles};
pub use remember::{Remembered, RememberedSection};
pub use store::{OpenOptions, SettingsStore, SharedSettings};
pub use toml_edit::DocumentMut;

use serde::Serialize;
use serde::de::DeserializeOwned;

/// Suite-default portable names, kept here so the trait also compiles on wasm.
mod paths_default {
    pub const MARKERS: &[&str] = &["portable"];
    pub const DIR: &str = "config";
}

/// An application's settings type.
///
/// Derive `Serialize`, `Deserialize`, `Default`, `Clone` and `PartialEq`, and put
/// `#[serde(default)]` on every struct so missing keys fall back to defaults. Do not use
/// `#[serde(deny_unknown_fields)]`: unknown keys are kept in the file on purpose.
pub trait Settings: Serialize + DeserializeOwned + Default + Clone + PartialEq {
    /// Lowercase application id, used for the folder name and the `<APP>_CONFIG_DIR` variable.
    const APP_ID: &'static str;

    /// Name of the platform config folder (and the current-directory fallback). Override to keep
    /// a folder that existing settings already sit in, e.g. `Photocraft` on macOS and Windows.
    const FOLDER_NAME: &'static str = Self::APP_ID;

    /// Version written as `schema_version` at the top of `config.toml`.
    const SCHEMA_VERSION: u32;

    /// File names next to the executable that turn on portable mode (any one; contents ignored).
    /// Override to keep an existing portable layout, e.g. `&["portable.txt", "PhotoCraft.portable"]`.
    const PORTABLE_MARKERS: &'static [&'static str] = paths_default::MARKERS;

    /// Folder beside the executable that holds settings in portable mode.
    const PORTABLE_DIR: &'static str = paths_default::DIR;

    /// Upgrade a document from `from` to `from + 1`. Called once per step for files whose
    /// `schema_version` is older than [`Settings::SCHEMA_VERSION`], before typed parsing.
    fn migrate(from: u32, doc: &mut DocumentMut) -> Result<(), MigrateError> {
        let _ = (from, doc);
        Ok(())
    }

    /// The rule for merging the item at `path` when another process wrote the file meanwhile. The default
    /// merges key by key.
    fn merge_rule(path: &[&str]) -> MergeRule {
        let _ = path;
        MergeRule::ByKey
    }
}

#[cfg(test)]
extern crate self as craft_settings;

#[cfg(test)]
#[path = "../tests/main.rs"]
mod integration;
