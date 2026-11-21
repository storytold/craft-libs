//! The font preview service's client: subset URLs, a pool that fetches and decodes them, and a
//! disk cache.
//!
//! The service serves a small subset of one font file per preview, addressed by the upstream
//! file's sha256: the face's default sample, printable ASCII, or a set of codepoints, at the
//! instance the list shows or with every axis kept. Responses never change, so the cache never
//! revalidates. With the service at `https://fonts.example.com`, the list's sample of a face is
//! `https://fonts.example.com/v1/faces/<sha256>/list/sample-<h>.woff2`.
//!
//! [`Previews`] requests nothing unless a preview service is set and previews are enabled
//! ([`Installer::preview_service`], [`Installer::previews_enabled`]). Previews are on by default;
//! a user setting turns them off. There is no prompt, and the install consent
//! ([`Installer::downloads_allowed`]) is not consulted: previews are fetched whether or not
//! installs are allowed. With previews off nothing goes to the network, but previews already in
//! the disk cache are still used. Without a service the panel keeps its whole-family previews.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::sync::{Arc, Condvar, Mutex, MutexGuard};
use std::time::{Duration, SystemTime};

use craft_font_store::{Cancel, Client, FetchError, Timeouts, is_font, sha256_hex};

use crate::store::Installer;

/// The most codepoints one request may name; the service rejects more.
pub const MAX_CODEPOINTS: usize = 256;

/// The largest response accepted. Subsets are a few KB; a `var` subset of CJK text is the largest.
pub const MAX_PREVIEW_BYTES: u64 = 8 << 20;

/// The disk cache's size.
pub const CACHE_LIMIT: u64 = 50 << 20;

/// Requests at a time.
pub const WORKERS: usize = 8;

/// A set of codepoints, as a subset request names them.
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
pub struct Codepoints(BTreeSet<char>);

impl Codepoints {
    /// The characters of `text` without C0 and C1 controls and DEL, which no font draws.
    pub fn from_text(text: &str) -> Codepoints {
        text.chars().collect()
    }

    pub fn len(&self) -> usize {
        self.0.len()
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    pub fn contains(&self, c: char) -> bool {
        self.0.contains(&c)
    }

    /// In ascending order.
    pub fn iter(&self) -> impl Iterator<Item = char> + '_ {
        self.0.iter().copied()
    }

    /// Keeps the codepoints `keep` accepts, such as those a face covers.
    pub fn retain(&mut self, mut keep: impl FnMut(char) -> bool) {
        self.0.retain(|&c| keep(c));
    }

    /// The canonical form the service accepts, and so the cache key: ascending lowercase hex
    /// without leading zeros, a run of two or more as `first-last`, joined by commas
    /// (`20,41-5a,e9`). Empty for the empty set.
    pub fn encode(&self) -> String {
        let mut out = String::new();
        let mut rest = self.0.iter().map(|&c| c as u32).peekable();
        while let Some(first) = rest.next() {
            let mut last = first;
            while rest.peek() == Some(&(last + 1)) {
                last += 1;
                rest.next();
            }
            if !out.is_empty() {
                out.push(',');
            }
            let _ = if last == first { write!(out, "{first:x}") } else { write!(out, "{first:x}-{last:x}") };
        }
        out
    }
}

impl FromIterator<char> for Codepoints {
    /// Without C0 and C1 controls and DEL.
    fn from_iter<I: IntoIterator<Item = char>>(chars: I) -> Codepoints {
        Codepoints(chars.into_iter().filter(|&c| !matches!(c as u32, 0..0x20 | 0x7f..=0x9f)).collect())
    }
}

/// Which instance of a face a subset is drawn at.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Instance {
    /// What the list shows: the "Regular" named instance of a variable face, else its default,
    /// pinned to a static font.
    List,
    /// Every axis kept, for the detail view's styles and sliders.
    Var,
}

impl Instance {
    fn as_str(self) -> &'static str {
        match self {
            Instance::List => "list",
            Instance::Var => "var",
        }
    }
}

