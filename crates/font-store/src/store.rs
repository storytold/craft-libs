//! The font store folder: consent, installs, the manifest, removal and checks.
//!
//! Layout of the folder ([`craft_font_catalog::font_store_dir`]):
//!
//! ```text
//! settings.json        the consent setting and the sources the user added
//! manifest.json        what is installed
//! indexes/             the last indexes fetched: sources.json, one file per source
//! <family dir>/        google/fonts' folder name, or <source>/<family>: the fonts and the licence
//! language-packs/      installed Language packs, one folder per locale (`crate::language_packs`)
//! language-packs.json  which Language packs are installed, from which index entry
//! .staging/            downloads being installed; nothing a font scan picks up
//! .lock                held while the store changes
//! ```

use std::fs::File;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::time::{Duration, SystemTime};

use craft_font_catalog::{Catalog, FontEnv, font_store_dir};
use serde::{Deserialize, Serialize};

use crate::http::{Cancel, Client, FetchError};
use crate::index::{Face, Family, FontFile, GOOGLE_FONTS, Index, IndexError, MAX_INDEX_BYTES, MAX_LICENSE_BYTES, SourceProblem, Unavailable};
use crate::pin::IndexPin;
use crate::source::{is_source_url, safe_dir, safe_file_name, slug};
use crate::sources::{MAX_SOURCES_BYTES, SourceEntry, SourcesFile};
use crate::verify::{VerifyError, is_font, sha256_hex, verify_font};

mod sources;
mod transaction;

pub(crate) use transaction::Stage;
use transaction::{Commit, merge_entry};

const SETTINGS: &str = "settings.json";
const MANIFEST: &str = "manifest.json";
pub(crate) const INDEX_CACHE: &str = "indexes";
const SOURCES_CACHE: &str = "sources.json";
const STAGING: &str = ".staging";
const LOCK: &str = ".lock";
const MAX_MANIFEST_BYTES: u64 = 16 << 20;
const MAX_SETTINGS_BYTES: u64 = 1 << 20;
const MAX_USER_SOURCES: usize = 64;
/// Staging folders older than this belong to an install that crashed.
const STALE: Duration = Duration::from_secs(3600);

/// Whether the store may download.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Network {
    /// No requests. The default.
    #[default]
    Off,
    /// The user allowed downloads from jsDelivr and GitHub.
    On,
}

/// Who drives the store. Only a person in the app can turn the network on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Caller {
    /// The app's own interface, acting on the user's click.
    User,
    /// An agent (MCP, the CLI, scripting): it can install when the user has turned the network
    /// on, and can never turn it on.
    Agent,
}

/// Why a store operation failed.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// Downloading is off; the user turns it on in the app.
    #[error("downloading fonts is turned off")]
    NetworkOff,
    /// An agent tried to turn the network on.
    #[error("only the user can turn on downloading fonts")]
    NotAllowed,
    /// Not an https URL a source can be added from.
    #[error("{0} is not an https URL")]
    BadUrl(String),
    /// No exact match in the index.
    #[error(transparent)]
    Unavailable(#[from] Unavailable),
    /// The index can't be used.
    #[error(transparent)]
    Index(#[from] IndexError),
    /// Every URL failed.
    #[error("downloading {what} failed: {}", reasons.join("; "))]
    Download {
        /// The file.
        what: String,
        /// Each URL's failure.
        reasons: Vec<String>,
    },
    /// The downloaded file isn't the one the index describes.
    #[error("{file} was rejected: {error}")]
    Verify {
        /// The file.
        file: String,
        /// What was wrong.
        error: VerifyError,
    },
    /// The family isn't installed.
    #[error("{0} is not installed")]
    NotInstalled(String),
    /// The operation was cancelled.
    #[error("cancelled")]
    Cancelled,
    /// Reading or writing the store failed.
    #[error("{what}: {source}")]
    Io {
        /// What was being done.
        what: String,
        /// The error.
        source: std::io::Error,
    },
}

pub(crate) fn io(what: impl Into<String>) -> impl FnOnce(std::io::Error) -> Error {
    let what = what.into();
    move |source| Error::Io { what, source }
}

/// Download progress of an install.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Progress {
    /// The file being downloaded, from 0.
    pub file: usize,
    /// Files to download.
    pub files: usize,
    /// Bytes downloaded, of all files.
    pub done: u64,
    /// Bytes of all files.
    pub total: u64,
}

/// An installed font file.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct InstalledFile {
    /// The file's name in the family folder.
    pub name: String,
    /// Its path in its source's repository.
    pub path: String,
    /// Bytes.
    pub size: u64,
    /// sha256, lowercase hex.
    pub sha256: String,
    /// The commit of the index it was installed from.
    pub commit: String,
    /// Its PostScript names.
    #[serde(default)]
    pub postscript: Vec<String>,
}

