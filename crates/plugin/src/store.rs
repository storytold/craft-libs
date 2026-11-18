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
//! storage, with the same [`Source`] and [`SourceRecord`] ([`crate::source`]).
//!
//! An app installs with [`Store::install_for`] (or [`Store::install_with_for`] for its own kinds)
//! and lists with [`Store::list_for`]: both check [`Manifest::target`] against the app, so another
//! app's plug-in is never written and one the app no longer runs is listed apart.

use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};

use crate::bundle::{BUNDLE_EXTENSION, Bundle, MAX_BUNDLE_BYTES, ModuleCheck, check_command_module, open_with, sha256_hex};
use crate::github::is_hex64;
use crate::manifest::Manifest;
/// [`Source`] and [`SourceRecord`] live in [`crate::source`] (every target); named here too.
pub use crate::source::{Source, SourceRecord};
use crate::target::{AppInfo, Target};

/// The store's folder name inside the app's settings folder.
pub const PLUGINS_DIR: &str = "Plug-ins";
/// The record of an installed plug-in.
pub const SOURCE_FILE: &str = "source.json";
const MAX_SOURCE_BYTES: u64 = 64 * 1024;
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

/// The installed plug-ins as [`Store::list_for`] sorts them for one app, each list by folder name.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Listing {
    /// The plug-ins the app runs, and how each targets it.
    pub runnable: Vec<(Installed, Target)>,
    /// The plug-ins it doesn't run, and why, in a sentence (show them so they can be removed).
    pub not_runnable: Vec<(Installed, String)>,
    /// The folders that couldn't be read.
    pub problems: Vec<Problem>,
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

    /// Install a plug-in for `app` (replacing an installed version of the same id): the bundle is
    /// decoded and fully checked ([`crate::bundle::open`]), and [`Manifest::target`] must accept
    /// `app`, before anything is written. What an app should call for a command plug-in; its own
    /// kinds use [`Store::install_with_for`].
    pub fn install_for(&self, app: &AppInfo<'_>, bytes: &[u8], source: Source) -> Result<(Installed, Target), String> {
        self.install_with_for(app, bytes, source, &check_command_module)
    }

    /// [`Store::install_for`] for an app that runs its own kinds: `check_module` checks the module
    /// of every kind the app runs (it gets the manifest, so it can tell them apart).
    pub fn install_with_for(
        &self,
        app: &AppInfo<'_>,
        bytes: &[u8],
        source: Source,
        check_module: &ModuleCheck,
    ) -> Result<(Installed, Target), String> {
        let mut target = None;
        let installed = self.install_checked(
            bytes,
            source,
            &mut |manifest: &Manifest| {
                target = Some(manifest.target(app).map_err(|why| format!("{} can't be installed: {why}", manifest.name))?);
                Ok(())
            },
            check_module,
        )?;
        let target = target.ok_or_else(|| "the plug-in's target wasn't checked".to_string())?;
        Ok((installed, target))
    }

    /// Install a command plug-in's bundle without asking whether it was written for the app:
    /// prefer [`Store::install_for`], which refuses another app's plug-in before writing it.
    pub fn install(&self, bytes: &[u8], source: Source) -> Result<Installed, String> {
        self.install_with(bytes, source, &check_command_module)
    }

    /// [`Store::install`] for an app's own kind of plug-in, whose module `check_module` checks;
    /// prefer [`Store::install_with_for`].
    pub fn install_with(&self, bytes: &[u8], source: Source, check_module: &ModuleCheck) -> Result<Installed, String> {
        self.install_checked(bytes, source, &mut |_: &Manifest| Ok(()), check_module)
    }

    /// Check the source and the bundle, run `accept` on its manifest, then write.
    fn install_checked(
        &self,
        bytes: &[u8],
        source: Source,
        accept: &mut dyn FnMut(&Manifest) -> Result<(), String>,
        check_module: &ModuleCheck,
    ) -> Result<Installed, String> {
        source.check()?;
        let (bundle, manifest) = open_with(bytes, check_module)?;
        accept(&manifest)?;
        let sha256 = sha256_hex(bytes);
        let dir = self.dir_of(&manifest.id);
        std::fs::create_dir_all(&dir).map_err(|e| format!("cannot create {}: {e}", dir.display()))?;
        let written = write_installation(&dir, bytes, &manifest, sha256, source);
        if written.is_err() && !dir.join(SOURCE_FILE).exists() {
            // A first install that failed leaves no folder behind (an update leaves the old one).
            let _ = std::fs::remove_file(dir.join(bundle_file(&sha256_hex(bytes))));
            let _ = std::fs::remove_dir(&dir);
        }
        let record = written?;
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

    /// The installed plug-in `id` when `app` can run it: `Ok(None)` when it isn't installed, an
    /// error when its folder is damaged or [`Manifest::target`] refuses it (installed by another
    /// version of the app, say).
    pub fn load_for(&self, app: &AppInfo<'_>, id: &str) -> Result<Option<(Installed, Target)>, String> {
        let Some(installed) = self.load(id)? else { return Ok(None) };
        let target = installed.manifest.target(app).map_err(|why| format!("{} can't run here: {why}", installed.manifest.name))?;
        Ok(Some((installed, target)))
    }

    /// The installed plug-ins sorted for `app`: those it runs, those it doesn't (another app's,
    /// a newer API, a newer `minApp`: the store is the app's own, but a plug-in installed by a
    /// newer version of it stays when it is downgraded) and the folders that couldn't be read.
    /// What an app should list; [`Store::list`] doesn't look at targets.
    pub fn list_for(&self, app: &AppInfo<'_>) -> Listing {
        let (all, problems) = self.list();
        let mut listing = Listing { runnable: vec![], not_runnable: vec![], problems };
        for installed in all {
            match installed.manifest.target(app) {
                Ok(target) => listing.runnable.push((installed, target)),
                Err(why) => listing.not_runnable.push((installed, why)),
            }
        }
        listing
    }

    /// Every installed plug-in, by folder name, and the folders that couldn't be read, whatever
    /// app they were written for: prefer [`Store::list_for`].
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

/// Write the bundle, then the record that names it, then drop the bundles it replaced.
fn write_installation(dir: &Path, bytes: &[u8], manifest: &Manifest, sha256: String, source: Source) -> Result<SourceRecord, String> {
    let file = dir.join(bundle_file(&sha256));
    if std::fs::read(&file).ok().is_none_or(|old| old != bytes) {
        craft_atomic::atomic_write(&file, bytes).map_err(|e| format!("cannot write {}: {e}", file.display()))?;
    }
    let record = SourceRecord { id: manifest.id.clone(), version: manifest.version.clone(), sha256, source };
    let json = serde_json::to_vec_pretty(&record).map_err(|e| e.to_string())?;
    let record_file = dir.join(SOURCE_FILE);
    craft_atomic::atomic_write(&record_file, &json).map_err(|e| format!("cannot write {}: {e}", record_file.display()))?;
    remove_other_bundles(dir, &record.sha256);
    Ok(record)
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
