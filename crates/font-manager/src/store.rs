//! What Craft Font Manager needs from the suite's font store: the download consent, installing a
//! family from the index, removing one, and downloading a family's files for a preview without
//! installing them.
//!
//! [`SuiteStore`] is craft-font-store's store; [`FakeStore`] stands in for it in tests.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Condvar, Mutex};

use craft_font_catalog::Catalog;
use craft_font_store::{Cancel, Client, Face, FontStore, Network, PINNED_INDEX, verify_font};
pub use craft_font_store::{Conflict, LegalText, PREVIEW_SERVICE_ENV, PreviewServiceSource, UserSource};

use crate::index::{Family, Index};

/// Store files to remove: a whole family, or some of its styles.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RemoveRequest {
    pub family: String,
    /// The files the catalog found for it in the font store.
    pub files: Vec<PathBuf>,
    /// The whole family, its licence included; otherwise only `files`.
    pub whole: bool,
}

/// Bytes downloaded so far, of `total`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Progress {
    pub done: u64,
    pub total: u64,
}

/// A downloaded and verified file.
#[derive(Clone, Debug)]
pub struct Downloaded {
    pub path: String,
    pub bytes: Vec<u8>,
}

/// An error to show the user, in English.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StoreError(pub String);

impl std::fmt::Display for StoreError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for StoreError {}

/// What [`Installer::make_available`] did.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MadeAvailable {
    Done,
    /// Refused, changing nothing: fonts on the system already have these of the family's
    /// PostScript names.
    Refused(Vec<Conflict>),
}

fn err(e: impl std::fmt::Display) -> StoreError {
    StoreError(e.to_string())
}

/// The font store. Calls block; the app makes the downloading ones on a worker thread.
pub trait Installer: Send + Sync {
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

    /// The preview service's base URL, if one is set: [`craft_font_store::PREVIEW_SERVICE_ENV`], else the font
    /// store's `preview_service` setting. Installs try it as a mirror first.
    fn preview_service(&self) -> Option<String> {
        None
    }

    /// Where [`Installer::preview_service`] comes from: the environment variable (the setting
    /// can't change it), the saved setting, or nowhere.
    fn preview_service_source(&self) -> PreviewServiceSource {
        PreviewServiceSource::None
    }

    /// The store's HTTP client, if it has one. [`Previews`](crate::remote::Previews) fetches
    /// with it, so a preview service that installs found unreachable is skipped by previews too,
    /// and the reverse.
    fn http_client(&self) -> Option<Client> {
        None
    }

