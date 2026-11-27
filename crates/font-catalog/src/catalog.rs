//! The catalog: every installed face, found by any of its names, loaded once.

use std::collections::hash_map::Entry;
use std::collections::{HashMap, HashSet, VecDeque};
use std::hash::{DefaultHasher, Hash, Hasher};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, OnceLock, PoisonError, RwLock, Weak};
use std::time::{Duration, SystemTime};

use crate::scan::{self, Face, FaceInfo, Source, norm, style_italic, style_weight, style_width};
use crate::{FontEnv, Os, font_dirs, font_store_dir};

/// Lists font files that the platform's font service knows outside the font folders. Called by
/// every scan.
pub type Lister = Arc<dyn Fn() -> Vec<PathBuf> + Send + Sync>;

/// How deep the scan goes below a font folder.
const MAX_DEPTH: usize = 12;
/// The most folder entries one scan looks at.
const MAX_ENTRIES: usize = 200_000;
/// The most bytes the scan reads from one file (its tables, or a suitcase's resource fork).
const MAX_SCAN_READ: u64 = 256 << 20;
/// The largest font file loaded.
const MAX_LOAD: u64 = 512 << 20;

/// The bytes of the file at `path`, at most `max`: the size was checked before, but a file can
/// grow between that check and the read, so the read itself stops one byte past the limit.
pub(crate) fn read_capped(path: &Path, max: u64) -> Result<Vec<u8>, FontFileError> {
    use std::io::Read as _;
    let mut bytes = Vec::new();
    std::fs::File::open(path)?.take(max.saturating_add(1)).read_to_end(&mut bytes)?;
    let size = bytes.len() as u64;
    if size > max {
        return Err(FontFileError::TooLarge { size, max });
    }
    Ok(bytes)
}

/// Why a font file or font data wasn't added or loaded.
#[derive(Debug, thiserror::Error)]
pub enum FontFileError {
    /// Reading the file failed.
    #[error("the font file can't be read: {0}")]
    Io(#[from] std::io::Error),
    /// The file is larger than the limit.
    #[error("the font file is {size} bytes, more than the {max} allowed")]
    TooLarge {
        /// The file's size.
        size: u64,
        /// The limit.
        max: u64,
    },
    /// No face in it could be read.
    #[error("not a font file, or a damaged one")]
    NotAFont,
    /// Adding it would pass the limit on added faces.
    #[error("more than {max} added font faces")]
    TooManyFaces {
        /// The limit.
        max: usize,
    },
    /// The face is no longer in the catalog (its data was removed).
    #[error("the font is no longer available")]
    Gone,
}

/// Limits on fonts added from files or bytes ([`Catalog::add_bytes`], [`Catalog::add_file`]),
/// which may come from documents or downloads.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Limits {
    /// The largest file accepted.
    pub max_file_bytes: u64,
    /// The most faces read from one file (a collection).
    pub max_faces_per_file: u32,
    /// The most added faces the catalog holds in all.
    pub max_added_faces: usize,
}

impl Default for Limits {
    fn default() -> Self {
        Self { max_file_bytes: 256 << 20, max_faces_per_file: 256, max_added_faces: 1000 }
    }
}

/// How [`Catalog::resolve`] matched.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Match {
    /// The family and style asked for, by any of their names.
    Exact,
    /// The family, in the style nearest to the one asked for.
    NearestStyle,
}

/// A face's data: the whole file (or a suitcase font's resource) and the face's index in it.
#[derive(Clone, Debug)]
pub struct FontData {
    /// The file's bytes, shared by every face loaded from it.
    pub bytes: Arc<[u8]>,
    /// The index to pass to a font parser.
    pub index: u32,
}

/// A loaded face ([`Catalog::load_family`]).
#[derive(Clone, Debug)]
pub struct LoadedFace {
    /// The face.
    pub info: FaceInfo,
    /// Its data.
    pub data: FontData,
}

/// A face the scan can't find again, as an alias records it.
#[derive(Clone, Debug)]
enum Alias {
    /// The family of this face.
    Family(usize),
    /// A legacy family name: this face, under this legacy style.
    Legacy(usize, String),
    /// A full or PostScript name: this face.
    Face(usize),
}

/// One state of the catalog: what lookups read while a scan builds the next one.
#[derive(Default)]
struct Index {
    faces: Vec<Face>,
    infos: Arc<[FaceInfo]>,
    families: Arc<[String]>,
    by_family: HashMap<String, Vec<usize>>,
    aliases: HashMap<String, Vec<Alias>>,
    postscript: HashMap<String, usize>,
    /// The faces left out of `faces` for having a listed face's family, style, PostScript name
    /// and variations (another file's version of it), by the listed face.
    versions: HashMap<usize, Vec<FaceInfo>>,
    /// A hash of each family's faces by lowercase name, for [`Catalog::changes_since`].
    signatures: HashMap<String, u64>,
}

