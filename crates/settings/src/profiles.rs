//! [`Profiles`]: named copies of an app's settings the user can switch to, export and import.
//!
//! CadCraft keeps one settings profile and lists "named profiles with import and export" as not
//! done (its `docs/gaps.md`); this is that feature, for every app on [`SettingsStore`](crate::SettingsStore).

use std::io;
use std::path::{Path, PathBuf};

use crate::Settings;
use crate::document;
use crate::migrate::{self, Upgrade};
use crate::names;

/// The extension of a profile file.
const EXTENSION: &str = "toml";

/// Longest profile file read (CadCraft's cap); a settings file is far smaller.
const MAX_PROFILE_BYTES: u64 = 64 * 1024;

/// The folder beside `config.toml` that holds the profiles.
pub const PROFILES_DIR: &str = "profiles";

/// What can go wrong with a profile.
#[derive(Debug)]
pub enum ProfileError {
    /// The name is empty or can't be a file name (a path separator, a reserved character).
    BadName,
    /// No profile has that name.
    NotFound,
    /// The file is not this app's settings (unparsable, or a newer schema).
    Unreadable(String),
    /// Reading or writing the profile file failed.
    Io(io::Error),
}

impl std::fmt::Display for ProfileError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::BadName => f.write_str("the profile name is not valid"),
            Self::NotFound => f.write_str("no profile has that name"),
            Self::Unreadable(why) => write!(f, "the profile can't be read: {why}"),
            Self::Io(e) => write!(f, "the profile could not be written: {e}"),
        }
    }
}

impl std::error::Error for ProfileError {}

impl From<io::Error> for ProfileError {
    fn from(e: io::Error) -> Self {
        if e.kind() == io::ErrorKind::NotFound { Self::NotFound } else { Self::Io(e) }
    }
}

/// Named settings profiles in `<config>/profiles/<name>.toml`, each a full settings file with
/// `schema_version` (migrated on load like `config.toml`).
///
/// - [`save`](Self::save) stores the current settings under a name, replacing a profile of that
///   name.
/// - [`load`](Self::load) reads one; the app applies it with `store.set(value)`.
/// - [`export`](Self::export) and [`import`](Self::import) copy a profile to and from a file the
///   user picked; import takes the file's stem as the name, adding " 2", " 3", … when taken.
/// - [`to_text`](Self::to_text) is the profile as TOML for "Copy as Text" (CadCraft's Copy as JSON).
///
/// Names compare without case; [`list`](Self::list) sorts them ignoring case.
pub struct Profiles<T: Settings> {
    dir: Option<PathBuf>,
    _settings: std::marker::PhantomData<T>,
}

impl<T: Settings> Profiles<T> {
    /// The profiles of the store whose folder is `config_dir`; `None` (an in-memory store) keeps
    /// no profiles.
    pub fn new(config_dir: Option<&Path>) -> Self {
        Self { dir: config_dir.map(|d| d.join(PROFILES_DIR)), _settings: std::marker::PhantomData }
    }

    /// The profile names, sorted.
    pub fn list(&self) -> Vec<String> {
        let mut names: Vec<String> = self.entries().into_iter().map(|(name, _)| name).collect();
        names.sort_by_cached_key(|n| n.to_lowercase());
        names
    }

    /// Stores `value` as the profile `name`, replacing a profile whose name matches without case.
    pub fn save(&self, name: &str, value: &T) -> Result<(), ProfileError> {
        let dir = self.dir()?;
        let name = checked_name(name)?;
        let text = Self::to_text(value)?;
        std::fs::create_dir_all(dir)?;
        let path = file_path(dir, &name);
        // On a case-insensitive file system the old spelling is the same file as the new one.
        if let Some((old, old_path)) = self.entries().into_iter().find(|(n, _)| n.to_lowercase() == name.to_lowercase())
            && old != name
            && old_path != path
        {
            std::fs::remove_file(old_path)?;
        }
        craft_atomic::atomic_write(&path, text.as_bytes())?;
        Ok(())
    }

    // cadcraft: crates/ui-egui/src/options.rs:350 load_profile (size cap, unknown entries skipped)
    /// Reads the profile `name`. Entries that don't fit `T` fall back to their defaults.
    pub fn load(&self, name: &str) -> Result<T, ProfileError> {
        let path = self.find(name)?;
        Self::read_value(&path).map(|(value, _)| value)
    }

    /// Removes the profile `name`.
    pub fn delete(&self, name: &str) -> Result<(), ProfileError> {
        let path = self.find(name)?;
        std::fs::remove_file(path)?;
        Ok(())
    }