    /// Whether previews may be fetched from the preview service: the user's setting, on by
    /// default.
    fn previews_enabled(&self) -> bool {
        true
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

    /// The folder [`crate::remote::Previews`] caches preview subsets in; none keeps them only
    /// for as long as the caller holds them.
    fn preview_cache(&self) -> Option<PathBuf> {
        None
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

    /// The folder in the font store holding `family`'s files. The detail sheet keeps only the
    /// installed files in it, so a family of the same name from another source isn't mistaken
    /// for this one. `None` when the store has no folders.
    fn family_folder(&self, _family: &Family) -> Option<PathBuf> {
        None
    }

    /// The copies of the store family `family` in the OS user font folder that this build can't
    /// change (a Flatpak whose access to the folder was revoked): removing the family leaves
    /// them. Empty when there are none or the folder can be changed.
    fn copies_out_of_reach(&self, _family: &str, _files: &[PathBuf]) -> Vec<PathBuf> {
        Vec::new()
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

    /// The text of `family`'s licence or NOTICE file: the installed copy, else downloaded when
    /// downloads are allowed. `None` when the family has no such file.
    fn legal_text(&self, _index: &Index, _family: &Family, _which: LegalText) -> Result<Option<String>, StoreError> {
        Err(StoreError(NO_FOLDER.into()))
    }
}

const NO_FOLDER: &str = "there is no folder for the font store";
const NOT_HERE: &str = "fonts can't be copied into your fonts folder here";

/// craft-font-store's store, in the user's font store folder.
pub struct SuiteStore {
    store: FontStore,
}

impl SuiteStore {
    /// The store the catalog's environment names, rescanning `catalog` after each change.
    pub fn for_user(catalog: Arc<Catalog>) -> Option<SuiteStore> {
        let store = FontStore::for_user(catalog.env())?.with_catalog(catalog);
        Some(SuiteStore { store })
    }

    /// The store's HTTP client. Clones share the preview service's down state with installs;
    /// [`Previews::new`](crate::remote::Previews::new) takes it through [`Installer::http_client`].
    pub fn client(&self) -> Client {
        self.store.client()
    }

    /// The store's source and name for the family `family` holding `files`, which can differ
    /// from the name in the fonts themselves.
    fn installed_name(&self, family: &str, files: &[PathBuf]) -> Option<(String, String)> {
        let dir = self.store.dir();
        let installed = self.store.installed();
        let lower = family.to_lowercase();
        installed
            .iter()
            .find(|f| files.iter().any(|path| path.starts_with(dir.join(&f.dir))))
            .or_else(|| installed.iter().find(|f| f.family.to_lowercase() == lower))
            .map(|f| (f.source.clone(), f.family.clone()))
    }
}

impl SuiteStore {
    fn found(&self, family: &str, files: &[PathBuf]) -> Result<(String, String), StoreError> {
        self.installed_name(family, files).ok_or_else(|| err(craft_font_store::Error::NotInstalled(family.to_owned())))
    }
}

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

    fn preview_service(&self) -> Option<String> {
        self.store.preview_service()
    }

    fn preview_service_source(&self) -> PreviewServiceSource {
        self.store.preview_service_source()
    }

    fn http_client(&self) -> Option<Client> {
        Some(self.store.client())
    }

    fn set_preview_service(&self, url: Option<&str>) -> Result<(), StoreError> {
        self.store.set_preview_service(url).map_err(err)
    }

    fn preview_cache(&self) -> Option<PathBuf> {
        Some(self.store.previews_dir())
    }

    fn previews_enabled(&self) -> bool {
        self.store.previews_enabled()
    }

    fn set_previews_enabled(&self, enabled: bool) -> Result<(), StoreError> {
        self.store.set_previews_enabled(enabled).map_err(err)
    }

    fn available_to_all_supported(&self) -> bool {
        self.store.available_to_all_supported()
    }

    fn family_folder(&self, family: &Family) -> Option<PathBuf> {
        Some(self.store.dir().join(family.dir()))
    }

    fn is_available_to_all(&self, family: &str, files: &[PathBuf]) -> bool {
        self.store.available_family_holding(family, files).is_some()
    }

    fn not_yet_available(&self, family: &str, files: &[PathBuf]) -> Vec<String> {
        self.installed_name(family, files).map(|(source, name)| self.store.pending_files(&source, &name)).unwrap_or_default()
    }

    fn available_to_all_unsupported(&self) -> Option<String> {
        self.store.available_to_all_unsupported().map(str::to_owned)
    }

    fn copies_out_of_reach(&self, family: &str, files: &[PathBuf]) -> Vec<PathBuf> {
        self.installed_name(family, files).map(|(source, name)| self.store.copies_out_of_reach(&source, &name)).unwrap_or_default()
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
        let (source, name) = self.found(family, files)?;
        self.store.withdraw(&source, &name).map_err(err)
    }

    fn legal_text(&self, index: &Index, family: &Family, which: LegalText) -> Result<Option<String>, StoreError> {
        self.store.legal_text(index, family, which, &Cancel::new()).map_err(err)
    }
}

/// The store when the system names no folder for it (no home folder): nothing installs.
pub struct NoStore;

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

/// What a [`FakeStore`] was asked to do.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Call {
    Install(String),
    /// The family and the file's path in the index.
    InstallFile(String, String),
    Remove(String),
    Download(String),
    MakeAvailable(String),
    Withdraw(String),
}

/// A font store without a network: "downloads" come from a table of file bytes by index path,
/// and installs write them into a folder as the real store does, `<dir>/<family>/<file>`. It
/// refuses to download while downloads are off, like the real one.
pub struct FakeStore {
    dir: PathBuf,
    files: HashMap<String, Vec<u8>>,
    allowed: AtomicBool,
    calls: Mutex<Vec<Call>>,
    failure: Mutex<Option<String>>,
    /// A google/fonts checkout to read files missing from `files` from.
    folder: Option<PathBuf>,
    /// The built-in indexes; the bundled ones when not given.
    builtin: Option<Index>,
    /// Index files user sources "download", by URL.
    served_indexes: HashMap<String, Vec<u8>>,
    user: Mutex<Vec<UserSource>>,
    preview_service: Mutex<Option<String>>,
    previews: Mutex<bool>,
    /// User sources downloaded so far, and those whose file changed since.
    fetched: Mutex<Vec<String>>,
    changed: Mutex<Vec<String>>,
    /// Downloads stop halfway while this is true.
    held: (Mutex<bool>, Condvar),
    /// Families made available to all apps, by name.
    available: Mutex<Vec<String>>,
    /// What the next [`Installer::make_available`] refuses with.
    refusal: Mutex<Option<Vec<Conflict>>>,
    /// Store files of available families not copied yet, by lower-case family name.
    pending: Mutex<HashMap<String, Vec<String>>>,
    /// The copies the next [`Installer::withdraw`] leaves in place.
    kept: Mutex<Vec<PathBuf>>,
    /// Why it can't change the OS folder, when it can't.
    unsupported: Mutex<Option<String>>,
}

impl FakeStore {
    pub fn new(dir: impl Into<PathBuf>, files: HashMap<String, Vec<u8>>) -> FakeStore {
        FakeStore {
            dir: dir.into(),
            files,
            allowed: AtomicBool::new(false),
            calls: Mutex::new(Vec::new()),
            failure: Mutex::new(None),
            folder: None,
            builtin: None,
            served_indexes: HashMap::new(),
            user: Mutex::new(Vec::new()),
            preview_service: Mutex::new(None),
            previews: Mutex::new(true),
            fetched: Mutex::new(Vec::new()),
            changed: Mutex::new(Vec::new()),
            held: (Mutex::new(false), Condvar::new()),
            available: Mutex::new(Vec::new()),
            refusal: Mutex::new(None),
            pending: Mutex::new(HashMap::new()),
            kept: Mutex::new(Vec::new()),
            unsupported: Mutex::new(None),
        }
    }