impl Index {
    /// Added faces first: a document's or the app's own face wins over an installed face of the
    /// same family and style. Among scanned duplicates the font store's face wins, so a store
    /// font copied into the OS font folder stays the store's wherever the folders sort.
    fn build(added: &[Face], scanned: &[Face]) -> Self {
        let all: Vec<&Face> = added.iter().chain(scanned).collect();
        // The positions in `all` of each face's versions, the listed one first.
        let mut groups: HashMap<_, Vec<usize>> = HashMap::new();
        for (i, f) in all.iter().enumerate() {
            let variations: Vec<(u32, u32)> = f.info.variations.iter().map(|(t, v)| (u32::from_be_bytes(*t), v.to_bits())).collect();
            let key = (f.info.family.to_ascii_lowercase(), norm(&f.info.style), f.keys.postscript.first().cloned(), variations);
            match groups.entry(key) {
                Entry::Vacant(e) => {
                    e.insert(vec![i]);
                }
                Entry::Occupied(mut e) => {
                    let group = e.get_mut();
                    let store_first =
                        group.first().is_some_and(|held| *held >= added.len() && all.get(*held).is_some_and(|h| h.info.source != Source::Store));
                    if store_first && f.info.source == Source::Store {
                        group.insert(0, i);
                    } else {
                        group.push(i);
                    }
                }
            }
        }
        let mut groups: Vec<Vec<usize>> = groups.into_values().collect();
        groups.sort_unstable();
        let mut faces: Vec<Face> = Vec::new();
        let mut versions: HashMap<usize, Vec<FaceInfo>> = HashMap::new();
        for group in &groups {
            let mut group = group.iter().filter_map(|i| all.get(*i));
            let Some(listed) = group.next() else { continue };
            let others: Vec<FaceInfo> = group.map(|f| f.info.clone()).collect();
            if !others.is_empty() {
                versions.insert(faces.len(), others);
            }
            faces.push((*listed).clone());
        }
        let mut by_family: HashMap<String, Vec<usize>> = HashMap::new();
        let mut aliases: HashMap<String, Vec<Alias>> = HashMap::new();
        let mut postscript: HashMap<String, usize> = HashMap::new();
        let mut families: Vec<String> = Vec::new();
        for (i, f) in faces.iter().enumerate() {
            let entry = by_family.entry(f.info.family.to_ascii_lowercase()).or_default();
            if entry.is_empty() {
                families.push(f.info.family.clone());
            }
            entry.push(i);
            for k in &f.keys.families {
                let list = aliases.entry(k.clone()).or_default();
                if !list
                    .iter()
                    .any(|a| matches!(a, Alias::Family(j) if faces.get(*j).is_some_and(|g| g.info.family.eq_ignore_ascii_case(&f.info.family))))
                {
                    list.push(Alias::Family(i));
                }
            }
            for (family, style) in &f.keys.legacy {
                aliases.entry(family.clone()).or_default().push(Alias::Legacy(i, style.clone()));
            }
            for k in f.keys.full_names.iter().chain(&f.keys.postscript) {
                aliases.entry(k.clone()).or_default().push(Alias::Face(i));
            }
            for k in &f.keys.postscript {
                postscript.entry(k.clone()).or_insert(i);
            }
        }
        // Narrow to wide, light to heavy, upright first. The sort is stable, so of two faces alike
        // the added one stays first and wins a tie in `pick_style`.
        for list in by_family.values_mut() {
            list.sort_by(|a, b| {
                let (Some(a), Some(b)) = (faces.get(*a), faces.get(*b)) else { return std::cmp::Ordering::Equal };
                (a.info.width.total_cmp(&b.info.width)).then(a.info.weight.total_cmp(&b.info.weight)).then(a.info.italic.cmp(&b.info.italic))
            });
        }
        families.sort_by_cached_key(|f| (f.to_lowercase(), f.clone()));
        let infos: Arc<[FaceInfo]> = faces.iter().map(|f| f.info.clone()).collect();
        let mut index = Self { faces, infos, families: families.into(), by_family, aliases, postscript, versions, signatures: HashMap::new() };
        index.signatures = index.family_signatures();
        index
    }

    fn family_signatures(&self) -> HashMap<String, u64> {
        self.by_family
            .iter()
            .map(|(key, faces)| {
                let mut h = DefaultHasher::new();
                for f in faces.iter().filter_map(|i| self.face(*i)) {
                    let i = &f.info;
                    (&i.family, &i.style, &i.postscript_name, &i.path, i.index, i.blob, i.source, i.italic).hash(&mut h);
                    (i.weight.to_bits(), i.width.to_bits()).hash(&mut h);
                    for (tag, value) in &i.variations {
                        (tag, value.to_bits()).hash(&mut h);
                    }
                }
                (key.clone(), h.finish())
            })
            .collect()
    }

    /// The name of the family with lowercase name `key`.
    fn family_named(&self, key: &str) -> Option<&str> {
        let i = *self.by_family.get(key)?.first()?;
        self.face(i).map(|f| f.info.family.as_str())
    }

    fn face(&self, i: usize) -> Option<&Face> {
        self.faces.get(i)
    }

    /// The family `name` names: a family name ignoring ASCII case, else any family name of a face
    /// (another language, legacy) ignoring case and everything but letters and digits.
    fn family_of(&self, name: &str) -> Option<&[usize]> {
        if let Some(faces) = self.by_family.get(&name.to_ascii_lowercase()) {
            return Some(faces);
        }
        let aliases = self.aliases.get(&norm(name))?;
        let i = aliases.iter().find_map(|a| match a {
            Alias::Family(i) => Some(*i),
            _ => None,
        })?;
        self.by_family.get(&self.face(i)?.info.family.to_ascii_lowercase()).map(Vec::as_slice)
    }

    fn resolve(&self, name: &str, style: &str) -> Option<(usize, Match)> {
        if let Some(faces) = self.family_of(name) {
            return self.pick_style(faces, style);
        }
        let aliases = self.aliases.get(&norm(name))?;
        let legacy: Vec<(usize, &str)> = aliases
            .iter()
            .filter_map(|a| match a {
                Alias::Legacy(i, s) => Some((*i, s.as_str())),
                _ => None,
            })
            .collect();
        if !legacy.is_empty() {
            let ns = style_key(style);
            if let Some((i, _)) = legacy.iter().find(|(_, s)| *s == ns) {
                return Some((*i, Match::Exact));
            }
            let faces: Vec<usize> = legacy.iter().map(|(i, _)| *i).collect();
            return self.pick_style(&faces, style).map(|(i, _)| (i, Match::NearestStyle));
        }
        aliases.iter().find_map(|a| match a {
            Alias::Face(i) => Some((*i, Match::Exact)),
            _ => None,
        })
    }

