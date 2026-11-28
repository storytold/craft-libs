//! What Craft Font Manager needs from the suite's font store: the download consent, installing a
//! family from the index, removing one, and downloading a family's files for a preview without
//! installing them.
//!
//! [`SuiteStore`] is craft-font-store's store; [`FakeStore`] stands in for it in tests.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
#[cfg(not(target_arch = "wasm32"))]
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Condvar, Mutex};

#[cfg(not(target_arch = "wasm32"))]
use craft_font_catalog::Catalog;
pub use craft_font_store::PreviewStore;
#[cfg(not(target_arch = "wasm32"))]
use craft_font_store::{Cancel, Client, Face, FontStore, Network, PINNED_INDEX, verify_font};
pub use craft_font_store::{Conflict, LegalText, PREVIEW_SERVICE_ENV, PreviewServiceSource, UserSource};

use crate::index::{Family, Index};

mod fake;

pub use fake::{Call, FakeStore};

/// Store files to remove: a whole family, or some of its styles.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct RemoveRequest {
    /// The family name.
    pub family: String,
    /// The files the catalog found for it in the font store.
    pub files: Vec<PathBuf>,
    /// The whole family, its licence included; otherwise only `files`.
    pub whole: bool,
}

impl RemoveRequest {
    /// A request to remove `files` of `family`, or the whole family when `whole`.
    pub fn new(family: impl Into<String>, files: Vec<PathBuf>, whole: bool) -> Self {
        RemoveRequest { family: family.into(), files, whole }
    }
}

/// Bytes downloaded so far, of `total`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct Progress {
    /// Bytes received.
    pub done: u64,
    /// Bytes expected.
    pub total: u64,
}

impl Progress {
    /// `done` bytes received of `total`.
    pub const fn new(done: u64, total: u64) -> Self {
        Progress { done, total }
    }
}

/// A downloaded and verified file.
#[derive(Clone, Debug)]
#[non_exhaustive]
pub struct Downloaded {
    /// The file's path in the index.
    pub path: String,
    /// The verified file contents.
    pub bytes: Vec<u8>,
}

impl Downloaded {
    /// The verified `bytes` of the file at `path` in the index.
    pub fn new(path: impl Into<String>, bytes: Vec<u8>) -> Self {
        Downloaded { path: path.into(), bytes }
    }
}

/// An error to show the user, in English.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct StoreError(pub String);

impl StoreError {
    /// An error that reads `message`.
    pub fn new(message: impl Into<String>) -> Self {
        StoreError(message.into())
    }
}

impl std::fmt::Display for StoreError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for StoreError {}

/// What [`Installer::make_available`] did.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum MadeAvailable {
    /// The family's copies are in the OS user font folder.
    Done,
    /// Refused, changing nothing: fonts on the system already have the family's PostScript
    /// names, or sit where its copies would go.
    Refused(Vec<Conflict>),
}

fn err(e: impl std::fmt::Display) -> StoreError {
    StoreError(e.to_string())
}

/// The font store. Calls block; the app makes the downloading ones on a worker thread. What the preview pool reads
/// (the preview service, the previews setting, the HTTP client and the cache folder) is its [`PreviewStore`] part.
pub trait Installer: PreviewStore {
    /// Has the user allowed downloads? The setting is the whole suite's.
    fn downloads_allowed(&self) -> bool;

    /// Turns downloads on or off, as the user's own action.
    fn allow_downloads(&self, allowed: bool) -> Result<(), StoreError>;

    /// Downloads, verifies and installs every file of `family`, with its licence.
    fn install(&self, index: &Index, family: &Family, progress: &mut dyn FnMut(Progress)) -> Result<(), StoreError>;

    /// Installs one file of `family` (`path` as the index has it), with the family's licence.
    fn install_file(&self, index: &Index, family: &Family, path: &str, progress: &mut dyn FnMut(Progress)) -> Result<(), StoreError>;

    /// Deletes a family's files from the store.
    fn remove(&self, request: &RemoveRequest) -> Result<(), StoreError>;

    /// Downloads and verifies `family`'s files without installing them.
    fn download(&self, index: &Index, family: &Family, progress: &mut dyn FnMut(Progress)) -> Result<Vec<Downloaded>, StoreError>;