    /// Uses `index` as the built-in indexes.
    pub fn with_index(mut self, index: Index) -> FakeStore {
        self.builtin = Some(index);
        self
    }

    /// Has the index at `url` "download" as `bytes` once added as a source.
    pub fn serving_index(mut self, url: &str, bytes: Vec<u8>) -> FakeStore {
        self.served_indexes.insert(url.to_owned(), bytes);
        self
    }

    /// The index at `url` now differs from the one first downloaded.
    pub fn change_index(&self, url: &str) {
        if let Ok(mut changed) = self.changed.lock() {
            changed.push(url.to_owned());
        }
    }

    /// Also "downloads" files from a google/fonts checkout, by their path in the index.
    pub fn serving(mut self, checkout: impl Into<PathBuf>) -> FakeStore {
        self.folder = Some(checkout.into());
        self
    }

    /// The next download fails with `error`, as when the network is down.
    pub fn fail_next(&self, error: &str) {
        if let Ok(mut failure) = self.failure.lock() {
            *failure = Some(error.to_owned());
        }
    }

    /// The next [`Installer::make_available`] is refused for `conflicts`.
    pub fn refuse_next(&self, conflicts: Vec<Conflict>) {
        if let Ok(mut refusal) = self.refusal.lock() {
            *refusal = Some(conflicts);
        }
    }

