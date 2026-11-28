//! The presets of one kind, one entry per preset (`PresetFolder`, `PresetFolderError`): name
//! checks, the file format, and import and export by bytes. On desktop the entries are files in
//! `<config>/presets/<kind>/`; on every target, the web build included, they can live in the
//! app's settings sections instead, the store library features already save through
//! (`craft_host::SectionStore`).

use std::io;
#[cfg(not(target_arch = "wasm32"))]
use std::io::Read;
use std::marker::PhantomData;
use std::path::Path;
#[cfg(not(target_arch = "wasm32"))]
use std::path::PathBuf;
use std::sync::Arc;

use craft_host::SectionStore;
use serde::Serialize;
use serde::de::DeserializeOwned;

use crate::names;

/// The folder beside `config.toml` that holds the preset folders, one per kind.
pub const PRESETS_DIR: &str = "presets";

/// The settings section that holds presets kept with [`PresetFolder::in_settings`]: one table per
/// kind, each entry a preset's name and its file text.
///
/// ```toml
/// [presets.curve]
/// "Strong Contrast" = '{"format":"curve","version":1,"points":[[0,0],[64,48],[255,255]]}'
/// ```
pub const PRESETS_SECTION: &str = "presets";

/// The greatest size of a preset file or settings entry: 1 MiB.
pub const MAX_FILE_BYTES: u64 = 1 << 20;

/// The longest preset name, in characters (`Profiles`' limit).
pub const MAX_NAME_CHARS: usize = names::MAX_NAME_CHARS;

/// The envelope's version written by this library.
pub const FORMAT_VERSION: u32 = 1;

/// What can go wrong with a preset file.
#[derive(Debug)]
#[non_exhaustive]
pub enum PresetFolderError {
    /// The name is empty, longer than [`MAX_NAME_CHARS`], or can't be a file name (a path
    /// separator, a reserved character, a leading dot).
    BadName,
    /// No preset has that name.
    NotFound,
    /// The bytes are a preset of another kind: the envelope's `format` is `found`.
    ForeignFormat {
        /// The envelope's `format` value.
        found: String,
    },
    /// The file is larger than [`MAX_FILE_BYTES`].
    TooLarge,
    /// The bytes are not a preset of this kind (unparsable JSON, a missing field, a newer
    /// `version`).
    Unreadable(String),
    /// The folder can't be read or written; `io::ErrorKind::Unsupported` when the folder has no
    /// place on disk (`PresetFolder::new(None, ..)`). Presets kept in settings never give it: a
    /// section write can't fail, and the settings store reports its own save errors.
    Io(io::Error),
}

impl std::fmt::Display for PresetFolderError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::BadName => f.write_str("the preset name is not valid"),
            Self::NotFound => f.write_str("no preset has that name"),
            Self::ForeignFormat { found } => write!(f, "the file holds {found} presets"),
            Self::TooLarge => f.write_str("the preset file is too large"),
            Self::Unreadable(why) => write!(f, "the preset can't be read: {why}"),
            Self::Io(e) => write!(f, "the preset could not be read or written: {e}"),
        }
    }
}

impl std::error::Error for PresetFolderError {}

impl From<io::Error> for PresetFolderError {
    fn from(e: io::Error) -> Self {
        if e.kind() == io::ErrorKind::NotFound { Self::NotFound } else { Self::Io(e) }
    }
}

