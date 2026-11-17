//! The per-user plug-in store: `Plug-ins/` in the app's settings folder (or beside a portable
//! install), one folder per plug-in named by a hash of its id, holding its bundle and a record of
//! where it came from.
//!
//! ```text
//! <root>/
//!   <32 hex digits: SHA-256 of the lower-case id>/
//!     <sha256>.craftplugin     the bundle, named by its SHA-256
//!     source.json              id, version, sha256 and where it came from
//! ```
//!
//! Nothing a plug-in or a download says becomes a path: folder and file names are hashes, so an id
//! or an asset name can't climb out of the store. Every write is atomic (craft-atomic), and an
//! update writes the new bundle beside the old one before `source.json` switches to it, so a crash
//! leaves either installation whole. Native builds only; the web builds keep plug-ins in browser
//! storage.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::bundle::{BUNDLE_EXTENSION, Bundle, MAX_BUNDLE_BYTES, ModuleCheck, check_command_module, open_with, sha256_hex};
use crate::github::is_hex64;
use crate::manifest::Manifest;

/// The store's folder name inside the app's settings folder.
pub const PLUGINS_DIR: &str = "Plug-ins";
/// The record of an installed plug-in.
pub const SOURCE_FILE: &str = "source.json";
const MAX_SOURCE_BYTES: u64 = 64 * 1024;
const MAX_FIELD: usize = 1000;
/// Most plug-in folders [`Store::list`] reads.
pub const MAX_INSTALLED: usize = 4096;

/// The store's root for an app whose settings live in `settings_dir` (what craft-settings reports,
/// or the portable install's folder).
pub fn plugins_dir(settings_dir: &Path) -> PathBuf {
    settings_dir.join(PLUGINS_DIR)
}

/// The folder name of plug-in `id`: the first 32 hex digits of the SHA-256 of its lower-case id
/// (ids differing only in case are one plug-in, as on Windows and macOS folders).
pub fn folder_name(id: &str) -> String {
    Sha256::digest(id.to_ascii_lowercase().as_bytes()).iter().take(16).map(|b| format!("{b:02x}")).collect()
}

/// Where an installed plug-in came from.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum Source {
    /// A bundle file the person picked (as the app chose to describe it: its name or path).
    File { path: String },
    /// A catalog's package.
    Catalog { catalog: String, package: String },
    /// A GitHub repository's release (`owner/repo`, its tag and the asset installed).
    Github { repo: String, tag: String, asset: String },
}

impl Source {
    /// Every text is short and has no control characters.
    fn check(&self) -> Result<(), String> {
        let texts: Vec<&str> = match self {
            Source::File { path } => vec![path],
            Source::Catalog { catalog, package } => vec![catalog, package],
            Source::Github { repo, tag, asset } => vec![repo, tag, asset],
        };
        if texts.iter().any(|t| t.chars().count() > MAX_FIELD || t.chars().any(char::is_control)) {
            return Err("the plug-in's source is too long or has control characters".into());
        }
        Ok(())
    }
}

/// `source.json`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SourceRecord {
    /// The manifest's id.
    pub id: String,
    /// The manifest's version.
    pub version: String,
    /// Lower-case hex SHA-256 of the bundle (also its file name).
    pub sha256: String,
    pub source: Source,
}

/// An installed plug-in, read back and checked against its record.
#[derive(Clone, Debug, PartialEq)]
pub struct Installed {
    /// Its folder.
    pub dir: PathBuf,
    pub record: SourceRecord,
    pub manifest: Manifest,
    pub bundle: Bundle,
}

/// A plug-in folder that couldn't be read.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Problem {
    pub dir: PathBuf,
    pub error: String,
}

/// The plug-ins installed under one root.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Store {
    root: PathBuf,
}

impl Store {
    /// The store at `root` ([`plugins_dir`] of the settings folder); nothing is created until a
    /// plug-in is installed.
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Store { root: root.into() }
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    /// The folder plug-in `id` is (or would be) installed in.
    pub fn dir_of(&self, id: &str) -> PathBuf {
        self.root.join(folder_name(id))
    }

    /// Install a command plug-in's bundle (replacing an installed version of the same id): the
    /// bundle is decoded and fully checked ([`crate::bundle::open`]) before anything is written.
    pub fn install(&self, bytes: &[u8], source: Source) -> Result<Installed, String> {
        self.install_with(bytes, source, &check_command_module)
    }

    /// [`Store::install`] for an app's own kind of plug-in, whose module `check_module` checks.
    pub fn install_with(&self, bytes: &[u8], source: Source, check_module: &ModuleCheck) -> Result<Installed, String> {
        source.check()?;
        let (bundle, manifest) = open_with(bytes, check_module)?;
        let sha256 = sha256_hex(bytes);
        let dir = self.dir_of(&manifest.id);
        std::fs::create_dir_all(&dir).map_err(|e| format!("cannot create {}: {e}", dir.display()))?;
        let file = dir.join(bundle_file(&sha256));
        if std::fs::read(&file).ok().is_none_or(|old| old != bytes) {
            craft_atomic::atomic_write(&file, bytes).map_err(|e| format!("cannot write {}: {e}", file.display()))?;
        }
        let record = SourceRecord { id: manifest.id.clone(), version: manifest.version.clone(), sha256, source };
        let json = serde_json::to_vec_pretty(&record).map_err(|e| e.to_string())?;
        let record_file = dir.join(SOURCE_FILE);
        craft_atomic::atomic_write(&record_file, &json).map_err(|e| format!("cannot write {}: {e}", record_file.display()))?;
        remove_other_bundles(&dir, &record.sha256);
        Ok(Installed { dir, record, manifest, bundle })
    }