    /// The indexes without a request: the built-in ones compiled in, and the sources the user
    /// added as last fetched.
    fn indexes(&self) -> Result<Index, StoreError> {
        crate::index::bundled().map_err(err)
    }

    /// Downloads the indexes: the built-in ones at the pinned commit and the user's sources. A
    /// source that fails is in [`Index::problems`]; the others load.
    fn fetch_indexes(&self) -> Result<Index, StoreError> {
        Err(StoreError(NO_FOLDER.into()))
    }

    /// The index URLs the user added as sources. Their families are never curated.
    fn user_sources(&self) -> Vec<UserSource> {
        Vec::new()
    }

    /// Adds an index URL (https, `.json` or `.json.gz`) as a source, as the user's own action.
    /// The next [`Installer::fetch_indexes`] downloads it.
    fn add_user_source(&self, _url: &str) -> Result<(), StoreError> {
        Err(StoreError(NO_FOLDER.into()))
    }

    /// Removes a source the user added. Families installed from it stay installed.
    fn remove_user_source(&self, _url: &str) -> Result<(), StoreError> {
        Err(StoreError(NO_FOLDER.into()))
    }

    /// Where [`PreviewStore::preview_service`] comes from: the environment variable (the setting
    /// can't change it), the saved setting, the host's default
    /// ([`PreviewServiceSource::Default`]), or nowhere.
    fn preview_service_source(&self) -> PreviewServiceSource {
        PreviewServiceSource::None
    }

    /// Turns previews on or off, as the user's own action.
    fn set_previews_enabled(&self, _enabled: bool) -> Result<(), StoreError> {
        Err(StoreError(NO_FOLDER.into()))
    }

    /// Sets or clears the preview service (https, or http for a loopback host), as the user's own
    /// action.
    fn set_preview_service(&self, _url: Option<&str>) -> Result<(), StoreError> {
        Err(StoreError(NO_FOLDER.into()))
    }

    /// The text of `family`'s licence or NOTICE file: the installed copy, else downloaded when
    /// downloads are allowed. `None` when the family has no such file.
    fn legal_text(&self, _index: &Index, _family: &Family, _which: LegalText) -> Result<Option<String>, StoreError> {
        Err(StoreError(NO_FOLDER.into()))
    }

    /// Whether store families can be made available to all apps (copied into the OS user font
    /// folder). The interface offers the action only when this is true.
    fn available_to_all_supported(&self) -> bool {
        false
    }

    /// Why store families can't be made available to all apps here, in English; `None` when they
    /// can.
    fn available_to_all_unsupported(&self) -> Option<String> {
        (!self.available_to_all_supported()).then(|| NOT_HERE.to_owned())
    }

    /// Whether the user keeps store fonts in Craft apps only by default (a setting of the font
    /// store, off by default): what a plain Install does with a family that has no choice of its
    /// own ([`Installer::available_to_all_choice`]).
    fn craft_apps_only(&self) -> bool {
        false
    }

    /// Sets [`Installer::craft_apps_only`], as the user's own action. Installed families keep
    /// their choice and their copies.
    fn set_craft_apps_only(&self, _on: bool) -> Result<(), StoreError> {
        Err(StoreError(NO_FOLDER.into()))
    }

    /// The installed store families, by name, each with its folder in the store (which tells
    /// families of the same name from different sources apart, as `files` below).
    fn store_families(&self) -> Vec<(String, Vec<PathBuf>)> {
        Vec::new()
    }

    /// The user's choice for the store family `family`: available to all apps (`true`) or kept
    /// in Craft apps only (`false`). `None` when none is recorded (installed before the choice
    /// existed, or by an agent): [`Installer::craft_apps_only`] decides.
    fn available_to_all_choice(&self, _family: &str, _files: &[PathBuf]) -> Option<bool> {
        None
    }

    /// Records [`Installer::available_to_all_choice`], as the user's own action. It neither copies
    /// nor withdraws the family.
    fn set_available_to_all_choice(&self, _family: &str, _files: &[PathBuf], _on: bool) -> Result<(), StoreError> {
        Err(StoreError(NOT_HERE.into()))
    }

