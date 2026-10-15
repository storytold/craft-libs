//! Schema migrations and import of older per-app settings files.

use std::path::PathBuf;

use toml_edit::DocumentMut;

use crate::Settings;
use crate::document::{schema_version, set_schema_version};
use crate::error::MigrateError;

/// What [`upgrade`] found.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Upgrade {
    /// Already at the current version, or no `schema_version` key (treated as current).
    Current,
    /// Upgraded step by step from this older version.
    Migrated { from: u32 },
    /// Written by a newer build; the document was left alone.
    Newer { file: u32 },
}

/// Brings `doc` up to [`Settings::SCHEMA_VERSION`] by calling [`Settings::migrate`] once per step.
///
/// A missing `schema_version` means a hand-written file; it is taken as current and the key is
/// added. A failed step leaves the document as that step left it; callers must not save it.
pub fn upgrade<T: Settings>(doc: &mut DocumentMut) -> Result<Upgrade, MigrateError> {
    let target = T::SCHEMA_VERSION;
    let Some(file) = schema_version(doc) else {
        set_schema_version(doc, target);
        return Ok(Upgrade::Current);
    };
    if file > target {
        return Ok(Upgrade::Newer { file });
    }
    if file == target {
        return Ok(Upgrade::Current);
    }
    for step in file..target {
        T::migrate(step, doc)?;
        set_schema_version(doc, step + 1);
    }
    Ok(Upgrade::Migrated { from: file })
}

/// An older settings file an application used before `config.toml`.
///
/// The application parses it with its own code, so this crate needs no JSON or RON support.
/// The file is read, never renamed or removed, so an older build keeps working after a rollback.
pub struct LegacyFile<T> {
    /// Where the old file lives on this platform.
    pub path: PathBuf,
    /// Parses the file's bytes into the current settings type.
    pub parse: fn(&[u8]) -> Result<T, String>,
}

impl<T> std::fmt::Debug for LegacyFile<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LegacyFile").field("path", &self.path).finish_non_exhaustive()
    }
}

/// Tries each legacy file in order and returns the first one that parses, with its path.
#[cfg(not(target_arch = "wasm32"))]
pub fn import_legacy<T>(files: &[LegacyFile<T>]) -> Option<(T, PathBuf)> {
    files.iter().find_map(|f| {
        let bytes = std::fs::read(&f.path).ok()?;
        (f.parse)(&bytes).ok().map(|v| (v, f.path.clone()))
    })
}