    /// The face of `faces` (one family) in `style`: by its style name, then by any other name of
    /// its style, else the nearest weight with italic differing costing 1000, then the nearest
    /// width; the first face wins a tie.
    fn pick_style(&self, faces: &[usize], style: &str) -> Option<(usize, Match)> {
        let ns = style_key(style);
        let cands: Vec<(usize, &Face)> = faces.iter().filter_map(|i| Some((*i, self.face(*i)?))).collect();
        if let Some((i, _)) = cands.iter().find(|(_, f)| norm(&f.info.style) == ns) {
            return Some((*i, Match::Exact));
        }
        if let Some((i, _)) = cands.iter().find(|(_, f)| f.keys.styles.contains(&ns)) {
            return Some((*i, Match::Exact));
        }
        let (tw, ti, twd) = (style_weight(style), style_italic(style), style_width(style));
        let score = |f: &Face| ((f.info.weight - tw).abs() + if f.info.italic != ti { 1000.0 } else { 0.0 }, (f.info.width - twd).abs());
        cands
            .iter()
            .min_by(|(_, a), (_, b)| {
                let (sa, sb) = (score(a), score(b));
                sa.0.total_cmp(&sb.0).then(sa.1.total_cmp(&sb.1))
            })
            .map(|(i, _)| (*i, Match::NearestStyle))
    }
}

/// How many changes of the family set the history keeps ([`Catalog::changes_since`]).
const HISTORY_CHANGES: usize = 64;
/// How many family names the history holds in all. A change naming more (the first scan of a
/// large system) clears the history.
const HISTORY_NAMES: usize = 4096;

/// How a family changed between two indexes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Change {
    Added,
    Removed,
    Changed,
}

/// A family's lowercase name, its name and how it changed.
type FamilyChange = (String, String, Change);

/// The families each recent generation added, removed or changed.
#[derive(Default)]
struct History {
    /// Each generation that changed a family, oldest first.
    changes: VecDeque<(u64, Vec<FamilyChange>)>,
    names: usize,
    /// The oldest generation [`Catalog::changes_since`] can start from: the history holds every
    /// change after it.
    floor: u64,
}

impl History {
    fn record(&mut self, generation: u64, old: &Index, new: &Index) {
        let mut changes = Vec::new();
        for (key, sig) in &new.signatures {
            let change = match old.signatures.get(key) {
                None => Change::Added,
                Some(old) if old != sig => Change::Changed,
                Some(_) => continue,
            };
            changes.push((key.clone(), new.family_named(key).unwrap_or(key).to_owned(), change));
        }
        for key in old.signatures.keys().filter(|k| !new.signatures.contains_key(*k)) {
            changes.push((key.clone(), old.family_named(key).unwrap_or(key).to_owned(), Change::Removed));
        }
        if changes.is_empty() {
            return;
        }
        if changes.len() > HISTORY_NAMES {
            self.changes.clear();
            self.names = 0;
            self.floor = generation;
            return;
        }
        self.names += changes.len();
        self.changes.push_back((generation, changes));
        while self.changes.len() > HISTORY_CHANGES || self.names > HISTORY_NAMES {
            let Some((generation, changes)) = self.changes.pop_front() else { break };
            self.names -= changes.len();
            self.floor = generation;
        }
    }
}

/// The families lookups find differently since a generation ([`Catalog::changes_since`]): what
/// an app re-resolves to know which documents to lay out again. Each list is sorted ignoring
/// case.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Changes {
    /// The generation these changes lead to: pass it to the next call.
    pub generation: u64,
    /// The catalog no longer knows what changed since then: re-resolve every family, and the
    /// lists are empty.
    pub full: bool,
    /// Families found now that weren't.
    pub added: Vec<String>,
    /// Families that were found and aren't any more.
    pub removed: Vec<String>,
    /// Families found before and now whose faces changed: a style installed or removed, a file
    /// replaced, a face added over an installed one.
    pub changed: Vec<String>,
}

impl Changes {
    /// Nothing changed.
    pub fn is_empty(&self) -> bool {
        !self.full && self.added.is_empty() && self.removed.is_empty() && self.changed.is_empty()
    }
}

/// A style name as compared: normalized, with no style meaning `Regular`.
fn style_key(style: &str) -> String {
    let ns = norm(style);
    if ns.is_empty() { "regular".into() } else { ns }
}

/// What the catalog reads besides the platform's font folders.
struct Config {
    app_dirs: Vec<PathBuf>,
    /// Folders a link inside is followed from only when it leads inside one of them ([`Catalog::set_confined_dirs`]).
    confined_dirs: Vec<PathBuf>,
    listers: Vec<Lister>,
    font_service: bool,
}

/// A file found by a scan, remembered so a rescan reads only files that changed.
struct Cached {
    len: u64,
    modified: Option<SystemTime>,
    faces: Vec<Face>,
}

#[derive(Default)]
struct ScanState {
    files: HashMap<PathBuf, Cached>,
}

/// What a change check compares with.
#[derive(Default)]
struct Stamp {
    app_dirs: Vec<PathBuf>,
    /// Every folder the scan listed, and the folders holding the files listers named, with their
    /// modification times (`None`: missing).
    dirs: Vec<(PathBuf, Option<SystemTime>)>,
    /// The font store's folders among `dirs`.
    store_dirs: Vec<(PathBuf, Option<SystemTime>)>,
    /// The fonts registered with Windows.
    registered: Vec<PathBuf>,
}

#[derive(Default)]
struct Added {
    faces: Vec<Face>,
    blobs: HashMap<u64, Arc<[u8]>>,
    next_blob: u64,
}