/// The presets of one kind, one entry per preset. For SoundCraft's plugin presets, LightCraft's
/// curve presets and FilmCraft's export presets.
///
/// - **Where.** [`new`](Self::new) (desktop): one JSON file per preset in
///   `<config>/presets/<kind>/<name>.json`, next to the settings file as `Profiles`
///   (`<config>/profiles`) and PhotoCraft's brush store (`<config dir>/Presets`), so portable mode
///   carries them and users can copy the files. [`in_settings`](Self::in_settings) (every target):
///   one entry per preset in the [`PRESETS_SECTION`] settings section, through the same
///   `SectionStore` the app gives its `Host` (`SharedSettings` on desktop; on the web the app's
///   store over browser storage). The web build has no file system, so a web app keeps its
///   presets there. A `Host` whose sections are a `MemorySections` keeps them until the page
///   closes. On desktop, prefer files: settings put every preset inside the settings file.
/// - **File format.** `{"format": "<kind>", "version": 1, ...fields of T}`: `T` serializes as a
///   map whose fields sit beside the two envelope keys (SoundCraft's plugin preset shape).
///   A file without `version` reads as version 1. `T` may not have fields named `format` or `version`. Unknown fields are ignored on load. A settings entry is the same text as a string, so
///   [`export`](Self::export) gives the same bytes in both places and a preset moves between them
///   unchanged.
/// - **Names** compare without case. [`list`](Self::list) sorts them ignoring case;
///   [`save`](Self::save) replaces a preset whose name matches ignoring case;
///   [`import`](Self::import) takes the file's stem as the name and numbers a taken one ("X 2",
///   "X 3", …). The name rules are the same in settings, so every preset can be exported as
///   `<name>.json`.
/// - **Writes.** A file is written atomically (`craft_atomic::atomic_write`). A settings write
///   reads the section, changes this kind's table and writes the section back
///   (`SectionStore::write`); the settings store saves it with its next save. Reads skip a
///   corrupt or oversized file or entry and leave it where it is.
/// - **No folder** (`new(None, ..)`): an empty list, and every write is
///   `Io(ErrorKind::Unsupported)`.
/// - **Threads.** One save, load or delete the user asked for may run in the pass (one small
///   file or one section write). [`list`](Self::list) on a folder reads every file of the folder:
///   call it at start-up or from a job, not every pass. In settings it reads the section only.
pub struct PresetFolder<T> {
    pub(crate) place: PresetPlace,
    pub(crate) kind: String,
    pub(crate) value: PhantomData<fn() -> T>,
}

/// Where a [`PresetFolder`] keeps its entries.
pub(crate) enum PresetPlace {
    /// `<config>/presets/<kind>`; `None` without a config folder.
    #[cfg(not(target_arch = "wasm32"))]
    Folder(Option<PathBuf>),
    /// This kind's table in the [`PRESETS_SECTION`] section of `store`.
    Settings(Arc<dyn SectionStore>),
}