/// An installed family.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct InstalledFamily {
    /// The family name.
    pub family: String,
    /// The ID of the source it was installed from.
    #[serde(default = "google_fonts")]
    pub source: String,
    /// The folder in the store holding its files.
    pub dir: String,
    /// `OFL`, `APACHE2` or `UFL`.
    pub license: String,
    /// The licence file's name in the folder.
    pub license_file: Option<String>,
    /// The installed files: all of the family's, or some faces.
    pub files: Vec<InstalledFile>,
}

#[derive(Default, Serialize, Deserialize)]
struct Manifest {
    version: u32,
    families: Vec<InstalledFamily>,
}

fn google_fonts() -> String {
    GOOGLE_FONTS.into()
}

/// A source the user added: an index file at an https URL, never curated.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct UserSource {
    /// The index file (format 1.x, JSON or gzipped JSON).
    pub url: String,
    /// When it was added, in seconds since 1970.
    pub added_at: u64,
    /// The sha256 of the file first fetched from it. A different file later shows as
    /// [`crate::FontSource::changed`].
    #[serde(default)]
    pub last_sha256: Option<String>,
}

#[derive(Clone, Default, Serialize, Deserialize)]
struct Settings {
    #[serde(default)]
    network: Network,
    #[serde(default)]
    user_sources: Vec<UserSource>,
}

/// Something [`FontStore::verify`] found wrong with an installed file.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Problem {
    /// The family.
    pub family: String,
    /// The file.
    pub file: String,
    /// What is wrong; `None` when the file is missing.
    pub error: Option<VerifyError>,
}

/// The suite's font store.
///
/// Nothing here touches the network on its own: only [`FontStore::fetch_indexes`] and the installs
/// do, and only while the user has turned the network on.
pub struct FontStore {
    dir: PathBuf,
    caller: Caller,
    client: Client,
    catalog: Option<Arc<Catalog>>,
    pub(crate) pack_removal_cursor: AtomicUsize,
    pub(crate) pack_removal_round_end: AtomicUsize,
}

impl std::fmt::Debug for FontStore {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FontStore").field("dir", &self.dir).field("caller", &self.caller).finish_non_exhaustive()
    }
}

impl FontStore {
    /// The store for the app's own interface, in the folder every app reads.
    /// `None` where there is no per-user folder.
    pub fn for_user(env: &FontEnv) -> Option<Self> {
        Some(Self::new(font_store_dir(env)?, Caller::User))
    }

    /// The store for an agent (MCP, CLI): it can never turn the network on.
    pub fn for_agent(env: &FontEnv) -> Option<Self> {
        Some(Self::new(font_store_dir(env)?, Caller::Agent))
    }

    /// A store in `dir`.
    pub fn new(dir: PathBuf, caller: Caller) -> Self {
        Self {
            dir,
            caller,
            client: Client::default(),
            catalog: None,
            pack_removal_cursor: AtomicUsize::new(0),
            pack_removal_round_end: AtomicUsize::new(0),
        }
    }

    /// Uses `client` for downloads (other timeouts).
    pub fn with_client(mut self, client: Client) -> Self {
        self.client = client;
        self
    }

    /// Rescans `catalog` after each install and removal, so the app sees the change at once.
    /// Apps pass `Catalog::global().clone()`.
    pub fn with_catalog(mut self, catalog: Arc<Catalog>) -> Self {
        self.catalog = Some(catalog);
        self
    }

    /// The store's folder.
    pub fn dir(&self) -> &Path {
        &self.dir
    }

    /// Who drives this store.
    pub fn caller(&self) -> Caller {
        self.caller
    }

    // ----- install

    /// Installs every file of `family`. Files already installed with the same sha256 aren't
    /// downloaded again.
    pub fn install_family(
        &self,
        index: &Index,
        family: &Family,
        cancel: &Cancel,
        progress: &mut dyn FnMut(Progress),
    ) -> Result<InstalledFamily, Error> {
        let files: Vec<&FontFile> = family.files.iter().collect();
        self.install(index, family, &files, cancel, progress)
    }

    /// Installs the file of one face (from [`Index::find_face`] or [`Index::find_postscript`]),
    /// with the family's licence.
    pub fn install_face(&self, index: &Index, face: Face<'_>, cancel: &Cancel, progress: &mut dyn FnMut(Progress)) -> Result<InstalledFamily, Error> {
        self.install(index, face.family, &[face.file], cancel, progress)
    }