    /// `files` (store file names) of the available family `family` aren't copied yet, as after
    /// an agent's install, until the next [`Installer::make_available`].
    pub fn set_pending(&self, family: &str, files: Vec<String>) {
        if let Ok(mut pending) = self.pending.lock() {
            pending.insert(family.to_lowercase(), files);
        }
    }

    /// The next [`Installer::withdraw`] leaves `copies` in place, as if they changed since.
    pub fn keep_next(&self, copies: Vec<PathBuf>) {
        if let Ok(mut kept) = self.kept.lock() {
            *kept = copies;
        }
    }

    /// It can't change the OS folder from now on, for `reason`. Families already available keep
    /// their copies, out of its reach.
    pub fn lose_os_folder(&self, reason: &str) {
        if let Ok(mut unsupported) = self.unsupported.lock() {
            *unsupported = Some(reason.to_owned());
        }
    }

    /// Downloads report half their bytes, then wait until [`FakeStore::release`] (with
    /// `Runner::Thread`).
    pub fn hold(&self) {
        if let Ok(mut held) = self.held.0.lock() {
            *held = true;
        }
    }

    pub fn release(&self) {
        if let Ok(mut held) = self.held.0.lock() {
            *held = false;
        }
        self.held.1.notify_all();
    }

    fn wait_while_held(&self) {
        let Ok(mut held) = self.held.0.lock() else { return };
        while *held {
            match self.held.1.wait(held) {
                Ok(h) => held = h,
                Err(_) => return,
            }
        }
    }

    /// Everything asked of it, downloads refused for lack of consent included.
    pub fn calls(&self) -> Vec<Call> {
        self.calls.lock().map(|c| c.clone()).unwrap_or_default()
    }

    fn record(&self, call: Call) {
        if let Ok(mut calls) = self.calls.lock() {
            calls.push(call);
        }
    }

    fn fetch(&self, family: &Family, only: Option<&str>, progress: &mut dyn FnMut(Progress)) -> Result<Vec<Downloaded>, StoreError> {
        if !self.downloads_allowed() {
            return Err(StoreError("downloading fonts is turned off".into()));
        }
        if let Some(error) = self.failure.lock().ok().and_then(|mut f| f.take()) {
            return Err(StoreError(error));
        }
        let wanted: Vec<_> = family.files.iter().filter(|f| only.is_none_or(|p| p == f.path)).collect();
        let total = wanted.iter().map(|f| f.size).sum();
        progress(Progress { done: total / 2, total });
        self.wait_while_held();
        let mut done = 0u64;
        let mut out = Vec::new();
        for file in wanted {
            let from_folder = || self.folder.as_ref().and_then(|dir| std::fs::read(dir.join(&file.path)).ok());
            let bytes =
                self.files.get(&file.path).cloned().or_else(from_folder).ok_or_else(|| StoreError(format!("{} isn't available", file.path)))?;
            done = done.saturating_add(file.size);
            progress(Progress { done, total });
            out.push(Downloaded { path: file.path.clone(), bytes });
        }
        Ok(out)
    }

    fn write(&self, family: &Family, files: Vec<Downloaded>) -> Result<(), StoreError> {
        let dir = self.dir.join(&family.family);
        std::fs::create_dir_all(&dir).map_err(err)?;
        for file in files {
            let name = file.path.rsplit('/').next().unwrap_or(&file.path);
            std::fs::write(dir.join(name), &file.bytes).map_err(err)?;
        }
        Ok(())
    }

    fn in_store(&self, path: &Path) -> bool {
        path.starts_with(&self.dir)
    }
}

impl Installer for FakeStore {
    fn downloads_allowed(&self) -> bool {
        self.allowed.load(Ordering::SeqCst)
    }