    /// Whether the store family `family` belongs in the OS user font folder: its own choice, else
    /// the default; never where families can't be copied there.
    fn shares(&self, family: &str, files: &[PathBuf]) -> bool {
        self.available_to_all_supported() && self.available_to_all_choice(family, files).unwrap_or(!self.craft_apps_only())
    }

    /// Whether the store family `family` (its name, and the files the catalog found for it in
    /// the font store) is available to all apps. Cheap enough to call for every row.
    fn is_available_to_all(&self, _family: &str, _files: &[PathBuf]) -> bool {
        false
    }

    /// The files of an available store family not available to all apps yet (styles an agent
    /// installed, say), by their names in the store, for the interface to offer copying them.
    fn not_yet_available(&self, _family: &str, _files: &[PathBuf]) -> Vec<String> {
        Vec::new()
    }

    /// Makes a store family available to all apps, as the user's own action; calling it again
    /// copies the styles [`Installer::not_yet_available`] lists. A font on the system with one of
    /// its PostScript names refuses it ([`MadeAvailable::Refused`]). It blocks (hashing, copying,
    /// a catalog rescan): call it on a worker thread.
    fn make_available(&self, _family: &str, _files: &[PathBuf]) -> Result<MadeAvailable, StoreError> {
        Err(StoreError(NOT_HERE.into()))
    }

    /// Takes a store family's copies out of the OS user font folder, as the user's own action.
    /// Returns the copies left in place because they changed since. It blocks: call it on a
    /// worker thread.
    fn withdraw(&self, _family: &str, _files: &[PathBuf]) -> Result<Vec<PathBuf>, StoreError> {
        Err(StoreError(NOT_HERE.into()))
    }
}

const NO_FOLDER: &str = "there is no folder for the font store";
const NOT_HERE: &str = "fonts can't be made available to all apps here";

#[cfg(not(target_arch = "wasm32"))]
/// craft-font-store's store, in the user's font store folder.
pub struct SuiteStore {
    store: FontStore,
}

#[cfg(not(target_arch = "wasm32"))]
impl SuiteStore {
    /// The store the catalog's environment names, rescanning `catalog` after each change.
    pub fn for_user(catalog: Arc<Catalog>) -> Option<SuiteStore> {
        let store = FontStore::for_user(catalog.env())?.with_catalog(catalog);
        Some(SuiteStore { store })
    }

    /// Uses `url` as the preview service while neither [`PREVIEW_SERVICE_ENV`] nor the user's
    /// setting names one; see [`FontStore::with_default_preview_service`]. Never saved.
    pub fn with_default_preview_service(self, url: &str) -> SuiteStore {
        SuiteStore { store: self.store.with_default_preview_service(url) }
    }

    /// Uses `network` for "Allow font downloads" while the user hasn't chosen; see
    /// [`FontStore::with_default_network`]. Never saved.
    pub fn with_default_network(self, network: Network) -> SuiteStore {
        SuiteStore { store: self.store.with_default_network(network) }
    }

    /// The store's HTTP client. Clones share the preview service's down state with installs;
    /// [`Previews::new`](craft_font_store::preview::Previews::new) takes it through [`PreviewStore::http_client`].
    pub fn client(&self) -> Client {
        self.store.client()
    }

    /// The store's source and name for the family `family` holding `files`, which can differ
    /// from the name in the fonts themselves.
    fn installed_name(&self, family: &str, files: &[PathBuf]) -> Option<(String, String)> {
        self.installed_entry(family, files).map(|f| (f.source, f.family))
    }

    fn installed_entry(&self, family: &str, files: &[PathBuf]) -> Option<craft_font_store::InstalledFamily> {
        let dir = self.store.dir();
        let mut installed = self.store.installed();
        let lower = family.to_lowercase();
        let pos = installed
            .iter()
            .position(|f| files.iter().any(|path| path.starts_with(dir.join(&f.dir))))
            .or_else(|| installed.iter().position(|f| f.family.to_lowercase() == lower))?;
        Some(installed.swap_remove(pos))
    }

