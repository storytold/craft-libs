//! Schema migrations and import of older per-app settings files.

use std::path::PathBuf;

use toml_edit::DocumentMut;

use crate::document::{schema_version, set_schema_version};
use crate::error::{MigrateError, SectionError};
use crate::{Settings, SettingsFiles};

/// What [`upgrade`] found.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Upgrade {
    /// Already at the current version, or no `schema_version` key (treated as current).
    Current,
    /// Upgraded step by step from this older version.
    Migrated {
        /// The schema version the document had.
        from: u32,
    },
    /// Written by a newer build; the document was left alone.
    Newer {
        /// The schema version the document has.
        file: u32,
    },
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
    /// Parses the file's bytes into the current settings type. The list names the values the
    /// parse replaced with defaults (`section` holds the key path, e.g. `interface.uiScale`); the
    /// store reports them in [`LoadReport::section_errors`](crate::LoadReport::section_errors), the
    /// same place a `config.toml` load puts its own. An `Err` means the file is unusable and the
    /// next legacy file is tried.
    pub parse: ParseLegacy<T>,
}

/// Parses a legacy file's bytes: the value and the values reset to defaults, or why the file is unusable.
pub type ParseLegacy<T> = fn(&[u8]) -> Result<(T, Vec<SectionError>), String>;

/// A legacy file that parsed: its value, the values reset to defaults, and where it was.
pub type Imported<T> = (T, Vec<SectionError>, PathBuf);

impl<T> std::fmt::Debug for LegacyFile<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LegacyFile").field("path", &self.path).finish_non_exhaustive()
    }
}

/// Tries each legacy file in order, read through `files` (the store's, so a browser store reads its old
/// localStorage item), and returns the first one that parses, with the values it reset and its path.
pub fn import_legacy<T>(files: &dyn SettingsFiles, legacy: &[LegacyFile<T>]) -> Option<Imported<T>> {
    legacy.iter().find_map(|f| {
        let text = files.read(&f.path).ok()??;
        (f.parse)(text.as_bytes()).ok().map(|(v, errors)| (v, errors, f.path.clone()))
    })
}