/// A family's faces, loaded once ([`Catalog::load_family`]).
type FamilyLoad = Arc<OnceLock<Arc<[LoadedFace]>>>;

/// The installed fonts: the platform's font folders, the files its font service knows, the font
/// store and the app's own folders, plus faces added from files or bytes.
///
/// The first scan runs on the first lookup or on a background thread ([`Catalog::scan_in_background`]);
/// lookups wait for it. Lookups read an immutable snapshot, so they never wait for a rescan.
/// Each family's files load once ([`Catalog::load_family`]): other threads asking for the same
/// family wait for that load, other families load at the same time.
pub struct Catalog {
    id: u64,
    env: FontEnv,
    config: RwLock<Config>,
    index: RwLock<Arc<Index>>,
    ready: OnceLock<()>,
    /// The first scan was started on a background thread.
    scanning: AtomicBool,
    scan: Mutex<ScanState>,
    /// The faces the last scan found, by path: what an added face is indexed with, read without waiting for a
    /// running scan.
    scanned: Mutex<Arc<[Face]>>,
    stamp: Mutex<Stamp>,
    generation: AtomicU64,
    /// Locked while [`Catalog::publish`] moves the generation on, so it and the history agree.
    history: Mutex<History>,
    added: Mutex<Added>,
    loaded: Mutex<HashMap<String, FamilyLoad>>,
    /// The bytes of each file read, while a face of it is loaded. A file's own lock is held while it is read, so faces of
    /// one collection loading on several threads read it once and share the bytes.
    files: Mutex<HashMap<PathBuf, FileSlot>>,
}

/// One file's shared bytes while any face of it is loaded, behind the lock held while the file is read.
type FileSlot = Arc<Mutex<Option<Weak<[u8]>>>>;

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

fn modified(path: &Path) -> Option<SystemTime> {
    std::fs::metadata(path).and_then(|m| m.modified()).ok()
}

/// The faces of the font file at `path` as a scan reads them, named instances included, with
/// `source` as their source. Empty when it isn't a font.
pub fn read_faces(path: &Path, source: Source) -> Vec<FaceInfo> {
    scan::file_faces(path, MAX_SCAN_READ, source).into_iter().map(|f| f.info).collect()
}

impl Catalog {
    /// A catalog of the fonts installed on the system `env` describes, the font store, and the
    /// files the platform's font service lists (with the `system` feature).
    pub fn new(env: FontEnv) -> Self {
        static NEXT_ID: AtomicU64 = AtomicU64::new(1);
        Self {
            id: NEXT_ID.fetch_add(1, Ordering::Relaxed),
            env,
            config: RwLock::new(Config {
                app_dirs: Vec::new(),
                confined_dirs: Vec::new(),
                listers: Vec::new(),
                font_service: cfg!(feature = "system"),
            }),
            index: RwLock::new(Arc::new(Index::default())),
            ready: OnceLock::new(),
            scanning: AtomicBool::new(false),
            scan: Mutex::new(ScanState::default()),
            scanned: Mutex::new(Arc::from([])),
            stamp: Mutex::new(Stamp::default()),
            generation: AtomicU64::new(0),
            history: Mutex::new(History::default()),
            added: Mutex::new(Added::default()),
            loaded: Mutex::new(HashMap::new()),
            files: Mutex::new(HashMap::new()),
        }
    }

    /// A catalog of exactly `files`, built on the existing scan with no font folders and no store.
    pub fn of_files(files: Vec<PathBuf>) -> Catalog {
        Catalog::new(FontEnv { os: Os::None, ..FontEnv::default() }).with_font_service(false).with_lister(Arc::new(move || files.clone()))
    }