    fn allow_downloads(&self, allowed: bool) -> Result<(), StoreError> {
        self.allowed.store(allowed, Ordering::SeqCst);
        Ok(())
    }

    fn install(&self, _: &Index, family: &Family, progress: &mut dyn FnMut(Progress)) -> Result<(), StoreError> {
        self.record(Call::Install(family.family.clone()));
        let files = self.fetch(family, None, progress)?;
        self.write(family, files)
    }

    fn install_file(&self, _: &Index, family: &Family, path: &str, progress: &mut dyn FnMut(Progress)) -> Result<(), StoreError> {
        self.record(Call::InstallFile(family.family.clone(), path.to_owned()));
        let files = self.fetch(family, Some(path), progress)?;
        self.write(family, files)
    }

    fn remove(&self, request: &RemoveRequest) -> Result<(), StoreError> {
        self.record(Call::Remove(request.family.clone()));
        if let Some(outside) = request.files.iter().find(|f| !self.in_store(f)) {
            return Err(StoreError(format!("{} isn't in the font store", outside.display())));
        }
        // Like the store, it leaves copies out of its reach listed.
        if request.whole && self.available_to_all_supported() {
            self.available.lock().map_err(err)?.retain(|f| !f.eq_ignore_ascii_case(&request.family));
        }
        for file in &request.files {
            std::fs::remove_file(file).map_err(err)?;
        }
        Ok(())
    }

    fn download(&self, _: &Index, family: &Family, progress: &mut dyn FnMut(Progress)) -> Result<Vec<Downloaded>, StoreError> {
        self.record(Call::Download(family.family.clone()));
        self.fetch(family, None, progress)
    }

    fn indexes(&self) -> Result<Index, StoreError> {
        let builtin = match &self.builtin {
            Some(index) => index.clone(),
            None => crate::index::bundled().map_err(err)?,
        };
        let fetched = self.fetched.lock().map(|f| f.clone()).unwrap_or_default();
        let changed = self.changed.lock().map(|c| c.clone()).unwrap_or_default();
        let mut users = Vec::new();
        let mut problems = Vec::new();
        for source in self.user_sources() {
            let problem = |error: String| craft_font_store::SourceProblem { source: source.url.clone(), name: source.url.clone(), error };
            if !fetched.contains(&source.url) {
                problems.push(problem("not downloaded yet".into()));
                continue;
            }
            match self.served_indexes.get(&source.url).map(|bytes| Index::from_user_bytes(bytes, &source.url)) {
                Some(Ok(mut index)) => {
                    if changed.contains(&source.url) {
                        index.mark_changed();
                    }
                    users.push(index);
                }
                Some(Err(e)) => problems.push(problem(e.to_string())),
                None => problems.push(problem("nothing at this address".into())),
            }
        }
        Ok(Index::merge(std::iter::once(builtin).chain(users)).with_problems(problems))
    }

    fn fetch_indexes(&self) -> Result<Index, StoreError> {
        if !self.downloads_allowed() {
            return Err(StoreError("downloading fonts is turned off".into()));
        }
        if let Ok(mut fetched) = self.fetched.lock() {
            *fetched = self.user_sources().into_iter().map(|s| s.url).collect();
        }
        self.indexes()
    }

    fn user_sources(&self) -> Vec<UserSource> {
        self.user.lock().map(|u| u.clone()).unwrap_or_default()
    }

    fn add_user_source(&self, url: &str) -> Result<(), StoreError> {
        if !url.starts_with("https://") {
            return Err(StoreError("an index address starts with https://".into()));
        }
        let mut user = self.user.lock().map_err(err)?;
        if !user.iter().any(|s| s.url == url) {
            user.push(UserSource { url: url.to_owned(), added_at: 0, last_sha256: None });
        }
        Ok(())
    }

    fn remove_user_source(&self, url: &str) -> Result<(), StoreError> {
        self.user.lock().map_err(err)?.retain(|s| s.url != url);
        Ok(())
    }