/// The first 12 lowercase hex digits of the sha256 of a sample text (UTF-8, as the index has
/// it), which names the sample subset for that text.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct SampleHash(String);

impl SampleHash {
    pub fn of(text: &str) -> SampleHash {
        SampleHash(sha256_hex(text.as_bytes())[..12].to_string())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Which characters a subset holds.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum Subset {
    /// The face's default sample (its script's sentence from the index) and the family name.
    /// The hash names the text the client has; the service answers 404 for any other.
    Sample(SampleHash),
    /// Printable ASCII, U+0020 to U+007E, for faces that cover Latin.
    Ascii,
    /// These codepoints, for typed text the static subsets don't cover.
    Codepoints(Codepoints),
}

/// One subset of one face: a request, a cache entry and the key results come back under.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct PreviewKey {
    sha256: String,
    instance: Instance,
    subset: Subset,
}

impl PreviewKey {
    /// The subset of the font file with this sha256 (as the index has it). `None` for a sha256
    /// that isn't 64 hex digits, and for codepoints that are empty or more than
    /// [`MAX_CODEPOINTS`].
    pub fn new(sha256: &str, instance: Instance, subset: Subset) -> Option<PreviewKey> {
        if sha256.len() != 64 || !sha256.bytes().all(|b| b.is_ascii_hexdigit()) {
            return None;
        }
        if let Subset::Codepoints(c) = &subset
            && (c.is_empty() || c.len() > MAX_CODEPOINTS)
        {
            return None;
        }
        Some(PreviewKey { sha256: sha256.to_ascii_lowercase(), instance, subset })
    }

    pub fn sha256(&self) -> &str {
        &self.sha256
    }

    pub fn instance(&self) -> Instance {
        self.instance
    }

    pub fn subset(&self) -> &Subset {
        &self.subset
    }

    /// The path below the service's base URL: `v1/faces/<sha256>/<instance>/sample-<h>.woff2`,
    /// `.../ascii.woff2` or `.../u/<codepoints>.woff2`.
    pub fn path(&self) -> String {
        let what = match &self.subset {
            Subset::Sample(h) => format!("sample-{}", h.as_str()),
            Subset::Ascii => "ascii".to_string(),
            Subset::Codepoints(c) => format!("u/{}", c.encode()),
        };
        format!("v1/faces/{}/{}/{what}.woff2", self.sha256, self.instance.as_str())
    }

    /// The URL at the service whose base URL is `service`.
    pub fn url(&self, service: &str) -> String {
        format!("{}/{}", service.trim_end_matches('/'), self.path())
    }
}

/// Why a preview didn't load.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PreviewError {
    /// The user turned previews off, and the subset isn't cached.
    PreviewsOff,
    /// No preview service is set.
    NoService,
    /// The request failed: the network, an HTTP status (503 while the service is busy), the
    /// size limit.
    Failed(String),
    /// The service has no such subset (HTTP 404): for a sample, its text isn't the client's.
    NotFound,
    /// The response isn't a WOFF2 font with outlines.
    NotAFont,
}

impl std::fmt::Display for PreviewError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PreviewError::PreviewsOff => f.write_str("font previews are turned off"),
            PreviewError::NoService => f.write_str("no preview service is set"),
            PreviewError::Failed(e) => f.write_str(e),
            PreviewError::NotFound => f.write_str("the service has no such preview"),
            PreviewError::NotAFont => f.write_str("the preview is not a font, or a damaged one"),
        }
    }
}

impl std::error::Error for PreviewError {}

/// A finished request.
#[derive(Debug)]
pub struct Preview {
    pub key: PreviewKey,
    /// The decoded font file (TrueType or OpenType), ready to register.
    pub font: Result<Vec<u8>, PreviewError>,
}

/// How the pool downloads: craft-font-store's [`Client`], or a stand-in in tests.
pub trait Fetch: Send + Sync {
    /// GETs `url`, built from the preview-service setting, at most `max` bytes, stopping soon after `cancel` is set.
    fn get(&self, url: &str, max: u64, cancel: &Cancel) -> Result<Vec<u8>, FetchError>;
}