    fn found(&self, family: &str, files: &[PathBuf]) -> Result<(String, String), StoreError> {
        self.installed_name(family, files).ok_or_else(|| err(craft_font_store::Error::NotInstalled(family.to_owned())))
    }
}

#[cfg(not(target_arch = "wasm32"))]
impl Installer for SuiteStore {
    fn downloads_allowed(&self) -> bool {
        self.store.network() == Network::On
    }

    fn allow_downloads(&self, allowed: bool) -> Result<(), StoreError> {
        if allowed { self.store.enable_network() } else { self.store.disable_network() }.map_err(err)
    }

    fn install(&self, index: &Index, family: &Family, progress: &mut dyn FnMut(Progress)) -> Result<(), StoreError> {
        let mut report = |p: craft_font_store::Progress| progress(Progress { done: p.done, total: p.total });
        self.store.install_family(index, family, &Cancel::new(), &mut report).map(drop).map_err(err)
    }

    fn install_file(&self, index: &Index, family: &Family, path: &str, progress: &mut dyn FnMut(Progress)) -> Result<(), StoreError> {
        let file = family.files.iter().find(|f| f.path == path).ok_or_else(|| StoreError(format!("{path} isn't in the index")))?;
        let mut report = |p: craft_font_store::Progress| progress(Progress { done: p.done, total: p.total });
        self.store.install_face(index, Face { family, file }, &Cancel::new(), &mut report).map(drop).map_err(err)
    }

    fn remove(&self, request: &RemoveRequest) -> Result<(), StoreError> {
        let (source, name) = self.found(&request.family, &request.files)?;
        if request.whole {
            return self.store.remove_family(&source, &name).map_err(err);
        }
        let files: Vec<String> = request.files.iter().filter_map(|f| f.file_name()).map(|f| f.to_string_lossy().into_owned()).collect();
        self.store.remove_files(&source, &name, &files).map_err(err)
    }

    fn download(&self, index: &Index, family: &Family, progress: &mut dyn FnMut(Progress)) -> Result<Vec<Downloaded>, StoreError> {
        if !self.downloads_allowed() {
            return Err(err(craft_font_store::Error::NetworkOff));
        }
        let total = family.size();
        let cancel = Cancel::new();
        let mut before = 0u64;
        let mut out = Vec::new();
        for file in &family.files {
            let mut reasons = Vec::new();
            for url in index.file_urls(family, file) {
                let mut on_bytes = |n: u64| progress(Progress { done: before.saturating_add(n), total });
                match self.store.client().get(&url, file.size, &cancel, &mut on_bytes) {
                    Ok(bytes) => match verify_font(file, &bytes) {
                        Ok(()) => {
                            out.push(Downloaded { path: file.path.clone(), bytes });
                            reasons.clear();
                            break;
                        }
                        Err(e) => reasons.push(format!("{url}: {e}")),
                    },
                    Err(e) => reasons.push(format!("{url}: {e}")),
                }
            }
            if !reasons.is_empty() {
                return Err(StoreError(format!("downloading {} failed: {}", file.file_name(), reasons.join("; "))));
            }
            before = before.saturating_add(file.size);
        }
        Ok(out)
    }

    fn indexes(&self) -> Result<Index, StoreError> {
        Ok(self.store.offline_indexes())
    }

    fn fetch_indexes(&self) -> Result<Index, StoreError> {
        self.store.fetch_indexes(&PINNED_INDEX, &Cancel::new()).map_err(err)
    }

    fn user_sources(&self) -> Vec<UserSource> {
        self.store.user_sources()
    }

    fn add_user_source(&self, url: &str) -> Result<(), StoreError> {
        self.store.add_user_source(url).map_err(err)
    }

    fn remove_user_source(&self, url: &str) -> Result<(), StoreError> {
        self.store.remove_user_source(url).map_err(err)
    }

    fn preview_service_source(&self) -> PreviewServiceSource {
        self.store.preview_service_source()
    }

    fn set_preview_service(&self, url: Option<&str>) -> Result<(), StoreError> {
        self.store.set_preview_service(url).map_err(err)
    }

    fn set_previews_enabled(&self, enabled: bool) -> Result<(), StoreError> {
        self.store.set_previews_enabled(enabled).map_err(err)
    }