impl<T: Serialize + DeserializeOwned> PresetFolder<T> {
    /// The presets of `kind` ("plugin", "curve", "export") as files under `config_dir`, the
    /// settings store's folder; `None` keeps nothing. Desktop only.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn new(config_dir: Option<&Path>, kind: &str) -> Self {
        Self { place: PresetPlace::Folder(config_dir.map(|d| d.join(PRESETS_DIR).join(kind))), kind: kind.to_owned(), value: PhantomData }
    }

    /// The presets of `kind` as entries of the [`PRESETS_SECTION`] settings section in
    /// `settings`: the store the app gave its `Host` (`HostBuilder::settings`). Every target; the
    /// web build's only place for presets.
    pub fn in_settings(settings: Arc<dyn SectionStore>, kind: &str) -> Self {
        Self { place: PresetPlace::Settings(settings), kind: kind.to_owned(), value: PhantomData }
    }

    /// The folder, `<config>/presets/<kind>`; `None` without a config folder and for presets
    /// kept in settings.
    pub fn dir(&self) -> Option<&Path> {
        match &self.place {
            #[cfg(not(target_arch = "wasm32"))]
            PresetPlace::Folder(dir) => dir.as_deref(),
            PresetPlace::Settings(_) => None,
        }
    }

    /// The kind, as written in each file's `format`.
    pub fn kind(&self) -> &str {
        &self.kind
    }

    /// The names of the readable presets, sorted ignoring case. A missing folder or table is an
    /// empty list; corrupt and oversized files or entries are left out (and left where they are).
    pub fn list(&self) -> Vec<String> {
        let mut names: Vec<String> =
            self.entry_names().into_iter().filter(|name| self.read_entry(name).and_then(|bytes| self.from_bytes(&bytes)).is_ok()).collect();
        names.sort_by_cached_key(|n| n.to_lowercase());
        names
    }

    /// Reads preset `name` (matched ignoring case).
    pub fn load(&self, name: &str) -> Result<T, PresetFolderError> {
        let stored = self.find(name)?;
        self.from_bytes(&self.read_entry(&stored)?)
    }

    /// Writes `value` as preset `name`, replacing a preset whose name matches ignoring case (the
    /// file or entry takes the new spelling). Creates the folder or the kind's table when needed.
    pub fn save(&self, name: &str, value: &T) -> Result<(), PresetFolderError> {
        let bytes = self.to_bytes(value)?;
        self.put(name, &bytes)
    }

    /// Deletes preset `name` (matched ignoring case).
    pub fn delete(&self, name: &str) -> Result<(), PresetFolderError> {
        let stored = self.find(name)?;
        self.remove_entry(&stored)
    }

    /// Renames preset `from` to `to`; `BadName` when `to` is not valid or another preset has it
    /// ignoring case (a change of case alone is allowed).
    pub fn rename(&self, from: &str, to: &str) -> Result<(), PresetFolderError> {
        let to = checked_name(to)?;
        let stored = self.find(from)?;
        if self.entry_names().iter().any(|n| *n != stored && n.to_lowercase() == to.to_lowercase()) {
            return Err(PresetFolderError::BadName);
        }
        let bytes = self.read_entry(&stored)?;
        self.put_replacing(&stored, &to, &bytes)
    }

    /// The file of preset `name` as bytes (a settings entry's text), for an export the user saves
    /// with `Files`.
    pub fn export(&self, name: &str) -> Result<Vec<u8>, PresetFolderError> {
        let stored = self.find(name)?;
        self.read_entry(&stored)
    }

    /// Imports the bytes of a file the user picked: checks the size and the envelope's `format`,
    /// takes `file_name`'s stem as the name, numbers a taken one, saves it, and returns the name.
    pub fn import(&self, file_name: &str, bytes: &[u8]) -> Result<String, PresetFolderError> {
        self.from_bytes(bytes)?;
        let stem = Path::new(file_name).file_stem().and_then(|s| s.to_str()).unwrap_or_default();
        let base = checked_name(stem)?;
        let taken: Vec<String> = self.entry_names().iter().map(|n| n.to_lowercase()).collect();
        let name = std::iter::once(base.clone())
            .chain((2..).map(|n| format!("{base} {n}")))
            .find(|candidate| !taken.contains(&candidate.to_lowercase()))
            .unwrap_or(base);
        self.put(&name, bytes)?;
        Ok(name)
    }

    /// `value` in the file format, for a copy or a host-side export.
    pub fn to_bytes(&self, value: &T) -> Result<Vec<u8>, PresetFolderError> {
        let unreadable = |e: serde_json::Error| PresetFolderError::Unreadable(e.to_string());
        let serde_json::Value::Object(fields) = serde_json::to_value(value).map_err(unreadable)? else {
            return Err(PresetFolderError::Unreadable("the preset is not a map of fields".into()));
        };
        let mut envelope = serde_json::Map::new();
        if fields.contains_key("format") || fields.contains_key("version") {
            return Err(PresetFolderError::Unreadable("the preset has a field named format or version, which the file format reserves".into()));
        }
        envelope.insert("format".into(), self.kind.clone().into());
        envelope.insert("version".into(), FORMAT_VERSION.into());
        envelope.extend(fields);
        let bytes = serde_json::to_vec(&envelope).map_err(unreadable)?;
        if bytes.len() as u64 > MAX_FILE_BYTES {
            return Err(PresetFolderError::TooLarge);
        }
        Ok(bytes)
    }

    /// A value of this kind from the file format.
    pub fn from_bytes(&self, bytes: &[u8]) -> Result<T, PresetFolderError> {
        if bytes.len() as u64 > MAX_FILE_BYTES {
            return Err(PresetFolderError::TooLarge);
        }
        let unreadable = |e: serde_json::Error| PresetFolderError::Unreadable(e.to_string());
        let serde_json::Value::Object(mut fields) = serde_json::from_slice(bytes).map_err(unreadable)? else {
            return Err(PresetFolderError::Unreadable("the file is not a preset".into()));
        };
        match fields.remove("format") {
            Some(serde_json::Value::String(found)) if found == self.kind => {}
            Some(serde_json::Value::String(found)) => return Err(PresetFolderError::ForeignFormat { found }),
            _ => return Err(PresetFolderError::Unreadable("the file has no format".into())),
        }
        match fields.remove("version") {
            // SoundCraft's files were written before the envelope had a version.
            None => {}
            Some(version) => match version.as_u64() {
                Some(v) if v <= u64::from(FORMAT_VERSION) => {}
                Some(v) => return Err(PresetFolderError::Unreadable(format!("it was saved by a newer version (format {v})"))),
                None => return Err(PresetFolderError::Unreadable("the file's version is not a number".into())),
            },
        }
        serde_json::from_value(serde_json::Value::Object(fields)).map_err(unreadable)
    }

    /// The names of every entry as stored, readable or not.
    fn entry_names(&self) -> Vec<String> {
        match &self.place {
            #[cfg(not(target_arch = "wasm32"))]
            PresetPlace::Folder(dir) => {
                let Some(dir) = dir else { return Vec::new() };
                let Ok(read) = std::fs::read_dir(dir) else { return Vec::new() };
                read.filter_map(Result::ok)
                    .filter_map(|entry| {
                        let path = entry.path();
                        if !path.extension()?.to_str()?.eq_ignore_ascii_case(EXTENSION) || !path.is_file() {
                            return None;
                        }
                        Some(path.file_stem()?.to_str()?.to_owned())
                    })
                    .collect()
            }
            PresetPlace::Settings(store) => {
                self.kind_table(store.as_ref()).map(|t| t.into_iter().filter(|(_, v)| v.is_str()).map(|(k, _)| k).collect()).unwrap_or_default()
            }
        }
    }

    /// The stored spelling of the entry whose name matches `name` ignoring case.
    fn find(&self, name: &str) -> Result<String, PresetFolderError> {
        let wanted = checked_name(name)?.to_lowercase();
        self.entry_names().into_iter().find(|n| n.to_lowercase() == wanted).ok_or(PresetFolderError::NotFound)
    }

    /// The text of the entry stored as `stored`; `TooLarge` over [`MAX_FILE_BYTES`].
    fn read_entry(&self, stored: &str) -> Result<Vec<u8>, PresetFolderError> {
        match &self.place {
            #[cfg(not(target_arch = "wasm32"))]
            PresetPlace::Folder(dir) => {
                let dir = dir.as_deref().ok_or(PresetFolderError::NotFound)?;
                let mut bytes = Vec::new();
                std::fs::File::open(entry_path(dir, stored))?.take(MAX_FILE_BYTES + 1).read_to_end(&mut bytes)?;
                if bytes.len() as u64 > MAX_FILE_BYTES {
                    return Err(PresetFolderError::TooLarge);
                }
                Ok(bytes)
            }
            PresetPlace::Settings(store) => {
                let text = self
                    .kind_table(store.as_ref())
                    .and_then(|mut t| t.remove(stored))
                    .and_then(|v| v.as_str().map(str::to_owned))
                    .ok_or(PresetFolderError::NotFound)?;
                if text.len() as u64 > MAX_FILE_BYTES {
                    return Err(PresetFolderError::TooLarge);
                }
                Ok(text.into_bytes())
            }
        }
    }

    /// Writes `bytes` as `name`, replacing a preset whose name matches ignoring case.
    fn put(&self, name: &str, bytes: &[u8]) -> Result<(), PresetFolderError> {
        let name = checked_name(name)?;
        let wanted = name.to_lowercase();
        let old = self.entry_names().into_iter().find(|n| n.to_lowercase() == wanted);
        self.put_replacing(old.as_deref().unwrap_or(&name), &name, bytes)
    }

    /// Writes `bytes` as `to`, dropping the entry stored as `old` when its spelling differs.
    fn put_replacing(&self, old: &str, to: &str, bytes: &[u8]) -> Result<(), PresetFolderError> {
        match &self.place {
            #[cfg(not(target_arch = "wasm32"))]
            PresetPlace::Folder(dir) => {
                let dir = dir.as_deref().ok_or_else(unsupported)?;
                std::fs::create_dir_all(dir)?;
                // `rename` also works for a change of case on a case-insensitive file system, where
                // removing the old spelling after the write would remove the new file.
                if old != to && entry_path(dir, old).exists() {
                    std::fs::rename(entry_path(dir, old), entry_path(dir, to))?;
                }
                craft_atomic::atomic_write(&entry_path(dir, to), bytes)?;
                Ok(())
            }
            PresetPlace::Settings(store) => {
                let text = String::from_utf8(bytes.to_vec()).map_err(|_| PresetFolderError::Unreadable("the preset is not text".into()))?;
                let mut section = store.read(PRESETS_SECTION).unwrap_or_default();
                let mut table = self.kind_table(store.as_ref()).unwrap_or_default();
                if old != to {
                    table.remove(old);
                }
                table.insert(to.to_owned(), toml::Value::String(text));
                section.insert(self.kind.clone(), toml::Value::Table(table));
                store.write(PRESETS_SECTION, section);
                Ok(())
            }
        }
    }

    fn remove_entry(&self, stored: &str) -> Result<(), PresetFolderError> {
        match &self.place {
            #[cfg(not(target_arch = "wasm32"))]
            PresetPlace::Folder(dir) => {
                let dir = dir.as_deref().ok_or(PresetFolderError::NotFound)?;
                std::fs::remove_file(entry_path(dir, stored))?;
                Ok(())
            }
            PresetPlace::Settings(store) => {
                let mut section = store.read(PRESETS_SECTION).unwrap_or_default();
                let mut table = self.kind_table(store.as_ref()).unwrap_or_default();
                if table.remove(stored).is_none() {
                    return Err(PresetFolderError::NotFound);
                }
                section.insert(self.kind.clone(), toml::Value::Table(table));
                store.write(PRESETS_SECTION, section);
                Ok(())
            }
        }
    }

    /// This kind's table in the section, if there is one.
    fn kind_table(&self, store: &dyn SectionStore) -> Option<toml::Table> {
        match store.read(PRESETS_SECTION)?.remove(&self.kind)? {
            toml::Value::Table(t) => Some(t),
            _ => None,
        }
    }
}