impl Fetch for Client {
    fn get(&self, url: &str, max: u64, cancel: &Cancel) -> Result<Vec<u8>, FetchError> {
        Client::get_from_service(self, url, max, cancel, &mut |_| {})
    }
}

/// Subsets as the service sent them (WOFF2), one file per key, named by the sha256 of the key's
/// path. The least recently used go once the folder holds more than its limit. Several apps can
/// share the folder: a file another one removed is a miss.
pub struct DiskCache {
    dir: PathBuf,
    limit: u64,
    /// Read from the folder on first use.
    entries: Mutex<Option<Entries>>,
}

#[derive(Default)]
struct Entries {
    /// File name to its size and last use.
    files: HashMap<String, (u64, u64)>,
    /// Last use to file name, oldest first.
    order: BTreeMap<u64, String>,
    total: u64,
    tick: u64,
}

impl Entries {
    fn touch(&mut self, name: &str, size: u64) {
        self.tick += 1;
        if let Some((old_size, old_tick)) = self.files.insert(name.to_string(), (size, self.tick)) {
            self.order.remove(&old_tick);
            self.total = self.total.saturating_sub(old_size);
        }
        self.order.insert(self.tick, name.to_string());
        self.total = self.total.saturating_add(size);
    }

    fn forget(&mut self, name: &str) {
        if let Some((size, tick)) = self.files.remove(name) {
            self.order.remove(&tick);
            self.total = self.total.saturating_sub(size);
        }
    }

    /// Removes the oldest files, except `keep`, until the rest fit in `limit`.
    fn evict(&mut self, dir: &Path, limit: u64, keep: Option<&str>) {
        while self.total > limit {
            let Some(name) = self.order.values().find(|n| Some(n.as_str()) != keep).cloned() else { break };
            let _ = std::fs::remove_file(dir.join(&name));
            self.forget(&name);
        }
    }
}

/// A temporary file older than this belongs to a write that never finished.
const STALE_TEMP: Duration = Duration::from_secs(600);