    fn install(
        &self,
        index: &Index,
        family: &Family,
        files: &[&FontFile],
        cancel: &Cancel,
        progress: &mut dyn FnMut(Progress),
    ) -> Result<InstalledFamily, Error> {
        let installed = self.installed_family(&family.source, &family.family);
        let folder = self.dir.join(family.dir());
        let have = |f: &FontFile| installed.as_ref().is_some_and(|i| i.files.iter().any(|x| x.name == f.file_name() && x.sha256 == f.sha256));
        let wanted: Vec<&FontFile> = files.iter().copied().filter(|f| !have(f) || !folder.join(f.file_name()).is_file()).collect();
        let license_missing = family.license_file_name().is_some_and(|name| !folder.join(name).is_file());
        if wanted.is_empty() && !license_missing {
            return installed.ok_or_else(|| Error::NotInstalled(family.family.clone()));
        }
        self.require_network()?;
        std::fs::create_dir_all(&self.dir).map_err(io("creating the font store folder"))?;
        self.sweep_staging();
        let stage = Stage::new(&self.dir)?;

        let total: u64 = wanted.iter().map(|f| f.size).sum();
        let mut done_before = 0u64;
        let mut staged: Vec<(String, PathBuf)> = Vec::new();
        for (i, file) in wanted.iter().enumerate() {
            let mut on_bytes = |n: u64| progress(Progress { file: i, files: wanted.len(), done: done_before + n, total });
            let check = |bytes: &[u8]| verify_font(file, bytes);
            let bytes = self.fetch(&index.file_urls(family, file), file.size, file.file_name(), cancel, &mut on_bytes, &check)?;
            done_before += file.size;
            staged.push((file.file_name().to_string(), stage.write(file.file_name(), &bytes)?));
        }
        if license_missing && let Some(name) = family.license_file_name() {
            // A format 1.0 index has no size or sha256 for the licence: the size cap is the only
            // check.
            let max = family.license_file.as_ref().map_or(MAX_LICENSE_BYTES, |f| f.size);
            let check = |bytes: &[u8]| match &family.license_file {
                Some(f) if bytes.len() as u64 != f.size => Err(VerifyError::Size { expected: f.size, actual: bytes.len() as u64 }),
                Some(f) if sha256_hex(bytes) != f.sha256 => Err(VerifyError::Sha256),
                _ => Ok(()),
            };
            let bytes = self.fetch(&index.license_urls(family), max, name, cancel, &mut |_| {}, &check)?;
            staged.push((name.to_string(), stage.write(name, &bytes)?));
        }
        if cancel.is_cancelled() {
            return Err(Error::Cancelled);
        }

        let result = {
            let _lock = self.lock()?;
            let mut manifest = self.read_manifest();
            let commit = Commit::apply(&folder, &stage.dir, &staged)?;
            let entry = merge_entry(&mut manifest, index, family, &wanted);
            match self.write_manifest(&manifest) {
                Ok(()) => {
                    commit.keep();
                    Ok(entry)
                }
                Err(e) => {
                    commit.undo();
                    Err(e)
                }
            }
        };
        drop(stage);
        self.rescan();
        result
    }

    /// Downloads from each URL in turn until one gives bytes that pass `check`.
    pub(crate) fn fetch(
        &self,
        urls: &[String],
        max: u64,
        what: &str,
        cancel: &Cancel,
        on_bytes: &mut dyn FnMut(u64),
        check: &dyn Fn(&[u8]) -> Result<(), VerifyError>,
    ) -> Result<Vec<u8>, Error> {
        let mut reasons = Vec::new();
        let mut rejected = None;
        for url in urls {
            match self.client.get(url, max, cancel, on_bytes) {
                Ok(bytes) => match check(&bytes) {
                    Ok(()) => return Ok(bytes),
                    Err(e) => {
                        reasons.push(format!("{}: {e}", host(url)));
                        rejected.get_or_insert(e);
                    }
                },
                Err(FetchError::Cancelled) => return Err(Error::Cancelled),
                Err(e) => reasons.push(format!("{}: {e}", host(url))),
            }
        }
        match rejected {
            Some(error) => Err(Error::Verify { file: what.to_string(), error }),
            None => Err(Error::Download { what: what.to_string(), reasons }),
        }
    }

    // ----- installed fonts

    /// The installed families, sorted by name.
    pub fn installed(&self) -> Vec<InstalledFamily> {
        self.read_manifest().families
    }

    /// The family of this name installed from `source`, ignoring case.
    pub fn installed_family(&self, source: &str, name: &str) -> Option<InstalledFamily> {
        self.installed().into_iter().find(|f| f.is(source, name))
    }

    /// Whether `file` is installed, with the index's sha256.
    pub fn is_installed(&self, family: &Family, file: &FontFile) -> bool {
        self.installed_family(&family.source, &family.family)
            .is_some_and(|i| i.files.iter().any(|x| x.name == file.file_name() && x.sha256 == file.sha256))
    }

