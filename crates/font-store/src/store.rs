//! The font store folder: consent, installs, the manifest, removal and checks.
//!
//! Layout of the folder ([`craft_font_catalog::font_store_dir`]):
//!
//! ```text
//! settings.json        the consent setting and the sources the user added
//! manifest.json        what is installed
//! indexes/             the last indexes fetched: sources.json, one file per source
//! previews/            preview subsets from the preview service (WOFF2), a cache of at most 50 MB
//! <family dir>/        google/fonts' folder name, or <source>/<family>: the fonts and the licence
//! .staging/            downloads being installed; nothing a font scan picks up
//! .lock                held while the store changes
//! ```

use std::fs::File;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, SystemTime};

use craft_font_catalog::{Catalog, FontEnv, font_store_dir};
use serde::{Deserialize, Serialize};

use crate::http::{Cancel, Client, FetchError};
use crate::index::{Face, Family, FontFile, GOOGLE_FONTS, Index, IndexError, MAX_INDEX_BYTES, MAX_LICENSE_BYTES, SourceProblem, Unavailable};
use crate::pin::IndexPin;
use crate::source::{is_source_url, safe_dir, safe_file_name, slug};
use crate::sources::{MAX_SOURCES_BYTES, SourceEntry, SourcesFile};
use crate::verify::{VerifyError, is_font, sha256_hex, verify_font};

const SETTINGS: &str = "settings.json";
const MANIFEST: &str = "manifest.json";
const INDEX_CACHE: &str = "indexes";
const PREVIEWS: &str = "previews";
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
    /// Not a usable preview service address.
    #[error("{0} is not an https URL, or an http URL to 127.0.0.1 or [::1]")]
    BadServiceUrl(String),
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