impl DiskCache {
    /// A cache in `dir` (created on the first write) of at most `limit` bytes.
    pub fn new(dir: impl Into<PathBuf>, limit: u64) -> DiskCache {
        DiskCache { dir: dir.into(), limit, entries: Mutex::new(None) }
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    /// The bytes cached for `path` ([`PreviewKey::path`]), marking them as just used.
    pub fn get(&self, path: &str) -> Option<Vec<u8>> {
        let name = file_name(path);
        let file = self.dir.join(&name);
        let bytes = std::fs::metadata(&file).ok().filter(|m| m.len() <= MAX_PREVIEW_BYTES).and_then(|_| std::fs::read(&file).ok());
        match bytes {
            Some(bytes) => {
                self.with(|e| e.touch(&name, bytes.len() as u64));
                // The modification time keeps the order for the next session.
                let _ = std::fs::File::options().write(true).open(&file).and_then(|f| f.set_modified(SystemTime::now()));
                Some(bytes)
            }
            None => {
                self.with(|e| e.forget(&name));
                None
            }
        }
    }

    /// Whether `path` is cached, without marking it used.
    pub fn contains(&self, path: &str) -> bool {
        self.with(|e| e.files.contains_key(&file_name(path)))
    }

    /// Caches `bytes` for `path`, then removes the least recently used past the limit. Bytes
    /// larger than the whole limit aren't cached.
    pub fn put(&self, path: &str, bytes: &[u8]) {
        let size = bytes.len() as u64;
        if size > self.limit {
            return;
        }
        let name = file_name(path);
        if std::fs::create_dir_all(&self.dir).is_err() {
            return;
        }
        // A reader never sees half a file: write beside it, then rename.
        static N: AtomicU64 = AtomicU64::new(0);
        let temp = self.dir.join(format!(".tmp-{}-{}", std::process::id(), N.fetch_add(1, Ordering::Relaxed)));
        if std::fs::write(&temp, bytes).and_then(|()| std::fs::rename(&temp, self.dir.join(&name))).is_err() {
            let _ = std::fs::remove_file(&temp);
            return;
        }
        self.with(|e| {
            e.touch(&name, size);
            e.evict(&self.dir, self.limit, Some(&name));
        });
    }

    /// Removes the entry for `path`, as when it doesn't decode.
    pub fn remove(&self, path: &str) {
        let name = file_name(path);
        let _ = std::fs::remove_file(self.dir.join(&name));
        self.with(|e| e.forget(&name));
    }

    /// Bytes held.
    pub fn size(&self) -> u64 {
        self.with(|e| e.total)
    }

    fn with<R>(&self, f: impl FnOnce(&mut Entries) -> R) -> R {
        let mut guard = self.entries.lock().unwrap_or_else(|e| e.into_inner());
        f(guard.get_or_insert_with(|| self.scan()))
    }

    /// The files in the folder, oldest modification first; drops stale temporary files and
    /// whatever no longer fits.
    fn scan(&self) -> Entries {
        let mut found = Vec::new();
        for entry in std::fs::read_dir(&self.dir).into_iter().flatten().flatten() {
            let name = entry.file_name().to_string_lossy().into_owned();
            let Ok(meta) = entry.metadata() else { continue };
            let modified = meta.modified().unwrap_or(SystemTime::UNIX_EPOCH);
            if name.starts_with(".tmp-") {
                if modified.elapsed().is_ok_and(|age| age > STALE_TEMP) {
                    let _ = std::fs::remove_file(entry.path());
                }
            } else if meta.is_file() && is_entry_name(&name) {
                found.push((modified, name, meta.len()));
            }
        }
        found.sort();
        let mut entries = Entries::default();
        for (_, name, size) in found {
            entries.touch(&name, size);
        }
        entries.evict(&self.dir, self.limit, None);
        entries
    }
}

fn file_name(path: &str) -> String {
    format!("{}.woff2", sha256_hex(path.as_bytes()))
}

fn is_entry_name(name: &str) -> bool {
    name.strip_suffix(".woff2").is_some_and(|h| h.len() == 64 && h.bytes().all(|b| b.is_ascii_hexdigit()))
}

type Wake = Arc<dyn Fn() + Send + Sync>;

/// Fetches subsets on [`WORKERS`] threads of its own, separate from the panel's jobs, highest
/// priority first. Results wait in [`Previews::poll`]; a cancelled request delivers nothing.
///
/// Each request reads the disk cache first. Only on a miss, and only while previews are enabled
/// and a service is set, does it go to the network; the response is decoded, checked and cached.
/// Without a service, requests come back as [`PreviewError::NoService`] at once and nothing is
/// read or fetched.
pub struct Previews {
    installer: Arc<dyn Installer>,
    fetcher: Arc<dyn Fetch>,
    cache: Option<Arc<DiskCache>>,
    wake: Option<Wake>,
    workers: usize,
    shared: Arc<Shared>,
    tx: Sender<Preview>,
    rx: Receiver<Preview>,
}

struct Shared {
    state: Mutex<State>,
    ready: Condvar,
}

#[derive(Default)]
struct State {
    /// Waiting: priority and order of arrival.
    queued: HashMap<PreviewKey, (u32, u64)>,
    /// Being fetched: the job's id and its cancel flag.
    running: HashMap<PreviewKey, (u64, Cancel)>,
    next: u64,
    started: bool,
    stop: bool,
}

impl State {
    /// The highest priority, then the oldest.
    fn take_next(&mut self) -> Option<PreviewKey> {
        let key = self.queued.iter().max_by_key(|(_, (priority, seq))| (*priority, std::cmp::Reverse(*seq))).map(|(k, _)| k.clone())?;
        self.queued.remove(&key);
        Some(key)
    }