    fn preview_service(&self) -> Option<String> {
        self.preview_service.lock().ok()?.clone()
    }

    fn preview_service_source(&self) -> PreviewServiceSource {
        if self.preview_service().is_some() { PreviewServiceSource::Saved } else { PreviewServiceSource::None }
    }

    fn previews_enabled(&self) -> bool {
        self.previews.lock().map_or(true, |p| *p)
    }

    fn set_previews_enabled(&self, enabled: bool) -> Result<(), StoreError> {
        *self.previews.lock().map_err(err)? = enabled;
        Ok(())
    }

    fn set_preview_service(&self, url: Option<&str>) -> Result<(), StoreError> {
        let url = url.map(craft_font_store::check_preview_service).transpose().map_err(err)?;
        *self.preview_service.lock().map_err(err)? = url;
        Ok(())
    }

    fn preview_cache(&self) -> Option<PathBuf> {
        Some(self.dir.join("previews"))
    }

    fn available_to_all_supported(&self) -> bool {
        self.unsupported.lock().is_ok_and(|u| u.is_none())
    }

    fn available_to_all_unsupported(&self) -> Option<String> {
        self.unsupported.lock().ok().and_then(|u| u.clone())
    }

    /// The family's files, as if copied, while it can't reach the OS folder.
    fn copies_out_of_reach(&self, family: &str, files: &[PathBuf]) -> Vec<PathBuf> {
        if self.available_to_all_supported() || !self.is_available_to_all(family, files) {
            return Vec::new();
        }
        files.iter().filter_map(|f| f.file_name()).map(|n| self.dir.join("os-fonts").join(n)).collect()
    }

    fn is_available_to_all(&self, family: &str, _files: &[PathBuf]) -> bool {
        self.available.lock().is_ok_and(|a| a.iter().any(|f| f.eq_ignore_ascii_case(family)))
    }

    fn make_available(&self, family: &str, _files: &[PathBuf]) -> Result<MadeAvailable, StoreError> {
        self.record(Call::MakeAvailable(family.to_owned()));
        if let Some(conflicts) = self.refusal.lock().map_err(err)?.take() {
            return Ok(MadeAvailable::Refused(conflicts));
        }
        self.pending.lock().map_err(err)?.remove(&family.to_lowercase());
        let mut available = self.available.lock().map_err(err)?;
        if !available.iter().any(|f| f.eq_ignore_ascii_case(family)) {
            available.push(family.to_owned());
        }
        Ok(MadeAvailable::Done)
    }

    fn withdraw(&self, family: &str, _files: &[PathBuf]) -> Result<Vec<PathBuf>, StoreError> {
        self.record(Call::Withdraw(family.to_owned()));
        self.available.lock().map_err(err)?.retain(|f| !f.eq_ignore_ascii_case(family));
        Ok(std::mem::take(&mut *self.kept.lock().map_err(err)?))
    }

    fn not_yet_available(&self, family: &str, files: &[PathBuf]) -> Vec<String> {
        if !self.is_available_to_all(family, files) {
            return Vec::new();
        }
        self.pending.lock().ok().and_then(|p| p.get(&family.to_lowercase()).cloned()).unwrap_or_default()
    }

    /// Served from the file table by the index path, like a font.
    fn legal_text(&self, _: &Index, family: &Family, which: LegalText) -> Result<Option<String>, StoreError> {
        let path = match which {
            LegalText::License => family.license_file.as_ref().map(|f| f.path.as_str()).or(family.license_path.as_deref()),
            LegalText::Notice => family.notice_file.as_ref().map(|f| f.path.as_str()),
        };
        let Some(path) = path else { return Ok(None) };
        if !self.downloads_allowed() {
            return Err(StoreError("downloading fonts is turned off".into()));
        }
        let bytes = self.files.get(path).ok_or_else(|| StoreError(format!("{path} isn't available")))?;
        Ok(Some(String::from_utf8_lossy(bytes).into_owned()))
    }
}