fn io(what: impl Into<String>) -> impl FnOnce(std::io::Error) -> Error {
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

#[derive(Clone, Serialize, Deserialize)]
struct Settings {
    #[serde(default)]
    network: Network,
    #[serde(default)]
    user_sources: Vec<UserSource>,
    #[serde(default)]
    preview_service: Option<String>,
    #[serde(default = "on")]
    previews: bool,
}

fn on() -> bool {
    true
}

impl Default for Settings {
    fn default() -> Self {
        Self { network: Network::default(), user_sources: Vec::new(), preview_service: None, previews: true }
    }
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
/// Nothing here touches the network on its own: only [`FontStore::fetch_index`] and the installs
/// do, and only while the user has turned the network on.
pub struct FontStore {
    dir: PathBuf,
    caller: Caller,
    client: Client,
    catalog: Option<Arc<Catalog>>,
    /// Tests: stands in for the environment variable.
    #[cfg(test)]
    preview_env: Option<Option<String>>,
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
            #[cfg(test)]
            preview_env: None,
        }
    }

    /// Tests: reads `env` instead of [`PREVIEW_SERVICE_ENV`].
    #[cfg(test)]
    pub(crate) fn with_preview_env(mut self, env: Option<&str>) -> Self {
        self.preview_env = Some(env.map(str::to_string));
        self
    }

    /// The download client. Clones share the preview service's down state, so a pool built on one
    /// stops asking a service the store found unreachable, and the reverse.
    pub fn client(&self) -> Client {
        self.client.clone()
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

    /// The folder for the preview service's subsets, which craft-font-manager caches. The store
    /// itself never reads or writes it.
    pub fn previews_dir(&self) -> PathBuf {
        self.dir.join(PREVIEWS)
    }

    /// Who drives this store.
    pub fn caller(&self) -> Caller {
        self.caller
    }

    // ----- consent

    /// The user's setting; [`Network::Off`] when it was never set or can't be read.
    pub fn network(&self) -> Network {
        self.settings().network
    }

    fn settings(&self) -> Settings {
        read_limited(&self.dir.join(SETTINGS), MAX_SETTINGS_BYTES).and_then(|b| serde_json::from_slice::<Settings>(&b).ok()).unwrap_or_default()
    }

    /// Turns downloading on: call it only from the user's own action (a setting, an Install
    /// click). [`Error::NotAllowed`] for an agent.
    pub fn enable_network(&self) -> Result<(), Error> {
        if self.caller != Caller::User {
            return Err(Error::NotAllowed);
        }
        self.change_settings(|s| s.network = Network::On)
    }

    /// The preview service's base URL, or `None` while there is none: [`PREVIEW_SERVICE_ENV`]
    /// environment variable when set and not empty, else the `preview_service` setting. Installs
    /// try its mirror (`<service>/v1/files/<sha256>`, `<service>/v1/licences/<sha256>`) before
    /// the index's URLs. A value that is not a usable URL counts as unset.
    pub fn preview_service(&self) -> Option<String> {
        self.preview_service_and_source().0
    }

    /// Where [`FontStore::preview_service`] gets its value: the environment, the saved setting,
    /// or nowhere. An unusable value counts as unset, as there.
    pub fn preview_service_source(&self) -> PreviewServiceSource {
        self.preview_service_and_source().1
    }

    fn preview_service_and_source(&self) -> (Option<String>, PreviewServiceSource) {
        #[cfg(test)]
        let env = self.preview_env.clone().unwrap_or_else(|| std::env::var(PREVIEW_SERVICE_ENV).ok());
        #[cfg(not(test))]
        let env = std::env::var(PREVIEW_SERVICE_ENV).ok();
        resolve_preview_source(env.as_deref(), self.settings().preview_service.as_deref())
    }

    /// Sets (or with `None` clears) the preview service in `settings.json`, from the user's own
    /// action. Only https URLs are accepted, and http for a loopback host. The environment
    /// variable still takes precedence. [`Error::NotAllowed`] for an agent, [`Error::BadServiceUrl`]
    /// for an unusable URL.
    pub fn set_preview_service(&self, url: Option<&str>) -> Result<(), Error> {
        if self.caller != Caller::User {
            return Err(Error::NotAllowed);
        }
        let url = match url {
            Some(u) => Some(check_preview_service(u)?),
            None => None,
        };
        self.change_settings(|s| s.preview_service = url)
    }

    /// Turns downloading off. Anyone may.
    pub fn disable_network(&self) -> Result<(), Error> {
        self.change_settings(|s| s.network = Network::Off)
    }

    /// Whether the app may fetch font previews from the preview service. On unless the user
    /// turned it off. It matters only while a service is set; it doesn't affect installs.
    pub fn previews_enabled(&self) -> bool {
        self.settings().previews
    }

    /// Turns previews from the preview service on or off, from the user's own action.
    /// [`Error::NotAllowed`] for an agent.
    pub fn set_previews_enabled(&self, enabled: bool) -> Result<(), Error> {
        if self.caller != Caller::User {
            return Err(Error::NotAllowed);
        }
        self.change_settings(|s| s.previews = enabled)
    }

    fn change_settings(&self, change: impl FnOnce(&mut Settings)) -> Result<(), Error> {
        let _lock = self.lock()?;
        let mut settings = self.settings();
        change(&mut settings);
        let bytes = serde_json::to_vec_pretty(&settings).map_err(|e| io("saving the setting")(std::io::Error::other(e)))?;
        self.write_file(SETTINGS, &bytes)
    }

    // ----- sources the user added

    /// The sources the user added, in the order added.
    pub fn user_sources(&self) -> Vec<UserSource> {
        self.settings().user_sources
    }

    /// Adds an index file at `url` as a source, from the user's own action; the next
    /// [`FontStore::fetch_indexes`] downloads it. Its families are never curated.
    /// [`Error::NotAllowed`] for an agent.
    pub fn add_user_source(&self, url: &str) -> Result<(), Error> {
        if self.caller != Caller::User {
            return Err(Error::NotAllowed);
        }
        let url = url.trim();
        if !is_source_url(url) {
            return Err(Error::BadUrl(url.chars().take(200).collect()));
        }
        let added_at = SystemTime::now().duration_since(SystemTime::UNIX_EPOCH).map_or(0, |d| d.as_secs());
        self.change_settings(|s| {
            if !s.user_sources.iter().any(|u| u.url == url) && s.user_sources.len() < MAX_USER_SOURCES {
                s.user_sources.push(UserSource { url: url.to_string(), added_at, last_sha256: None });
            }
        })
    }

    /// Removes a source the user added, and its cached index. Families installed from it stay
    /// installed.
    pub fn remove_user_source(&self, url: &str) -> Result<(), Error> {
        self.change_settings(|s| s.user_sources.retain(|u| u.url != url))?;
        let _ = std::fs::remove_file(self.user_cache(url));
        Ok(())
    }

    fn user_cache(&self, url: &str) -> PathBuf {
        let key: String = sha256_hex(url.as_bytes()).chars().take(16).collect();
        self.dir.join(INDEX_CACHE).join(format!("user-{key}"))
    }

    fn require_network(&self) -> Result<(), Error> {
        if self.network() == Network::On { Ok(()) } else { Err(Error::NetworkOff) }
    }

    // ----- index

    /// The indexes saved by the last [`FontStore::fetch_indexes`], when its `sources.json` is
    /// `pin`'s and every built-in index matches it, with the sources the user added.
    pub fn cached_indexes(&self, pin: &IndexPin) -> Option<Index> {
        let cache = self.dir.join(INDEX_CACHE);
        let bytes = read_limited(&cache.join(SOURCES_CACHE), MAX_SOURCES_BYTES as u64)?;
        if sha256_hex(&bytes) != pin.sha256 {
            return None;
        }
        let sources = SourcesFile::from_json(&bytes).ok()?;
        let mut parts = Vec::new();
        for entry in &sources.indexes {
            let bytes = read_limited(&cache.join(builtin_cache_name(entry)), MAX_INDEX_BYTES as u64)?;
            parts.push(entry.read(&bytes, entry.gz().is_some()).ok()?);
        }
        let (users, problems) = self.cached_user_indexes();
        Some(Index::merge(parts.into_iter().chain(users)).with_problems(problems))
    }

    /// The indexes without a request: the built-in ones compiled in, with the sources the user
    /// added as last fetched.
    #[cfg(feature = "bundled-index")]
    pub fn offline_indexes(&self) -> Index {
        let (users, mut problems) = self.cached_user_indexes();
        let builtin = Index::bundled().unwrap_or_else(|e| {
            problems.insert(0, SourceProblem { source: "sources.json".into(), name: "Built-in sources".into(), error: e.to_string() });
            Index::default()
        });
        Index::merge(std::iter::once(builtin).chain(users)).with_problems(problems)
    }

    fn cached_user_indexes(&self) -> (Vec<Index>, Vec<SourceProblem>) {
        let mut indexes = Vec::new();
        let mut problems = Vec::new();
        for user in self.user_sources() {
            let problem = |error: String| SourceProblem { source: user.url.clone(), name: user.url.clone(), error };
            let Some(bytes) = read_limited(&self.user_cache(&user.url), MAX_INDEX_BYTES as u64) else {
                problems.push(problem("not downloaded yet".into()));
                continue;
            };
            match user_index(&bytes, &user) {
                Ok(index) => indexes.push(index),
                Err(e) => problems.push(problem(e.to_string())),
            }
        }
        (indexes, problems)
    }

    /// Downloads `sources.json` as `pin` names it, every index it lists and every source the user
    /// added, checks them and saves them for [`FontStore::cached_indexes`]. Needs the network on.
    ///
    /// An index that fails leaves the others usable: it is reported in [`Index::problems`]. Only
    /// a `sources.json` that can't be had fails the whole call.
    pub fn fetch_indexes(&self, pin: &IndexPin, cancel: &Cancel) -> Result<Index, Error> {
        self.require_network()?;
        let check = |bytes: &[u8]| if sha256_hex(bytes) == pin.sha256 { Ok(()) } else { Err(VerifyError::Sha256) };
        let bytes = self.fetch(&pin.urls(), 0, MAX_SOURCES_BYTES as u64, "sources.json", cancel, &mut |_| {}, &check)?;
        let sources = SourcesFile::from_json(&bytes)?;
        let cache = self.dir.join(INDEX_CACHE);
        std::fs::create_dir_all(&cache).map_err(io("creating the index folder"))?;
        let mut parts = Vec::new();
        let mut problems = Vec::new();
        for entry in &sources.indexes {
            match self.fetch_builtin(pin, entry, cancel) {
                Ok(index) => parts.push(index),
                Err(Error::Cancelled) => return Err(Error::Cancelled),
                Err(e) => problems.push(SourceProblem { source: entry.source.clone(), name: entry.name.clone(), error: e.to_string() }),
            }
        }
        self.write_file(&format!("{INDEX_CACHE}/{SOURCES_CACHE}"), &bytes)?;
        for user in self.user_sources() {
            match self.fetch_user(&user, cancel) {
                Ok(index) => parts.push(index),
                Err(Error::Cancelled) => return Err(Error::Cancelled),
                Err(e) => problems.push(SourceProblem { source: user.url.clone(), name: user.url.clone(), error: e.to_string() }),
            }
        }
        Ok(Index::merge(parts).with_problems(problems))
    }

    fn fetch_builtin(&self, pin: &IndexPin, entry: &SourceEntry, cancel: &Cancel) -> Result<Index, Error> {
        let (path, size, sha256) = entry.gz().unwrap_or((&entry.path, entry.size, &entry.sha256));
        let check = |bytes: &[u8]| {
            if bytes.len() as u64 != size {
                Err(VerifyError::Size { expected: size, actual: bytes.len() as u64 })
            } else if sha256_hex(bytes) != sha256 {
                Err(VerifyError::Sha256)
            } else {
                Ok(())
            }
        };
        let bytes = self.fetch(&pin.file_urls(path), 0, size, &entry.name, cancel, &mut |_| {}, &check)?;
        let index = entry.read(&bytes, entry.gz().is_some())?;
        self.write_file(&format!("{INDEX_CACHE}/{}", builtin_cache_name(entry)), &bytes)?;
        Ok(index)
    }

    fn fetch_user(&self, user: &UserSource, cancel: &Cancel) -> Result<Index, Error> {
        let bytes = self.fetch(std::slice::from_ref(&user.url), 0, MAX_INDEX_BYTES as u64, &user.url, cancel, &mut |_| {}, &|_| Ok(()))?;
        let index = user_index(&bytes, user)?;
        if let Some(name) = self.user_cache(&user.url).file_name().and_then(|n| n.to_str()) {
            self.write_file(&format!("{INDEX_CACHE}/{name}"), &bytes)?;
        }
        if user.last_sha256.is_none() {
            let sha = sha256_hex(&bytes);
            self.change_settings(|s| {
                if let Some(u) = s.user_sources.iter_mut().find(|u| u.url == user.url && u.last_sha256.is_none()) {
                    u.last_sha256 = Some(sha);
                }
            })?;
        }
        Ok(index)
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
            let mut urls = self.mirror_urls("files", &file.sha256);
            let mirrors = urls.len();
            urls.extend(index.file_urls(family, file));
            let bytes = self.fetch(&urls, mirrors, file.size, file.file_name(), cancel, &mut on_bytes, &check)?;
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
            // Without a sha256 there is no mirror address.
            let mut urls = family.license_file.as_ref().map(|f| self.mirror_urls("licences", &f.sha256)).unwrap_or_default();
            let mirrors = urls.len();
            urls.extend(index.license_urls(family));
            let bytes = self.fetch(&urls, mirrors, max, name, cancel, &mut |_| {}, &check)?;
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

    /// The preview service's address for a file by its sha256; empty without a service.
    fn mirror_urls(&self, kind: &str, sha256: &str) -> Vec<String> {
        self.preview_service().map(|base| format!("{base}/v1/{kind}/{sha256}")).into_iter().collect()
    }

    /// Downloads from each URL in turn until one gives bytes that pass `check`. The first
    /// `mirrors` URLs come from the preview-service setting and may use loopback http; the rest
    /// are https only.
    #[allow(clippy::too_many_arguments)]
    fn fetch(
        &self,
        urls: &[String],
        mirrors: usize,
        max: u64,
        what: &str,
        cancel: &Cancel,
        on_bytes: &mut dyn FnMut(u64),
        check: &dyn Fn(&[u8]) -> Result<(), VerifyError>,
    ) -> Result<Vec<u8>, Error> {
        let mut reasons = Vec::new();
        // A rejection by an index URL is a corrupt file; a mirror's alone is one more failed source.
        let mut rejected = None;
        for (i, url) in urls.iter().enumerate() {
            let got =
                if i < mirrors { self.client.get_from_service(url, max, cancel, on_bytes) } else { self.client.get(url, max, cancel, on_bytes) };
            match got {
                Ok(bytes) => match check(&bytes) {
                    Ok(()) => return Ok(bytes),
                    Err(e) => {
                        reasons.push(format!("{}: {e}", host(url)));
                        if i >= mirrors {
                            rejected.get_or_insert(e);
                        }
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

    // ----- folder

    fn rescan(&self) {
        if let Some(catalog) = &self.catalog {
            catalog.rescan();
        }
    }

    fn lock(&self) -> Result<File, Error> {
        std::fs::create_dir_all(&self.dir).map_err(io("creating the font store folder"))?;
        let file = File::options().create(true).truncate(false).write(true).open(self.dir.join(LOCK)).map_err(io("locking the font store"))?;
        file.lock().map_err(io("locking the font store"))?;
        Ok(file)
    }

    /// The manifest, without entries that aren't safe to act on. A manifest that can't be read
    /// is kept aside as `manifest.json.bad` and counts as empty.
    fn read_manifest(&self) -> Manifest {
        let path = self.dir.join(MANIFEST);
        let Some(bytes) = read_limited(&path, MAX_MANIFEST_BYTES) else { return Manifest::default() };
        let Ok(mut manifest) = serde_json::from_slice::<Manifest>(&bytes) else {
            let _ = std::fs::rename(&path, self.dir.join("manifest.json.bad"));
            return Manifest::default();
        };
        manifest
            .families
            .retain(|f| safe_dir(&f.dir) && f.files.iter().all(|x| safe_file_name(&x.name)) && f.license_file.as_deref().is_none_or(safe_file_name));
        manifest
    }

    fn write_manifest(&self, manifest: &Manifest) -> Result<(), Error> {
        let manifest = Manifest { version: 1, families: manifest.families.clone() };
        let bytes = serde_json::to_vec_pretty(&manifest).map_err(|e| io("saving the manifest")(std::io::Error::other(e)))?;
        self.write_file(MANIFEST, &bytes)
    }

    fn write_file(&self, name: &str, bytes: &[u8]) -> Result<(), Error> {
        std::fs::create_dir_all(&self.dir).map_err(io("creating the font store folder"))?;
        craft_atomic::atomic_write(&self.dir.join(name), bytes).map_err(io(format!("saving {name}")))
    }

    fn sweep_staging(&self) {
        for e in std::fs::read_dir(self.dir.join(STAGING)).into_iter().flatten().flatten() {
            let old = e.metadata().and_then(|m| m.modified()).is_ok_and(|t| SystemTime::now().duration_since(t).is_ok_and(|d| d > STALE));
            if old {
                let _ = std::fs::remove_dir_all(e.path());
            }
        }
    }
}

/// The family's manifest entry after installing `files`, merged into `manifest`.
fn merge_entry(manifest: &mut Manifest, index: &Index, family: &Family, files: &[&FontFile]) -> InstalledFamily {
    let pos = match manifest.families.iter().position(|f| f.is(&family.source, &family.family)) {
        Some(p) => p,
        None => {
            manifest.families.push(InstalledFamily {
                family: family.family.clone(),
                source: family.source.clone(),
                dir: family.dir().to_string(),
                license: family.license.clone(),
                license_file: None,
                files: Vec::new(),
            });
            manifest.families.len() - 1
        }
    };
    let Some(entry) = manifest.families.get_mut(pos) else { return InstalledFamily::empty() };
    entry.license.clone_from(&family.license);
    let commit = index.source_of(family).map(|s| s.commit.clone()).unwrap_or_default();
    if let Some(name) = family.license_file_name() {
        entry.license_file = Some(name.to_string());
    }
    for f in files {
        entry.files.retain(|x| x.name != f.file_name());
        entry.files.push(InstalledFile {
            name: f.file_name().to_string(),
            path: f.path.clone(),
            size: f.size,
            sha256: f.sha256.clone(),
            commit: commit.clone(),
            postscript: f.postscript.clone(),
        });
    }
    entry.files.sort_by(|a, b| a.name.cmp(&b.name));
    let entry = entry.clone();
    manifest.families.sort_by_key(|f| (f.family.to_lowercase(), f.source.clone()));
    entry
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

fn builtin_cache_name(entry: &SourceEntry) -> String {
    format!("{}{}", slug(&entry.source), if entry.gz().is_some() { ".json.gz" } else { ".json" })
}

/// A user-added source's index from the bytes fetched from it.
fn user_index(bytes: &[u8], user: &UserSource) -> Result<Index, IndexError> {
    let mut index = Index::from_user_bytes(bytes, &user.url)?;
    if user.last_sha256.as_ref().is_some_and(|h| *h != sha256_hex(bytes)) {
        index.mark_changed();
    }
    Ok(index)
}

fn read_limited(path: &Path, max: u64) -> Option<Vec<u8>> {
    if std::fs::metadata(path).ok()?.len() > max {
        return None;
    }
    std::fs::read(path).ok()
}

/// The environment variable that sets the preview service's base URL; it beats the saved setting.
pub const PREVIEW_SERVICE_ENV: &str = "CRAFT_FONT_PREVIEW_URL";

/// Where the preview service in effect comes from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PreviewServiceSource {
    /// [`PREVIEW_SERVICE_ENV`]; the setting can't change it.
    Env,
    /// The `preview_service` setting in `settings.json`.
    Saved,
    /// There is no service.
    None,
}

/// The environment value when set and not empty, else the setting; an unusable value counts as
/// unset.
#[cfg(test)]
pub(crate) fn resolve_preview_service(env: Option<&str>, setting: Option<&str>) -> Option<String> {
    resolve_preview_source(env, setting).0
}

/// [`resolve_preview_service`] and where the value came from.
pub(crate) fn resolve_preview_source(env: Option<&str>, setting: Option<&str>) -> (Option<String>, PreviewServiceSource) {
    let (url, source) = match env.map(str::trim).filter(|e| !e.is_empty()) {
        Some(e) => (normalize_service_url(e), PreviewServiceSource::Env),
        None => (setting.and_then(normalize_service_url), PreviewServiceSource::Saved),
    };
    if url.is_some() { (url, source) } else { (None, PreviewServiceSource::None) }
}

/// The preview-service address `url` stands for: without a trailing slash, when it is an https URL,
/// or an http one for a loopback host, with no credentials, query or fragment. [`Error::BadServiceUrl`]
/// otherwise.
pub fn check_preview_service(url: &str) -> Result<String, Error> {
    normalize_service_url(url).ok_or_else(|| Error::BadServiceUrl(url.trim().chars().take(200).collect()))
}

/// `url` without a trailing slash, when it is an https URL, or an http one for a loopback host,
/// with no credentials, query or fragment.
fn normalize_service_url(url: &str) -> Option<String> {
    let url = url.trim().trim_end_matches('/');
    if url.len() > 2000 || url.chars().any(|c| c.is_control() || c.is_whitespace() || c == '\\') {
        return None;
    }
    let (https, rest) = match (url.strip_prefix("https://"), url.strip_prefix("http://")) {
        (Some(r), _) => (true, r),
        (None, Some(r)) => (false, r),
        _ => return None,
    };
    let authority = rest.split('/').next().unwrap_or_default();
    if authority.is_empty() || authority.contains('@') || url.contains(['?', '#']) {
        return None;
    }
    let host = match authority.strip_prefix('[') {
        Some(v6) => v6.split(']').next().unwrap_or_default(),
        None => authority.split(':').next().unwrap_or_default(),
    };
    let loopback = matches!(host.to_ascii_lowercase().as_str(), "127.0.0.1" | "::1");
    (https || loopback).then(|| url.to_string())
}

fn host(url: &str) -> &str {
    url.split("://").nth(1).and_then(|r| r.split('/').next()).unwrap_or(url)
}

/// A folder of downloads under `.staging`, deleted when dropped.
struct Stage {
    dir: PathBuf,
}

impl Stage {
    fn new(store: &Path) -> Result<Self, Error> {
        static N: AtomicU64 = AtomicU64::new(0);
        let dir = store.join(STAGING).join(format!("{}-{}", std::process::id(), N.fetch_add(1, Ordering::Relaxed)));
        std::fs::create_dir_all(&dir).map_err(io("creating a download folder"))?;
        Ok(Self { dir })
    }

    /// Writes a verified download as `<name>.part`, which no font scan picks up.
    fn write(&self, name: &str, bytes: &[u8]) -> Result<PathBuf, Error> {
        let path = self.dir.join(format!("{name}.part"));
        let write = || -> std::io::Result<()> {
            let mut f = File::options().write(true).create_new(true).open(&path)?;
            f.write_all(bytes)?;
            f.sync_all()
        };
        write().map_err(io(format!("saving {name}")))?;
        Ok(path)
    }
}

impl Drop for Stage {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

/// Staged files moved into a family folder, undone unless kept.
struct Commit {
    folder: PathBuf,
    created_folder: bool,
    /// Each file moved in, and where the file it replaced went.
    moved: Vec<(PathBuf, Option<PathBuf>)>,
    kept: bool,
}

impl Commit {
    fn apply(folder: &Path, stage: &Path, staged: &[(String, PathBuf)]) -> Result<Self, Error> {
        let created_folder = !folder.is_dir();
        let mut commit = Self { folder: folder.to_path_buf(), created_folder, moved: Vec::new(), kept: false };
        std::fs::create_dir_all(folder).map_err(io("creating the family folder"))?;
        for (name, from) in staged {
            let dest = folder.join(name);
            let backup = if dest.exists() {
                let b = stage.join(format!("{name}.old"));
                std::fs::rename(&dest, &b).map_err(io(format!("replacing {name}")))?;
                Some(b)
            } else {
                None
            };
            if let Err(e) = std::fs::rename(from, &dest) {
                if let Some(b) = &backup {
                    let _ = std::fs::rename(b, &dest);
                }
                return Err(io(format!("installing {name}"))(e));
            }
            commit.moved.push((dest, backup));
        }
        Ok(commit)
    }

    fn keep(mut self) {
        self.kept = true;
    }

    fn undo(self) {}
}

impl Drop for Commit {
    fn drop(&mut self) {
        if self.kept {
            return;
        }
        for (dest, backup) in self.moved.iter().rev() {
            let _ = std::fs::remove_file(dest);
            if let Some(b) = backup {
                let _ = std::fs::rename(b, dest);
            }
        }
        if self.created_folder {
            let _ = std::fs::remove_dir(&self.folder);
        }
    }
}