    fn is_running(&self, key: &PreviewKey) -> bool {
        self.running.get(key).is_some_and(|(_, c)| !c.is_cancelled())
    }
}

impl Previews {
    /// A pool fetching with the installer's client ([`Installer::http_client`]) given short timeouts
    /// (10 s to connect, 15 s to respond, 30 s overall; the down state stays shared), else with its
    /// own craft-font-store [`Client`]. A network failure of the service pauses its requests for a
    /// minute, for every user of the same client (installs too). It caches in the store's
    /// [`Installer::preview_cache`] folder, up to [`CACHE_LIMIT`]. It starts its threads on the
    /// first request.
    pub fn new(installer: Arc<dyn Installer>) -> Previews {
        let timeouts = Timeouts { connect: Duration::from_secs(10), response: Duration::from_secs(15), overall: Duration::from_secs(30) };
        let client = installer.http_client().map_or_else(|| Client::new(timeouts), |c| c.with_timeouts(timeouts));
        let cache = installer.preview_cache().map(|dir| Arc::new(DiskCache::new(dir, CACHE_LIMIT)));
        let (tx, rx) = channel();
        Previews {
            installer,
            fetcher: Arc::new(client),
            cache,
            wake: None,
            workers: WORKERS,
            shared: Arc::new(Shared { state: Mutex::new(State::default()), ready: Condvar::new() }),
            tx,
            rx,
        }
    }

    /// Fetches through `fetcher` instead.
    pub fn with_fetcher(mut self, fetcher: Arc<dyn Fetch>) -> Previews {
        self.fetcher = fetcher;
        self
    }

    /// Caches in `cache`, or with `None` not at all.
    pub fn with_cache(mut self, cache: Option<DiskCache>) -> Previews {
        self.cache = cache.map(Arc::new);
        self
    }

    /// Runs `workers` requests at a time (at least one) instead of [`WORKERS`].
    pub fn with_workers(mut self, workers: usize) -> Previews {
        self.workers = workers.max(1);
        self
    }

    /// Calls `wake` from a worker thread after each result, such as egui's `request_repaint`.
    pub fn on_ready(mut self, wake: impl Fn() + Send + Sync + 'static) -> Previews {
        self.wake = Some(Arc::new(wake));
        self
    }

    /// Whether a preview service is set and previews are enabled. It reads the store's settings, so callers ask when the
    /// panel opens or its settings change, not every frame.
    pub fn available(&self) -> bool {
        self.installer.preview_service().is_some() && self.installer.previews_enabled()
    }

    /// The disk cache, if there is one.
    pub fn cache(&self) -> Option<&DiskCache> {
        self.cache.as_deref()
    }

    /// Queues `key` at `priority` (higher first; the same priority in the order asked). A key
    /// already queued takes the new priority; one being fetched is left alone.
    pub fn request(&self, key: PreviewKey, priority: u32) {
        let mut state = self.lock();
        if state.is_running(&key) {
            return;
        }
        if !state.started {
            state.started = true;
            self.start();
        }
        let seq = state.next;
        state.next += 1;
        let entry = state.queued.entry(key).or_insert((priority, seq));
        entry.0 = priority;
        drop(state);
        self.shared.ready.notify_one();
    }

    /// Drops `key` from the queue, or stops fetching it. Nothing is delivered for it, unless it
    /// finished before the call.
    pub fn cancel(&self, key: &PreviewKey) {
        self.retain(|k| k != key);
    }

    /// Cancels every queued and running request `keep` rejects, such as rows that scrolled out
    /// of the window.
    pub fn retain(&self, mut keep: impl FnMut(&PreviewKey) -> bool) {
        let mut state = self.lock();
        state.queued.retain(|k, _| keep(k));
        for (key, (_, cancel)) in &state.running {
            if !keep(key) {
                cancel.cancel();
            }
        }
    }

    /// Whether `key` is queued or being fetched.
    pub fn is_pending(&self, key: &PreviewKey) -> bool {
        let state = self.lock();
        state.queued.contains_key(key) || state.is_running(key)
    }

    /// Requests queued or being fetched.
    pub fn pending(&self) -> usize {
        let state = self.lock();
        state.queued.len() + state.running.values().filter(|(_, c)| !c.is_cancelled()).count()
    }