    /// Removes the family of this name installed from `source`: its files, its licence and its
    /// manifest entry. Its folder disappears at once and is deleted after.
    pub fn remove_family(&self, source: &str, name: &str) -> Result<(), Error> {
        let result = {
            let _lock = self.lock()?;
            let mut manifest = self.read_manifest();
            let pos = manifest.families.iter().position(|f| f.is(source, name)).ok_or_else(|| Error::NotInstalled(name.to_string()))?;
            let entry = manifest.families.remove(pos);
            let folder = self.dir.join(&entry.dir);
            let stage = Stage::new(&self.dir)?;
            let moved = stage.dir.join("removed");
            let had_folder = folder.exists();
            if had_folder {
                std::fs::rename(&folder, &moved).map_err(io(format!("removing {}", entry.family)))?;
            }
            match self.write_manifest(&manifest) {
                Ok(()) => Ok(()),
                Err(e) => {
                    if had_folder {
                        let _ = std::fs::rename(&moved, &folder);
                    }
                    Err(e)
                }
            }
        };
        self.rescan();
        result
    }

    /// Removes some files of an installed family (one style, say), by their names in its folder.
    /// Taking out the last one removes the family. Names the family's manifest entry doesn't list
    /// are left alone.
    pub fn remove_files(&self, source: &str, name: &str, files: &[String]) -> Result<(), Error> {
        let result = {
            let _lock = self.lock()?;
            let mut manifest = self.read_manifest();
            let pos = manifest.families.iter().position(|f| f.is(source, name)).ok_or_else(|| Error::NotInstalled(name.to_string()))?;
            let Some(entry) = manifest.families.get_mut(pos) else { return Err(Error::NotInstalled(name.to_string())) };
            // Only listed, safe names become paths, so a request can't reach outside the folder.
            let removing: Vec<String> = entry.files.iter().map(|f| f.name.clone()).filter(|n| safe_file_name(n) && files.contains(n)).collect();
            if removing.is_empty() {
                return Ok(());
            }
            entry.files.retain(|f| !removing.contains(&f.name));
            let folder = self.dir.join(&entry.dir);
            let stage = Stage::new(&self.dir)?;
            let mut moving: Vec<(PathBuf, PathBuf)> = removing.iter().map(|f| (folder.join(f), stage.dir.join(f))).collect();
            let last = entry.files.is_empty();
            if last {
                // The licence goes with the last style. Files the store didn't install stay, and
                // keep the folder.
                if let Some(licence) = entry.license_file.clone().filter(|n| safe_file_name(n) && !removing.contains(n)) {
                    moving.push((folder.join(&licence), stage.dir.join(&licence)));
                }
                manifest.families.remove(pos);
            }
            let mut moved = Vec::new();
            for (from, to) in &moving {
                if from.exists() {
                    std::fs::rename(from, to).map_err(io(format!("removing {}", from.display())))?;
                    moved.push((from, to));
                }
            }
            match self.write_manifest(&manifest) {
                Ok(()) => {
                    if last {
                        let _ = std::fs::remove_dir(&folder);
                    }
                    Ok(())
                }
                Err(e) => {
                    for (from, to) in moved {
                        let _ = std::fs::rename(to, from);
                    }
                    Err(e)
                }
            }
        };
        self.rescan();
        result
    }

    /// Checks every installed file against the manifest: present, the right size and sha256,
    /// and a font.
    pub fn verify(&self) -> Vec<Problem> {
        let mut problems = Vec::new();
        for fam in self.installed() {
            for f in &fam.files {
                let path = self.dir.join(&fam.dir).join(&f.name);
                let problem = |error| Problem { family: fam.family.clone(), file: f.name.clone(), error };
                let Ok(bytes) = std::fs::read(&path) else {
                    problems.push(problem(None));
                    continue;
                };
                let error = if bytes.len() as u64 != f.size {
                    Some(VerifyError::Size { expected: f.size, actual: bytes.len() as u64 })
                } else if sha256_hex(&bytes) != f.sha256 {
                    Some(VerifyError::Sha256)
                } else if !is_font(&bytes) {
                    Some(VerifyError::NotAFont)
                } else {
                    None
                };
                if error.is_some() {
                    problems.push(problem(error));
                }
            }
        }
        problems
    }
}

impl InstalledFamily {
    fn empty() -> Self {
        Self { family: String::new(), source: String::new(), dir: String::new(), license: String::new(), license_file: None, files: Vec::new() }
    }

    /// Whether this is the family `name` (ignoring case) from `source`.
    pub fn is(&self, source: &str, name: &str) -> bool {
        self.source == source && self.family.to_lowercase() == name.to_lowercase()
    }
}

pub(crate) fn read_limited(path: &Path, max: u64) -> Option<Vec<u8>> {
    if std::fs::metadata(path).ok()?.len() > max {
        return None;
    }
    std::fs::read(path).ok()
}

fn host(url: &str) -> &str {
    url.split("://").nth(1).and_then(|r| r.split('/').next()).unwrap_or(url)
}