    /// The process's catalog, for [`FontEnv::from_process`].
    pub fn global() -> &'static Arc<Catalog> {
        static GLOBAL: OnceLock<Arc<Catalog>> = OnceLock::new();
        GLOBAL.get_or_init(|| Arc::new(Catalog::new(FontEnv::from_process())))
    }

    /// Whether scans ask the platform's font service (CoreText, DirectWrite, fontconfig) for the
    /// font files it knows: on by default with the `system` feature, which provides it.
    #[must_use]
    pub fn with_font_service(self, on: bool) -> Self {
        self.write_config().font_service = on && cfg!(feature = "system");
        self
    }

    /// Have every scan also read the files `lister` names (any file name; a folder is read with
    /// its subfolders), as fonts of the system.
    #[must_use]
    pub fn with_lister(self, lister: Lister) -> Self {
        self.write_config().listers.push(lister);
        self
    }

    /// The app's own font folders (its Fonts folder, Office's, folders the user picked), read
    /// with their subfolders from the next scan on. A change shows in
    /// [`Catalog::installed_fonts_changed`].
    pub fn set_app_dirs(&self, dirs: Vec<PathBuf>) {
        self.write_config().app_dirs = dirs;
    }

    /// Folders in which a link is followed only when it stays inside a confined folder, from the next scan on: a link
    /// in one of them (or in a folder below) that leads outside all of them is skipped, so a folder the user picked
    /// reads no fonts from elsewhere.
    pub fn set_confined_dirs(&self, dirs: Vec<PathBuf>) {
        self.write_config().confined_dirs = dirs;
    }

    fn write_config(&self) -> std::sync::RwLockWriteGuard<'_, Config> {
        self.config.write().unwrap_or_else(PoisonError::into_inner)
    }

    fn read_config(&self) -> std::sync::RwLockReadGuard<'_, Config> {
        self.config.read().unwrap_or_else(PoisonError::into_inner)
    }

    /// Whether scans ask the platform's font service ([`Catalog::with_font_service`]).
    pub(crate) fn uses_font_service(&self) -> bool {
        self.read_config().font_service
    }

    /// The environment the catalog was made for.
    pub fn env(&self) -> &FontEnv {
        &self.env
    }

    /// Start the first scan on a background thread, so the first lookup doesn't wait for all of
    /// it. Does nothing once it has started or run, or when no thread can be started (the first
    /// lookup then scans).
    pub fn scan_in_background(self: &Arc<Self>) {
        if self.ready.get().is_none() && !self.scanning.swap(true, Ordering::AcqRel) {
            let me = Arc::clone(self);
            if std::thread::Builder::new().name("font-catalog-scan".into()).spawn(move || me.background_scan()).is_err() {
                self.scanning.store(false, Ordering::Release);
            }
        }
    }

    /// The background thread's scan. A scan that panics leaves the catalog unscanned and not
    /// scanning, so the next [`Catalog::scan_in_background`] or lookup scans again.
    fn background_scan(&self) {
        struct Done<'a>(&'a Catalog);
        impl Drop for Done<'_> {
            fn drop(&mut self) {
                if self.0.ready.get().is_none() {
                    self.0.scanning.store(false, Ordering::Release);
                }
            }
        }
        let _done = Done(self);
        self.ensure();
    }

    fn ensure(&self) {
        self.ready.get_or_init(|| {
            self.scan_now(false);
        });
    }

    /// Whether the first scan has finished: lookups won't wait.
    pub fn is_ready(&self) -> bool {
        self.ready.get().is_some()
    }

    /// Scan again, for fonts installed or removed since; only new and changed files are read.
    /// Families loaded before load again on their next [`Catalog::load_family`]. Returns the
    /// number of faces cataloged: the faces [`Catalog::faces`] lists, added faces included, each
    /// face once however many files have it ([`Catalog::versions`]).
    pub fn rescan(&self) -> usize {
        let mut first = false;
        self.ready.get_or_init(|| {
            first = true;
            self.scan_now(false);
        });
        if !first {
            self.scan_now(true);
        }
        self.snapshot().faces.len()
    }

    /// [`Catalog::rescan`] when [`Catalog::installed_fonts_changed`] says so, for an app to call
    /// when its window is activated. Returns whether it rescanned.
    pub fn rescan_if_changed(&self) -> bool {
        let changed = self.installed_fonts_changed();
        if changed {
            self.rescan();
        }
        changed
    }

    /// Whether fonts were installed or removed since the last scan: a folder the scan read (or
    /// the folder of a file the font service named) changed, a missing one appeared, the app's
    /// folders changed, or on Windows the registry's font list changed. It looks at folders,
    /// not files, so it costs far less than a scan. `false` before the first scan.
    ///
    /// Fonts a font service registers from a folder no scan has seen are found by the next
    /// [`Catalog::rescan`] only.
    pub fn installed_fonts_changed(&self) -> bool {
        if !self.is_ready() {
            return false;
        }
        let stamp = lock(&self.stamp);
        if stamp.app_dirs != self.read_config().app_dirs {
            return true;
        }
        if stamp.dirs.iter().any(|(p, t)| modified(p) != *t) {
            return true;
        }
        self.read_config().font_service && self.env.os == Os::Windows && stamp.registered != crate::registry::registered_font_files()
    }

    /// Whether the font store's folders ([`font_store_dir`]) changed since the last scan: the
    /// suite's font store installed or removed a font. It reads only the store's folders, so it
    /// costs far less than [`Catalog::installed_fonts_changed`] and can be polled often.
    /// `false` before the first scan.
    pub fn store_changed(&self) -> bool {
        self.is_ready() && lock(&self.stamp).store_dirs.iter().any(|(p, t)| modified(p) != *t)
    }

    /// Poll for installed and removed fonts every `interval` on a background thread: when
    /// [`Catalog::installed_fonts_changed`], rescan and call `on_change`. Stops when the returned
    /// [`Watch`] or the catalog is dropped.
    pub fn watch(self: &Arc<Self>, interval: Duration, on_change: impl Fn(&Catalog) + Send + 'static) -> Watch {
        let stop = Arc::new(AtomicBool::new(false));
        let (me, flag) = (Arc::downgrade(self), Arc::clone(&stop));
        let _ = std::thread::Builder::new().name("font-catalog-watch".into()).spawn(move || {
            loop {
                std::thread::sleep(interval);
                if flag.load(Ordering::Relaxed) {
                    return;
                }
                let Some(catalog) = me.upgrade() else { return };
                if catalog.rescan_if_changed() {
                    on_change(&catalog);
                }
            }
        });
        Watch { stop }
    }

    /// The catalog's identity: unique among the catalogs of the process, fixed at construction. Clones of the `Arc`
    /// share it. A holder of derived data keys it by the id and the [`generation`](Catalog::generation), which counts
    /// changes of this catalog only.
    pub fn id(&self) -> u64 {
        self.id
    }

    /// Counts the catalog's changes: each scan, and each face added or removed.
    pub fn generation(&self) -> u64 {
        self.generation.load(Ordering::Acquire)
    }

    /// What changed since `generation` (a value of [`Catalog::generation`]): the families added,
    /// removed and changed, so an app re-resolves only the names those affect and lays out only
    /// the documents that use them. Take the generation before resolving a document's fonts, so a
    /// change during that shows in the next call. `full` when the history no longer reaches back
    /// that far (it keeps the last 64 changes of the family set, and the first scan of a large
    /// system clears it): re-resolve everything then.
    pub fn changes_since(&self, generation: u64) -> Changes {
        let history = lock(&self.history);
        let now = self.generation();
        if generation >= now {
            return Changes { generation: now, ..Changes::default() };
        }
        if generation < history.floor {
            return Changes { generation: now, full: true, ..Changes::default() };
        }
        // Each family's state before the first change and after the last, with its latest name.
        let mut net: HashMap<&str, (bool, bool, &str)> = HashMap::new();
        for (key, name, change) in history.changes.iter().filter(|(g, _)| *g > generation).flat_map(|(_, c)| c) {
            let entry = net.entry(key.as_str()).or_insert((*change != Change::Added, false, name.as_str()));
            entry.1 = *change != Change::Removed;
            entry.2 = name.as_str();
        }
        let mut changes = Changes { generation: now, ..Changes::default() };
        for (before, after, name) in net.into_values() {
            match (before, after) {
                (false, true) => changes.added.push(name.to_owned()),
                (true, false) => changes.removed.push(name.to_owned()),
                (true, true) => changes.changed.push(name.to_owned()),
                (false, false) => {}
            }
        }
        for list in [&mut changes.added, &mut changes.removed, &mut changes.changed] {
            list.sort_by_cached_key(|f| (f.to_lowercase(), f.clone()));
        }
        changes
    }

    fn snapshot(&self) -> Arc<Index> {
        self.ensure();
        Arc::clone(&self.index.read().unwrap_or_else(PoisonError::into_inner))
    }

    fn publish(&self, index: Index) {
        let mut history = lock(&self.history);
        let new = Arc::new(index);
        let old = std::mem::replace(&mut *self.index.write().unwrap_or_else(PoisonError::into_inner), Arc::clone(&new));
        lock(&self.loaded).clear();
        let generation = self.generation.fetch_add(1, Ordering::AcqRel) + 1;
        history.record(generation, &old, &new);
    }

    /// Every face the last scan found, in path order, with the faces [`Catalog::faces`] drops as
    /// duplicates of another of the same family, style and PostScript name. Faces added from
    /// files or bytes aren't in it. For telling apart copies of one font in different places.
    pub fn scanned_faces(&self) -> Vec<FaceInfo> {
        self.ensure();
        self.scanned().iter().map(|f| f.info.clone()).collect()
    }

    /// The family names, sorted ignoring case. Private system families are among them ([`is_private_family`]): a
    /// document may name one, while font lists leave them out.
    pub fn families(&self) -> Arc<[String]> {
        Arc::clone(&self.snapshot().families)
    }

    /// Every face, added faces first.
    pub fn faces(&self) -> Arc<[FaceInfo]> {
        Arc::clone(&self.snapshot().infos)
    }

    /// The family `name` names: its own name ignoring ASCII case, or any other of its names
    /// (another language, a legacy name) ignoring case and everything but letters and digits.
    pub fn family(&self, name: &str) -> Option<String> {
        let index = self.snapshot();
        let i = *index.family_of(name)?.first()?;
        index.face(i).map(|f| f.info.family.clone())
    }

    /// The faces of the family `name` names ([`Catalog::family`]).
    pub fn styles(&self, family: &str) -> Vec<FaceInfo> {
        let index = self.snapshot();
        index.family_of(family).unwrap_or_default().iter().filter_map(|i| index.face(*i)).map(|f| f.info.clone()).collect()
    }

    /// Every cataloged version of `face`: the faces with its family, style, PostScript name and
    /// variations, from every file that has it. The one the catalog lists ([`Catalog::faces`])
    /// comes first, then the others: added faces, then installed files by path. Empty when `face`
    /// is no longer cataloged.
    pub fn versions(&self, face: &FaceInfo) -> Vec<FaceInfo> {
        let index = self.snapshot();
        let same = |f: &FaceInfo| {
            f.path == face.path && f.blob == face.blob && f.index == face.index && f.style == face.style && f.variations == face.variations
        };
        for i in index.by_family.get(&face.family.to_ascii_lowercase()).map(Vec::as_slice).unwrap_or_default() {
            let Some(listed) = index.face(*i) else { continue };
            let others = index.versions.get(i).map(Vec::as_slice).unwrap_or_default();
            if same(&listed.info) || others.iter().any(same) {
                return std::iter::once(&listed.info).chain(others).cloned().collect();
            }
        }
        Vec::new()
    }

    /// The face for `name` in `style`, and how it matched. `name` is a family name (any of its
    /// names, as [`Catalog::family`]), a legacy family name (`Roboto Light`, matched with its
    /// legacy style), a full name or a PostScript name (which name one face; `style` is then
    /// ignored). Within the family, `style` matches by name; otherwise the nearest weight wins,
    /// italic differing counting as 1000, then the nearest width. An empty style is `Regular`.
    /// `None` when nothing has that name.
    pub fn resolve(&self, name: &str, style: &str) -> Option<(FaceInfo, Match)> {
        let index = self.snapshot();
        let (i, matched) = index.resolve(name, style)?;
        Some((index.face(i)?.info.clone(), matched))
    }

    /// The face with this PostScript name (a named instance's too), ignoring case and everything
    /// but letters and digits.
    pub fn by_postscript_name(&self, name: &str) -> Option<FaceInfo> {
        let index = self.snapshot();
        let i = *index.postscript.get(&norm(name))?;
        index.face(i).map(|f| f.info.clone())
    }

    /// The data of `face`, read from its file (shared with other faces of the file still loaded).
    pub fn load(&self, face: &FaceInfo) -> Result<FontData, FontFileError> {
        if let Some(blob) = face.blob {
            let bytes = lock(&self.added).blobs.get(&blob).cloned().ok_or(FontFileError::Gone)?;
            return Ok(FontData { bytes, index: face.index });
        }
        let path = face.path.as_deref().ok_or(FontFileError::Gone)?;
        if crate::suitcase::is_suitcase(path) {
            let font = crate::suitcase::read_fonts(path).into_iter().nth(face.index as usize).ok_or(FontFileError::NotAFont)?;
            return Ok(FontData { bytes: font.into(), index: 0 });
        }
        Ok(FontData { bytes: self.file_bytes(path)?, index: face.index })
    }

    fn file_bytes(&self, path: &Path) -> Result<Arc<[u8]>, FontFileError> {
        let slot = {
            let mut files = lock(&self.files);
            // A slot another thread holds (it reads the file) stays; the others go once their bytes are dropped.
            files.retain(|_, slot| {
                Arc::strong_count(slot) > 1 || slot.try_lock().ok().is_none_or(|held| held.as_ref().is_some_and(|w| w.strong_count() > 0))
            });
            Arc::clone(files.entry(path.to_path_buf()).or_default())
        };
        let mut held = lock(&slot);
        if let Some(bytes) = held.as_ref().and_then(Weak::upgrade) {
            return Ok(bytes);
        }
        let size = std::fs::metadata(path)?.len();
        if size > MAX_LOAD {
            return Err(FontFileError::TooLarge { size, max: MAX_LOAD });
        }
        let bytes: Arc<[u8]> = read_capped(path, MAX_LOAD)?.into();
        *held = Some(Arc::downgrade(&bytes));
        Ok(bytes)
    }

    /// Every face of the family `family` names ([`Catalog::family`]) with its data. The first
    /// call reads the family's files; calls for the same family on other threads wait for it and
    /// share the result, while other families load at the same time. Faces whose files can't be
    /// read are left out. After a rescan, the next call reads the files again.
    pub fn load_family(&self, family: &str) -> Arc<[LoadedFace]> {
        let Some(name) = self.family(family) else { return Arc::from([]) };
        let cell = Arc::clone(lock(&self.loaded).entry(name.to_ascii_lowercase()).or_default());
        Arc::clone(
            cell.get_or_init(|| self.styles(&name).into_iter().filter_map(|info| Some(LoadedFace { data: self.load(&info).ok()?, info })).collect()),
        )
    }

    /// Add the faces of a font file's `bytes` (a font or a collection) for this session, as
    /// `source`. Returns the faces added. Doesn't wait for or start the first scan: the installed
    /// faces join them when a lookup scans.
    pub fn add_bytes(&self, bytes: Vec<u8>, source: Source, limits: &Limits) -> Result<Vec<FaceInfo>, FontFileError> {
        let size = bytes.len() as u64;
        if size > limits.max_file_bytes {
            return Err(FontFileError::TooLarge { size, max: limits.max_file_bytes });
        }
        let mut faces = scan::data_faces(&bytes, limits.max_faces_per_file, None, source);
        if faces.is_empty() {
            return Err(FontFileError::NotAFont);
        }
        let mut added = lock(&self.added);
        if added.faces.len() + faces.len() > limits.max_added_faces {
            return Err(FontFileError::TooManyFaces { max: limits.max_added_faces });
        }
        let blob = added.next_blob;
        added.next_blob += 1;
        added.blobs.insert(blob, bytes.into());
        for f in &mut faces {
            f.info.blob = Some(blob);
        }
        Ok(self.push_added(added, faces))
    }

    /// Add the faces of the font file at `path` for this session, as `source`, read in place.
    /// Returns the faces added. Like [`Catalog::add_bytes`], it doesn't scan.
    pub fn add_file(&self, path: &Path, source: Source, limits: &Limits) -> Result<Vec<FaceInfo>, FontFileError> {
        let size = std::fs::metadata(path)?.len();
        if size > limits.max_file_bytes {
            return Err(FontFileError::TooLarge { size, max: limits.max_file_bytes });
        }
        let mut faces = scan::file_faces(path, limits.max_file_bytes, source);
        let mut fonts: Vec<u32> = faces.iter().map(|f| f.info.index).collect();
        fonts.dedup();
        if let Some(&keep) = fonts.get(limits.max_faces_per_file as usize) {
            faces.retain(|f| f.info.index < keep);
        }
        if faces.is_empty() {
            return Err(FontFileError::NotAFont);
        }
        let added = lock(&self.added);
        if added.faces.len() + faces.len() > limits.max_added_faces {
            return Err(FontFileError::TooManyFaces { max: limits.max_added_faces });
        }
        Ok(self.push_added(added, faces))
    }

    fn push_added(&self, mut added: MutexGuard<'_, Added>, faces: Vec<Face>) -> Vec<FaceInfo> {
        let infos = faces.iter().map(|f| f.info.clone()).collect();
        added.faces.extend(faces);
        let scanned = self.scanned();
        self.publish(Index::build(&added.faces, &scanned));
        infos
    }

    /// Remove every added face of `source` (a closed document's fonts). Returns how many.
    pub fn remove_added(&self, source: Source) -> usize {
        let mut added = lock(&self.added);
        let before = added.faces.len();
        added.faces.retain(|f| f.info.source != source);
        let blobs: HashSet<u64> = added.faces.iter().filter_map(|f| f.info.blob).collect();
        added.blobs.retain(|b, _| blobs.contains(b));
        let removed = before - added.faces.len();
        if removed > 0 {
            let scanned = self.scanned();
            self.publish(Index::build(&added.faces, &scanned));
        }
        removed
    }

    /// The faces the last scan found.
    fn scanned(&self) -> Arc<[Face]> {
        Arc::clone(&lock(&self.scanned))
    }

    fn scan_now(&self, refresh: bool) {
        let mut state = lock(&self.scan);
        let (app_dirs, confined_dirs, listers, service) = {
            let c = self.read_config();
            (c.app_dirs.clone(), c.confined_dirs.clone(), c.listers.clone(), c.font_service)
        };
        let ignore_case = self.env.os == Os::Windows;
        let mut roots: Vec<(PathBuf, Source)> = font_dirs(&self.env).into_iter().map(|d| (d, Source::System)).collect();
        roots.extend(font_store_dir(&self.env).map(|d| (d, Source::Store)));
        roots.extend(app_dirs.iter().map(|d| (d.clone(), Source::App)));

        let mut named: Vec<PathBuf> = Vec::new();
        let mut registered = Vec::new();
        if service {
            named.extend(crate::system::font_files(refresh));
            if self.env.os == Os::Windows {
                registered = crate::registry::registered_font_files();
                named.extend(registered.iter().cloned());
            }
        }
        for lister in &listers {
            named.extend(lister());
        }

        let mut walk = Walk {
            ignore_case,
            confined: Vec::new(),
            seen_dirs: HashSet::new(),
            seen_files: HashSet::new(),
            entries: 0,
            dirs: Vec::new(),
            files: Vec::new(),
        };
        walk.confined = confined_dirs.iter().map(|dir| walk.key(dir)).collect();
        let mut store_dirs = Vec::new();
        for (root, source) in &roots {
            let first = walk.dirs.len();
            walk.dir(root, *source, 0);
            if *source == Source::Store {
                store_dirs.extend(walk.dirs.iter().skip(first).cloned());
            }
        }
        let mut parents = HashSet::new();
        for file in named {
            if file.is_dir() {
                walk.dir(&file, Source::System, 0);
                continue;
            }
            if let Some(parent) = file.parent()
                && parents.insert(parent.to_path_buf())
            {
                walk.dirs.push((parent.to_path_buf(), modified(parent)));
            }
            walk.file(file, Source::System, true);
        }

        let mut files = HashMap::new();
        let mut read = false;
        for (path, source, by_name) in walk.files {
            let Ok(meta) = std::fs::metadata(&path) else { continue };
            let (len, mtime) = (meta.len(), meta.modified().ok());
            let cached = state.files.remove(&path).filter(|c| c.len == len && c.modified == mtime);
            read |= cached.is_none();
            let mut faces = match cached {
                Some(c) => c.faces,
                None if by_name || scan::is_font_file(&path) || crate::suitcase::is_suitcase(&path) => scan::file_faces(&path, MAX_SCAN_READ, source),
                None => continue,
            };
            for f in &mut faces {
                f.info.source = source;
            }
            files.insert(path, Cached { len, modified: mtime, faces });
        }
        // A rescan that read no file and lost none keeps the index and its generation, so the caches keyed on the
        // generation (the picker's family infos and samples) stay.
        let unchanged = !read && state.files.is_empty() && self.generation() > 0;
        if !unchanged {
            let mut by_path: Vec<(&PathBuf, &Cached)> = files.iter().collect();
            by_path.sort_by(|a, b| a.0.cmp(b.0));
            *lock(&self.scanned) = by_path.into_iter().flat_map(|(_, c)| c.faces.iter().cloned()).collect();
        }
        state.files = files;
        drop(state);

        *lock(&self.stamp) = Stamp { app_dirs, dirs: walk.dirs, store_dirs, registered };
        if unchanged {
            return;
        }
        let added = lock(&self.added);
        let scanned = self.scanned();
        self.publish(Index::build(&added.faces, &scanned));
    }
}