    /// The installed plug-in `id`: `Ok(None)` when it isn't installed, an error when its folder
    /// is damaged (a record that doesn't match its bundle, a bundle that doesn't hash to its name).
    /// The module isn't compiled here; [`crate::Instance::new`] checks it again before it runs.
    pub fn load(&self, id: &str) -> Result<Option<Installed>, String> {
        let dir = self.dir_of(id);
        if !dir.join(SOURCE_FILE).exists() {
            return Ok(None);
        }
        let installed = read_installed(&dir)?;
        if !installed.record.id.eq_ignore_ascii_case(id) {
            return Err(format!("{} holds another plug-in", dir.display()));
        }
        Ok(Some(installed))
    }

    /// Every installed plug-in, by folder name, and the folders that couldn't be read.
    pub fn list(&self) -> (Vec<Installed>, Vec<Problem>) {
        let (mut ok, mut bad) = (vec![], vec![]);
        let Ok(entries) = std::fs::read_dir(&self.root) else { return (ok, bad) };
        let mut dirs: Vec<PathBuf> = entries
            .filter_map(Result::ok)
            .filter(|e| e.file_name().to_str().is_some_and(|n| n.len() == 32 && n.bytes().all(|b| b.is_ascii_hexdigit())))
            .map(|e| e.path())
            .filter(|p| p.is_dir())
            .take(MAX_INSTALLED)
            .collect();
        dirs.sort();
        for dir in dirs {
            match read_installed(&dir) {
                Ok(i) => ok.push(i),
                Err(error) => bad.push(Problem { dir, error }),
            }
        }
        (ok, bad)
    }

    /// Remove plug-in `id` (its whole folder); whether it was installed.
    pub fn remove(&self, id: &str) -> Result<bool, String> {
        let dir = self.dir_of(id);
        if !dir.is_dir() {
            return Ok(false);
        }
        std::fs::remove_dir_all(&dir).map_err(|e| format!("cannot remove {}: {e}", dir.display()))?;
        Ok(true)
    }
}

/// A bundle's file name in its folder.
fn bundle_file(sha256: &str) -> String {
    format!("{sha256}.{BUNDLE_EXTENSION}")
}

/// Delete the bundles in `dir` other than `keep`'s (an update's old version), best effort.
fn remove_other_bundles(dir: &Path, keep: &str) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    let suffix = format!(".{BUNDLE_EXTENSION}");
    for e in entries.filter_map(Result::ok).take(MAX_INSTALLED) {
        let name = e.file_name();
        let Some(name) = name.to_str() else { continue };
        if name.strip_suffix(&suffix).is_some_and(|h| is_hex64(h) && h != keep) {
            let _ = std::fs::remove_file(e.path());
        }
    }
}

/// Read at most `max` bytes of `path`.
fn read_capped(path: &Path, max: u64) -> Result<Vec<u8>, String> {
    let len = std::fs::metadata(path).map_err(|e| format!("cannot read {}: {e}", path.display()))?.len();
    if len > max {
        return Err(format!("{} is larger than {} KB", path.display(), max / 1024));
    }
    std::fs::read(path).map_err(|e| format!("cannot read {}: {e}", path.display()))
}

/// Read a plug-in folder and check it: its record, the bundle the record names (its SHA-256), the
/// manifest, and that both name the plug-in the folder is named for.
fn read_installed(dir: &Path) -> Result<Installed, String> {
    let record: SourceRecord = serde_json::from_slice(&read_capped(&dir.join(SOURCE_FILE), MAX_SOURCE_BYTES)?)
        .map_err(|e| format!("{} is damaged: {e}", dir.join(SOURCE_FILE).display()))?;
    record.source.check()?;
    if !is_hex64(&record.sha256) || dir.file_name().and_then(|n| n.to_str()) != Some(folder_name(&record.id).as_str()) {
        return Err(format!("{} doesn't match its folder", dir.join(SOURCE_FILE).display()));
    }
    let bytes = read_capped(&dir.join(bundle_file(&record.sha256.to_ascii_lowercase())), MAX_BUNDLE_BYTES as u64)?;
    if !sha256_hex(&bytes).eq_ignore_ascii_case(&record.sha256) {
        return Err(format!("the bundle in {} was changed after it was installed", dir.display()));
    }
    let bundle = Bundle::decode(&bytes, MAX_BUNDLE_BYTES)?;
    let manifest = Manifest::parse(&bundle.manifest)?;
    if !manifest.id.eq_ignore_ascii_case(&record.id) {
        return Err(format!("the bundle in {} is another plug-in's", dir.display()));
    }
    Ok(Installed { dir: dir.to_path_buf(), record, manifest, bundle })
}