    fn legal_text(&self, index: &Index, family: &Family, which: LegalText) -> Result<Option<String>, StoreError> {
        self.store.legal_text(index, family, which, &Cancel::new()).map_err(err)
    }

    fn available_to_all_supported(&self) -> bool {
        self.store.available_to_all_supported()
    }

    fn available_to_all_unsupported(&self) -> Option<String> {
        self.store.available_to_all_unsupported().map(str::to_owned)
    }

    fn craft_apps_only(&self) -> bool {
        self.store.craft_apps_only()
    }

    fn set_craft_apps_only(&self, on: bool) -> Result<(), StoreError> {
        self.store.set_craft_apps_only(on).map_err(err)
    }

    fn store_families(&self) -> Vec<(String, Vec<PathBuf>)> {
        let dir = self.store.dir();
        self.store.installed().into_iter().map(|f| (f.family, vec![dir.join(&f.dir)])).collect()
    }

    fn available_to_all_choice(&self, family: &str, files: &[PathBuf]) -> Option<bool> {
        self.installed_entry(family, files)?.available_to_all
    }

    fn set_available_to_all_choice(&self, family: &str, files: &[PathBuf], on: bool) -> Result<(), StoreError> {
        let (source, name) = self.found(family, files)?;
        self.store.set_available_to_all_choice(&source, &name, on).map_err(err)
    }

    fn is_available_to_all(&self, family: &str, files: &[PathBuf]) -> bool {
        self.store.available_family_holding(family, files).is_some()
    }

    fn not_yet_available(&self, family: &str, files: &[PathBuf]) -> Vec<String> {
        self.installed_name(family, files).map(|(source, name)| self.store.pending_files(&source, &name)).unwrap_or_default()
    }

    fn make_available(&self, family: &str, files: &[PathBuf]) -> Result<MadeAvailable, StoreError> {
        let (source, name) = self.found(family, files)?;
        match self.store.make_available(&source, &name) {
            Ok(_) => Ok(MadeAvailable::Done),
            Err(craft_font_store::Error::Conflict(conflicts)) => Ok(MadeAvailable::Refused(conflicts)),
            Err(e) => Err(err(e)),
        }
    }

    fn withdraw(&self, family: &str, files: &[PathBuf]) -> Result<Vec<PathBuf>, StoreError> {
        // A family an unsupported build removed keeps its ledger entry and copies.
        let (source, name) = self.found(family, files).or_else(|e| self.store.available_family_holding(family, files).ok_or(e))?;
        self.store.withdraw(&source, &name).map_err(err)
    }
}

#[cfg(not(target_arch = "wasm32"))]
impl PreviewStore for SuiteStore {
    fn preview_service(&self) -> Option<String> {
        self.store.preview_service()
    }

    fn http_client(&self) -> Option<Client> {
        Some(self.store.client())
    }

    fn previews_enabled(&self) -> bool {
        self.store.previews_enabled()
    }

    fn preview_cache(&self) -> Option<PathBuf> {
        Some(self.store.previews_dir())
    }
}

/// The store when the system names no folder for it (no home folder): nothing installs.
pub struct NoStore;

impl PreviewStore for NoStore {}

impl Installer for NoStore {
    fn downloads_allowed(&self) -> bool {
        false
    }

    fn allow_downloads(&self, _: bool) -> Result<(), StoreError> {
        Err(StoreError(NO_FOLDER.into()))
    }

    fn install(&self, _: &Index, _: &Family, _: &mut dyn FnMut(Progress)) -> Result<(), StoreError> {
        Err(StoreError(NO_FOLDER.into()))
    }

    fn install_file(&self, _: &Index, _: &Family, _: &str, _: &mut dyn FnMut(Progress)) -> Result<(), StoreError> {
        Err(StoreError(NO_FOLDER.into()))
    }

    fn remove(&self, _: &RemoveRequest) -> Result<(), StoreError> {
        Err(StoreError(NO_FOLDER.into()))
    }

    fn download(&self, _: &Index, _: &Family, _: &mut dyn FnMut(Progress)) -> Result<Vec<Downloaded>, StoreError> {
        Err(StoreError(NO_FOLDER.into()))
    }
}