    /// Writes the profile `name` to `to`, replacing a file there.
    pub fn export(&self, name: &str, to: &Path) -> Result<(), ProfileError> {
        let path = self.find(name)?;
        let bytes = std::fs::read(path)?;
        craft_atomic::atomic_write(to, &bytes)?;
        Ok(())
    }

    /// Imports a settings file as a profile and returns its name.
    pub fn import(&self, from: &Path) -> Result<String, ProfileError> {
        let dir = self.dir()?;
        let (_, text) = Self::read_value(from)?;
        let stem = from.file_stem().and_then(|s| s.to_str()).unwrap_or_default();
        let base = checked_name(stem)?;
        let taken: Vec<String> = self.list().iter().map(|n| n.to_lowercase()).collect();
        let name = std::iter::once(base.clone())
            .chain((2..).map(|n| format!("{base} {n}")))
            .find(|candidate| !taken.contains(&candidate.to_lowercase()))
            .unwrap_or(base);
        std::fs::create_dir_all(dir)?;
        craft_atomic::atomic_write(&file_path(dir, &name), text.as_bytes())?;
        Ok(name)
    }

    // cadcraft: crates/ui-egui/src/options.rs:333 profile_json
    /// The profile as TOML: `schema_version` first, then the settings.
    pub fn to_text(value: &T) -> Result<String, ProfileError> {
        let mut doc = document::new_document(T::SCHEMA_VERSION);
        document::merge_into(&mut doc, value).map_err(|e| ProfileError::Unreadable(e.to_string()))?;
        Ok(document::render(&doc))
    }

    fn dir(&self) -> Result<&Path, ProfileError> {
        self.dir.as_deref().ok_or_else(|| ProfileError::Io(io::Error::new(io::ErrorKind::Unsupported, "settings are kept in memory only")))
    }

    /// The `(name, path)` of every profile file.
    fn entries(&self) -> Vec<(String, PathBuf)> {
        let Some(dir) = &self.dir else { return Vec::new() };
        let Ok(read) = std::fs::read_dir(dir) else { return Vec::new() };
        read.filter_map(Result::ok)
            .filter_map(|entry| {
                let path = entry.path();
                if path.extension()?.to_str()? != EXTENSION || !path.is_file() {
                    return None;
                }
                let name = path.file_stem()?.to_str()?.to_owned();
                Some((name, path))
            })
            .collect()
    }

    /// The file of the profile whose name matches `name` without case.
    fn find(&self, name: &str) -> Result<PathBuf, ProfileError> {
        let name = checked_name(name)?;
        let wanted = name.to_lowercase();
        self.entries().into_iter().find(|(n, _)| n.to_lowercase() == wanted).map(|(_, path)| path).ok_or(ProfileError::NotFound)
    }

    /// Parses a settings file: the value, and the file's text upgraded to the current schema.
    fn read_value(path: &Path) -> Result<(T, String), ProfileError> {
        use std::io::Read;
        let file = std::fs::File::open(path)?;
        let mut bytes = Vec::new();
        file.take(MAX_PROFILE_BYTES + 1).read_to_end(&mut bytes)?;
        if bytes.len() as u64 > MAX_PROFILE_BYTES {
            return Err(ProfileError::Unreadable("the file is too large".into()));
        }
        let text = String::from_utf8(bytes).map_err(|_| ProfileError::Unreadable("the file is not text".into()))?;
        let mut doc = document::parse(&text).map_err(|e| ProfileError::Unreadable(e.to_string()))?;
        match migrate::upgrade::<T>(&mut doc).map_err(|e| ProfileError::Unreadable(e.to_string()))? {
            Upgrade::Newer { file } => {
                return Err(ProfileError::Unreadable(format!(
                    "it was saved by a newer version (schema {file}, this one reads {})",
                    T::SCHEMA_VERSION
                )));
            }
            Upgrade::Current | Upgrade::Migrated { .. } => {}
        }
        let (value, _skipped) = document::deserialize_sections::<T>(&doc);
        Ok((value, document::render(&doc)))
    }
}

fn file_path(dir: &Path, name: &str) -> PathBuf {
    dir.join(format!("{name}.{EXTENSION}"))
}

/// `name` trimmed, or [`ProfileError::BadName`] when it can't be a file name on every platform.
fn checked_name(name: &str) -> Result<String, ProfileError> {
    names::checked_name(name).map_err(|_| ProfileError::BadName)
}