/// The file extension of a preset file.
#[cfg(not(target_arch = "wasm32"))]
const EXTENSION: &str = "json";

#[cfg(not(target_arch = "wasm32"))]
fn entry_path(dir: &Path, name: &str) -> PathBuf {
    dir.join(format!("{name}.{EXTENSION}"))
}

#[cfg(not(target_arch = "wasm32"))]
fn unsupported() -> PresetFolderError {
    PresetFolderError::Io(io::Error::new(io::ErrorKind::Unsupported, "no preset folder"))
}

/// `name` trimmed, or `BadName` when it can't be a file name on every platform.
fn checked_name(name: &str) -> Result<String, PresetFolderError> {
    names::checked_name(name).map_err(|_| PresetFolderError::BadName)
}

#[cfg(test)]
mod tests {
    use serde::Deserialize;

    use super::*;

    #[derive(Debug, PartialEq, Serialize, Deserialize)]
    struct Curve {
        points: Vec<(u8, u8)>,
    }

    fn folder(dir: &Path) -> PresetFolder<Curve> {
        PresetFolder::new(Some(dir), "curve")
    }

    // A SoundCraft-shaped file (no `version`) loads and lists; a `T` with a `version` field is refused.
    #[test]
    fn a_file_without_a_version_reads_as_version_one_and_a_clashing_field_is_refused() {
        let tmp = tempfile::tempdir().unwrap();
        let presets = folder(tmp.path());
        std::fs::create_dir_all(presets.dir().unwrap()).unwrap();
        std::fs::write(presets.dir().unwrap().join("old.json"), br#"{"format":"curve","points":[[0,0],[9,9]]}"#).unwrap();
        assert_eq!(presets.list(), ["old"]);
        assert_eq!(presets.load("old").unwrap(), Curve { points: vec![(0, 0), (9, 9)] });
        let bad = br#"{"format":"curve","version":"x","points":[]}"#;
        assert!(matches!(presets.from_bytes(bad), Err(PresetFolderError::Unreadable(_))));

        #[derive(Serialize, serde::Deserialize)]
        struct Clash {
            version: u32,
        }
        let clash = PresetFolder::<Clash>::new(Some(tmp.path()), "clash");
        assert!(matches!(clash.save("x", &Clash { version: 3 }), Err(PresetFolderError::Unreadable(_))));
    }
}