    /// The results since the last call.
    pub fn poll(&self) -> Vec<Preview> {
        self.rx.try_iter().collect()
    }

    fn lock(&self) -> MutexGuard<'_, State> {
        self.shared.state.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn start(&self) {
        for i in 0..self.workers {
            let worker = Worker {
                installer: Arc::clone(&self.installer),
                fetcher: Arc::clone(&self.fetcher),
                cache: self.cache.clone(),
                wake: self.wake.clone(),
                shared: Arc::clone(&self.shared),
                tx: self.tx.clone(),
            };
            // A pool short of threads still works, more slowly; with none, requests stay queued.
            let _ = std::thread::Builder::new().name(format!("font-previews-{i}")).spawn(move || worker.run());
        }
    }
}

impl Drop for Previews {
    fn drop(&mut self) {
        let mut state = self.lock();
        state.stop = true;
        state.queued.clear();
        for (_, cancel) in state.running.values() {
            cancel.cancel();
        }
        drop(state);
        self.shared.ready.notify_all();
    }
}

struct Worker {
    installer: Arc<dyn Installer>,
    fetcher: Arc<dyn Fetch>,
    cache: Option<Arc<DiskCache>>,
    wake: Option<Wake>,
    shared: Arc<Shared>,
    tx: Sender<Preview>,
}

impl Worker {
    fn run(self) {
        while let Some((key, id, cancel)) = self.next() {
            let font = self.load(&key, &cancel);
            let mut state = self.shared.state.lock().unwrap_or_else(|e| e.into_inner());
            if state.running.get(&key).is_some_and(|(i, _)| *i == id) {
                state.running.remove(&key);
            }
            if cancel.is_cancelled() {
                continue;
            }
            // Sent under the lock, so a cancel that returned has either dropped it or comes after.
            let _ = self.tx.send(Preview { key, font });
            drop(state);
            if let Some(wake) = &self.wake {
                wake();
            }
        }
    }

    /// Waits for a request; `None` once the pool is dropped.
    fn next(&self) -> Option<(PreviewKey, u64, Cancel)> {
        let mut state = self.shared.state.lock().unwrap_or_else(|e| e.into_inner());
        loop {
            if state.stop {
                return None;
            }
            if let Some(key) = state.take_next() {
                let id = state.next;
                state.next += 1;
                let cancel = Cancel::new();
                state.running.insert(key.clone(), (id, cancel.clone()));
                return Some((key, id, cancel));
            }
            state = self.shared.ready.wait(state).unwrap_or_else(|e| e.into_inner());
        }
    }

    fn load(&self, key: &PreviewKey, cancel: &Cancel) -> Result<Vec<u8>, PreviewError> {
        let Some(service) = self.installer.preview_service() else { return Err(PreviewError::NoService) };
        let path = key.path();
        if let Some(cache) = &self.cache
            && let Some(bytes) = cache.get(&path)
        {
            match decode(&bytes) {
                Ok(font) => return Ok(font),
                Err(_) => cache.remove(&path),
            }
        }
        if !self.installer.previews_enabled() {
            return Err(PreviewError::PreviewsOff);
        }
        let bytes = self.fetcher.get(&key.url(&service), MAX_PREVIEW_BYTES, cancel).map_err(|e| match e {
            FetchError::Failed(m) if m == "HTTP 404" => PreviewError::NotFound,
            e => PreviewError::Failed(e.to_string()),
        })?;
        let font = decode(&bytes)?;
        if let Some(cache) = &self.cache {
            cache.put(&path, &bytes);
        }
        Ok(font)
    }
}

/// The font file inside a WOFF2 response, when it parses and has outlines.
fn decode(woff2: &[u8]) -> Result<Vec<u8>, PreviewError> {
    let font = wuff::decompress_woff2(woff2).map_err(|_| PreviewError::NotAFont)?;
    if is_font(&font) { Ok(font) } else { Err(PreviewError::NotAFont) }
}