/// Stops a [`Catalog::watch`] when dropped.
#[must_use = "dropping a Watch stops it"]
pub struct Watch {
    stop: Arc<AtomicBool>,
}

impl Drop for Watch {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
    }
}

/// The files of one scan: each folder and file once, however links lead back to it.
struct Walk {
    ignore_case: bool,
    /// The keys of the confined folders ([`Catalog::set_confined_dirs`]).
    confined: Vec<PathBuf>,
    seen_dirs: HashSet<PathBuf>,
    seen_files: HashSet<PathBuf>,
    entries: usize,
    dirs: Vec<(PathBuf, Option<SystemTime>)>,
    /// Each file with where it was found and whether a lister named it.
    files: Vec<(PathBuf, Source, bool)>,
}

impl Walk {
    fn key(&self, path: &Path) -> PathBuf {
        let real = std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
        if self.ignore_case { PathBuf::from(real.to_string_lossy().to_lowercase()) } else { real }
    }

    /// Whether the folder or file with `key` is inside a confined folder.
    fn confined(&self, key: &Path) -> bool {
        self.confined.iter().any(|dir| key.starts_with(dir))
    }

    fn dir(&mut self, dir: &Path, source: Source, depth: usize) {
        let key = self.key(dir);
        let confined = self.confined(&key);
        if !self.seen_dirs.insert(key) {
            return;
        }
        // Stamped before it is listed: a font installed meanwhile changes it after this.
        self.dirs.push((dir.to_path_buf(), modified(dir)));
        let Ok(entries) = std::fs::read_dir(dir) else { return };
        let mut paths: Vec<(PathBuf, bool)> = Vec::new();
        for e in entries.flatten() {
            self.entries += 1;
            if self.entries > MAX_ENTRIES {
                break;
            }
            let path = e.path();
            let link = e.file_type().is_ok_and(|t| t.is_symlink());
            if link && confined && !self.confined(&self.key(&path)) {
                continue;
            }
            let is_dir = e.file_type().is_ok_and(|t| t.is_dir() || (t.is_symlink() && path.is_dir()));
            paths.push((path, is_dir));
        }
        paths.sort();
        for (path, is_dir) in paths {
            if is_dir {
                if depth < MAX_DEPTH {
                    self.dir(&path, source, depth + 1);
                }
            } else {
                self.file(path, source, false);
            }
        }
    }

    fn file(&mut self, path: PathBuf, source: Source, by_name: bool) {
        let key = self.key(&path);
        if self.seen_files.insert(key) {
            self.files.push((path, source, by_name));
        }
    }
}

/// Whether `family` is a private system family, which font menus don't list: a name with a leading '.' (macOS's
/// `.SF NS`, `.Aqua Kana`), as Photoshop hides them.
pub fn is_private_family(family: &str) -> bool {
    family.starts_with('.')
}
