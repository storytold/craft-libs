//! The catalog: every installed face, found by any of its names, loaded once.

use std::collections::{BTreeSet, HashMap, HashSet, VecDeque};
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

/// Limits on fonts added from files or bytes ([`Catalog::add_bytes`], [`Catalog::add_file`] and
/// their scoped forms), which may come from documents or downloads.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Limits {
    /// The largest file accepted.
    pub max_file_bytes: u64,
    /// The most faces read from one file (a collection).
    pub max_faces_per_file: u32,
    /// The most added faces one scope holds, and the most added without a scope: each scope and
    /// the unscoped faces count on their own.
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

/// A set of added faces that only lookups in it see: one document's fonts
/// ([`Catalog::new_scope`]). Inside a scope, its own faces come first, then the faces added
/// without a scope, then the installed faces.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Scope(u64);

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

impl Alias {
    fn face(&self) -> usize {
        match self {
            Alias::Family(i) | Alias::Legacy(i, _) | Alias::Face(i) => *i,
        }
    }

    /// The order [`Index::resolve`] tries kinds of alias in.
    fn tier(&self) -> u8 {
        match self {
            Alias::Family(_) => 0,
            Alias::Legacy(..) => 1,
            Alias::Face(_) => 2,
        }
    }
}

/// What tells faces apart: of the faces alike in it, lookups see only the first.
fn identity(f: &Face) -> (String, String, Option<String>, Vec<(u32, u32)>) {
    let variations = f.info.variations.iter().map(|(t, v)| (u32::from_be_bytes(*t), v.to_bits())).collect();
    (f.info.family.to_ascii_lowercase(), norm(&f.info.style), f.keys.postscript.first().cloned(), variations)
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
    /// How many of `faces` (the first ones) came from the added faces.
    added: usize,
    /// A hash of each family's faces by lowercase name, for [`Catalog::changes_since`]; set by
    /// [`Catalog::publish`] only, so scope views don't pay for it.
    signatures: HashMap<String, u64>,
}

impl Index {
    /// Added faces first: a document's or the app's own face wins over an installed face of the
    /// same family and style.
    fn build<'a>(added: &'a [Face], scanned: impl IntoIterator<Item = &'a Face>) -> Self {
        let mut faces: Vec<Face> = Vec::new();
        let mut seen = HashSet::new();
        let mut from_added = 0;
        for (n, f) in added.iter().chain(scanned).enumerate() {
            if seen.insert(identity(f)) {
                faces.push(f.clone());
                if n < added.len() {
                    from_added += 1;
                }
            }
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
        Self { faces, infos, families: families.into(), by_family, aliases, postscript, added: from_added, signatures: HashMap::new() }
    }

    /// What [`Index::signatures`] holds: for each family, a hash of its faces' names, styles,
    /// weights, widths, files and the files' sizes and modification times, so whatever can lay
    /// text out differently changes it.
    fn family_signatures(&self) -> HashMap<String, u64> {
        self.by_family
            .iter()
            .map(|(key, faces)| {
                let mut h = DefaultHasher::new();
                for f in faces.iter().filter_map(|i| self.face(*i)) {
                    let i = &f.info;
                    (&i.family, &i.style, &i.postscript_name, &i.path, i.index, i.blob, i.source, i.italic, f.file).hash(&mut h);
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

/// The families each recent generation added, removed or changed, for unscoped lookups.
#[derive(Default)]
struct History {
    /// Each generation that changed a family, oldest first, with the families (lowercase name,
    /// name) and how they changed.
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

/// The families unscoped lookups find differently since a generation ([`Catalog::changes_since`]):
/// what an app re-resolves to know which documents to lay out again. Each list is sorted
/// ignoring case.
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

fn family_name(index: &Index, name: &str) -> Option<String> {
    let i = *index.family_of(name)?.first()?;
    index.face(i).map(|f| f.info.family.clone())
}

/// The faces of the family `family` names; `None` when no family has that name.
fn styles_of(index: &Index, family: &str) -> Option<Vec<FaceInfo>> {
    Some(index.family_of(family)?.iter().filter_map(|i| index.face(*i)).map(|f| f.info.clone()).collect())
}

fn resolve_face(index: &Index, name: &str, style: &str) -> Option<(FaceInfo, Match)> {
    let (i, matched) = index.resolve(name, style)?;
    Some((index.face(i)?.info.clone(), matched))
}

fn postscript_face(index: &Index, name: &str) -> Option<FaceInfo> {
    let i = *index.postscript.get(&norm(name))?;
    index.face(i).map(|f| f.info.clone())
}

/// What the catalog reads besides the platform's font folders.
struct Config {
    app_dirs: Vec<PathBuf>,
    confined_dirs: Vec<PathBuf>,
    listers: Vec<Lister>,
    font_service: bool,
}

/// A file's size and modification time: when either differs, the file changed.
type FileStamp = (u64, Option<SystemTime>);

fn file_stamp(meta: &std::fs::Metadata) -> FileStamp {
    (meta.len(), meta.modified().ok())
}

/// A loaded file's bytes, while anything holds them.
struct Loaded {
    /// The file's stamp when read: a file replaced since is read again.
    stamp: FileStamp,
    /// Read without following a link in the file's place.
    no_follow: bool,
    bytes: Weak<[u8]>,
}

/// A file found by a scan, remembered so a rescan reads only files that changed.
struct Cached {
    stamp: FileStamp,
    /// Shared with [`Added::scanned`].
    faces: Arc<Vec<Face>>,
}

#[derive(Default)]
struct ScanState {
    files: HashMap<PathBuf, Cached>,
}

/// What a change check compares with.
#[derive(Default)]
struct Stamp {
    app_dirs: Vec<PathBuf>,
    confined_dirs: Vec<PathBuf>,
    /// Every folder the scan listed, and the folders holding the files listers named, with their
    /// modification times (`None`: missing).
    dirs: Vec<(PathBuf, Option<SystemTime>)>,
    /// The fonts registered with Windows.
    registered: Vec<PathBuf>,
}

#[derive(Default)]
struct Added {
    /// Faces added without a scope.
    faces: Vec<Face>,
    /// The faces the last scan found, by file in path order; `None` before the first scan, which
    /// then builds the index with the faces added before it. Kept here, not read from the scan's
    /// state, so adding faces never waits for a scan.
    scanned: Option<Vec<Arc<Vec<Face>>>>,
    scopes: HashMap<Scope, Scoped>,
    /// The bytes of faces added as bytes, with or without a scope.
    blobs: HashMap<u64, Arc<[u8]>>,
    next_blob: u64,
    /// Counts every scope's changes together, so a scope's stamp never repeats.
    scope_changes: u64,
}

impl Added {
    /// Record a change of `scope`'s faces: a new stamp, and a view to build again.
    fn changed(&mut self, scope: Scope) -> &mut Scoped {
        self.scope_changes += 1;
        let stamp = self.scope_changes;
        let scoped = self.scopes.entry(scope).or_default();
        scoped.stamp = stamp;
        scoped.changes += 1;
        scoped.view = None;
        scoped
    }

    /// Whether `scope` (or the unscoped faces) has room for `more` faces within `limits`.
    fn room(&self, scope: Option<Scope>, more: usize, limits: &Limits) -> Result<(), FontFileError> {
        let held = match scope {
            None => self.faces.len(),
            Some(scope) => self.scopes.get(&scope).map_or(0, |s| s.faces.len()),
        };
        if held + more > limits.max_added_faces {
            return Err(FontFileError::TooManyFaces { max: limits.max_added_faces });
        }
        Ok(())
    }

    /// Keep `bytes` for `faces` when `pending` more faces about to be added to `scope` leave room
    /// within `limits`; returns the faces, pointing at the bytes.
    fn store(
        &mut self,
        scope: Option<Scope>,
        bytes: Arc<[u8]>,
        mut faces: Vec<Face>,
        pending: usize,
        limits: &Limits,
    ) -> Result<Vec<Face>, FontFileError> {
        self.room(scope, pending + faces.len(), limits)?;
        let blob = self.next_blob;
        self.next_blob += 1;
        self.blobs.insert(blob, bytes);
        for f in &mut faces {
            f.info.blob = Some(blob);
        }
        Ok(faces)
    }

    /// Drop the bytes no added face uses any more.
    fn drop_unused_blobs(&mut self) {
        let used: HashSet<u64> = self.faces.iter().chain(self.scopes.values().flat_map(|s| &s.faces)).filter_map(|f| f.info.blob).collect();
        self.blobs.retain(|b, _| used.contains(b));
    }
}

/// A scope's faces, and the view its lookups read. A removed scope keeps its entry, with no
/// faces, so its stamp never goes back.
#[derive(Default)]
struct Scoped {
    faces: Vec<Face>,
    /// [`Added::scope_changes`] at the scope's last change, for [`Catalog::generation_in`].
    stamp: u64,
    /// The scope's own changes, for [`Catalog::scope_generation`].
    changes: u64,
    /// Built on the first lookup after a change of the scope or of the shared index.
    view: Option<Arc<View>>,
}

/// What lookups in a scope read: an index of the scope's faces and the shared faces of their
/// families, consulted before the shared index. [`View::answers`] says which of the two a name
/// resolves in, so the result is what one index of every face would give.
struct View {
    /// The shared index this view was built from.
    base: Arc<Index>,
    index: Index,
    /// The shared families and the scope's, sorted ignoring case.
    families: Arc<[String]>,
}

impl View {
    fn build(own: &[Face], base: Arc<Index>) -> Self {
        let keys: BTreeSet<String> = own.iter().map(|f| f.info.family.to_ascii_lowercase()).collect();
        let shared: Vec<Face> = keys.iter().filter_map(|k| base.by_family.get(k)).flatten().filter_map(|i| base.face(*i)).cloned().collect();
        let index = Index::build(own, &shared);
        let mut families: Vec<String> = base.families.to_vec();
        let before = families.len();
        families.extend(index.families.iter().filter(|f| !base.by_family.contains_key(&f.to_ascii_lowercase())).cloned());
        if families.len() > before {
            families.sort_by_cached_key(|f| (f.to_lowercase(), f.clone()));
        }
        Self { base, index, families: families.into() }
    }

    /// Whether `name` resolves to the scope's faces, trying names in [`Index::resolve`]'s order:
    /// an exact family name, then each kind of alias. A family the view holds has scope faces;
    /// otherwise the first kind of alias either index has decides, and the view's aliases list
    /// the scope's faces first.
    fn answers(&self, name: &str) -> bool {
        let lower = name.to_ascii_lowercase();
        if self.index.by_family.contains_key(&lower) {
            return true;
        }
        if self.base.by_family.contains_key(&lower) {
            return false;
        }
        let key = norm(name);
        let (own, shared) = (self.index.aliases.get(&key), self.base.aliases.get(&key));
        for tier in 0..3 {
            if let Some(a) = own.into_iter().flatten().find(|a| a.tier() == tier) {
                return a.face() < self.index.added;
            }
            if shared.into_iter().flatten().any(|a| a.tier() == tier) {
                return false;
            }
        }
        false
    }

    /// The scope's face with PostScript name `name`.
    fn postscript(&self, name: &str) -> Option<usize> {
        self.index.postscript.get(&norm(name)).copied().filter(|i| *i < self.index.added)
    }
}

/// The faces of a font file's `bytes`, within `limits`, before they are added.
fn bytes_faces(bytes: Arc<[u8]>, source: Source, limits: &Limits) -> Result<(Arc<[u8]>, Vec<Face>), FontFileError> {
    let size = bytes.len() as u64;
    if size > limits.max_file_bytes {
        return Err(FontFileError::TooLarge { size, max: limits.max_file_bytes });
    }
    let faces = scan::data_faces(&bytes, limits.max_faces_per_file, None, source);
    if faces.is_empty() {
        return Err(FontFileError::NotAFont);
    }
    Ok((bytes, faces))
}

/// A family's faces, loaded once ([`Catalog::load_family`]).
type FamilyLoad = Arc<OnceLock<Arc<[LoadedFace]>>>;

/// Why a path isn't read as a font: it is a folder, a FIFO, a device or a socket, which could
/// block or never end.
fn not_a_regular_file() -> FontFileError {
    FontFileError::Io(scan::not_a_regular_file())
}

/// The installed fonts: the platform's font folders, the files its font service knows, the font
/// store and the app's own folders, plus faces added from files or bytes.
///
/// The first scan runs on first use or on a background thread ([`Catalog::scan_in_background`]);
/// lookups wait for it. Lookups read an immutable snapshot, so they never wait for a rescan.
/// Each family's files load once ([`Catalog::load_family`]): other threads asking for the same
/// family wait for that load, other families load at the same time.
pub struct Catalog {
    env: FontEnv,
    config: RwLock<Config>,
    index: RwLock<Arc<Index>>,
    ready: OnceLock<()>,
    scan: Mutex<ScanState>,
    stamp: Mutex<Stamp>,
    generation: AtomicU64,
    /// Locked while [`Catalog::publish`] moves the generation on, so it and the history agree.
    history: Mutex<History>,
    added: Mutex<Added>,
    next_scope: AtomicU64,
    /// Loaded families by scope (`None`: unscoped) and lowercase family name.
    loaded: Mutex<HashMap<(Option<Scope>, String), FamilyLoad>>,
    files: Mutex<HashMap<PathBuf, Loaded>>,
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

fn modified(path: &Path) -> Option<SystemTime> {
    std::fs::metadata(path).and_then(|m| m.modified()).ok()
}

impl Catalog {
    /// A catalog of the fonts installed on the system `env` describes, the font store, and the
    /// files the platform's font service lists (with the `system` feature).
    pub fn new(env: FontEnv) -> Self {
        Self {
            env,
            config: RwLock::new(Config {
                app_dirs: Vec::new(),
                confined_dirs: Vec::new(),
                listers: Vec::new(),
                font_service: cfg!(feature = "system"),
            }),
            index: RwLock::new(Arc::new(Index::default())),
            ready: OnceLock::new(),
            scan: Mutex::new(ScanState::default()),
            stamp: Mutex::new(Stamp::default()),
            generation: AtomicU64::new(0),
            history: Mutex::new(History::default()),
            added: Mutex::new(Added::default()),
            next_scope: AtomicU64::new(0),
            loaded: Mutex::new(HashMap::new()),
            files: Mutex::new(HashMap::new()),
        }
    }

    /// A catalog of no installed fonts: no font folders, no font store and no font service. It
    /// finds only the faces added to it ([`Catalog::add_bytes`]) and the folders given to
    /// [`Catalog::set_app_dirs`] and [`Catalog::set_confined_dirs`]. For a CLI's "no system
    /// fonts" switch ([`Catalog::init_global`]) and for tests that must not see the host's fonts.
    pub fn without_installed_fonts() -> Self {
        Self::new(FontEnv { os: Os::None, ..FontEnv::default() }).with_font_service(false)
    }

    /// The process's catalog: the one [`Catalog::init_global`] set, else one for
    /// [`FontEnv::from_process`].
    pub fn global() -> &'static Arc<Catalog> {
        GLOBAL.get_or_init(|| Arc::new(Catalog::new(FontEnv::from_process())))
    }

    /// Make `catalog` the process's catalog ([`Catalog::global`]), for example
    /// [`Catalog::without_installed_fonts`] for a CLI's "no system fonts" switch. Call it before
    /// anything uses the global catalog; once something has, the catalog comes back as the error.
    pub fn init_global(catalog: Catalog) -> Result<&'static Arc<Catalog>, Box<Catalog>> {
        let mut catalog = Some(catalog);
        let global = GLOBAL.get_or_init(|| Arc::new(catalog.take().unwrap_or_else(|| Catalog::new(FontEnv::from_process()))));
        match catalog {
            Some(catalog) => Err(Box::new(catalog)),
            None => Ok(global),
        }
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

    /// Folders the user or an automation caller picked, read with their subfolders from the next
    /// scan on, as [`Source::User`]. The scan stays inside each: a link whose target is outside
    /// the folder is skipped. A change shows in [`Catalog::installed_fonts_changed`].
    pub fn set_confined_dirs(&self, dirs: Vec<PathBuf>) {
        self.write_config().confined_dirs = dirs;
    }

    fn write_config(&self) -> std::sync::RwLockWriteGuard<'_, Config> {
        self.config.write().unwrap_or_else(PoisonError::into_inner)
    }

    fn read_config(&self) -> std::sync::RwLockReadGuard<'_, Config> {
        self.config.read().unwrap_or_else(PoisonError::into_inner)
    }

    /// The environment the catalog was made for.
    pub fn env(&self) -> &FontEnv {
        &self.env
    }

    /// Start the first scan on a background thread, so the first lookup doesn't wait for all of
    /// it. Does nothing once it has run, or when no thread can be started (the first lookup
    /// then scans).
    pub fn scan_in_background(self: &Arc<Self>) {
        if self.ready.get().is_none() {
            let me = Arc::clone(self);
            let _ = std::thread::Builder::new().name("font-catalog-scan".into()).spawn(move || me.ensure());
        }
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
    /// number of faces cataloged.
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
        {
            let config = self.read_config();
            if stamp.app_dirs != config.app_dirs || stamp.confined_dirs != config.confined_dirs {
                return true;
            }
        }
        if stamp.dirs.iter().any(|(p, t)| modified(p) != *t) {
            return true;
        }
        self.read_config().font_service && self.env.os == Os::Windows && stamp.registered != crate::registry::registered_font_files()
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

    /// Counts the changes unscoped lookups see: each scan, and each face added or removed
    /// without a scope. Faces added to or removed from a scope don't change it, so other
    /// documents don't lay out again; [`Catalog::generation_in`] counts those too.
    pub fn generation(&self) -> u64 {
        self.generation.load(Ordering::Acquire)
    }

    /// Counts the changes lookups in `scope` see: [`Catalog::generation`] plus a stamp of the
    /// scope's last change, taken from one counter of every scope's changes. It never goes down
    /// and never repeats a value, [`Catalog::remove_scope`] and adding to the scope again
    /// included. Read it before resolving or laying out, as for [`Catalog::changes_since`]: a
    /// change that lands in between then shows as a newer value next time, instead of the old
    /// layout being kept under it.
    pub fn generation_in(&self, scope: Scope) -> u64 {
        let own = lock(&self.added).scopes.get(&scope).map_or(0, |s| s.stamp);
        self.generation() + own
    }

    /// Counts the changes of `scope`'s own faces: each add to it and each removal from it, and
    /// nothing else (no scan, no unscoped change, no other scope's). 0 for a scope nothing was
    /// added to. It never goes down, [`Catalog::remove_scope`] included. Read it before resolving
    /// or laying out, as [`Catalog::generation_in`] says.
    pub fn scope_generation(&self, scope: Scope) -> u64 {
        lock(&self.added).scopes.get(&scope).map_or(0, |s| s.changes)
    }

    fn snapshot(&self) -> Arc<Index> {
        self.ensure();
        Arc::clone(&self.index.read().unwrap_or_else(PoisonError::into_inner))
    }

    /// What changed for unscoped lookups since `generation` (a value of
    /// [`Catalog::generation`]): the families added, removed and changed, so an app re-resolves
    /// only the names those affect and lays out only the documents that use them. Take the
    /// generation before resolving a document's fonts, so a change during that shows in the next
    /// call. `full` when the history no longer reaches back that far (it keeps the last 64
    /// changes of the family set, and the first scan of a large system clears it): re-resolve
    /// everything then. Scopes have their own [`Catalog::generation_in`].
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

    fn publish(&self, mut index: Index) {
        index.signatures = index.family_signatures();
        let mut history = lock(&self.history);
        let new = Arc::new(index);
        let old = std::mem::replace(&mut *self.index.write().unwrap_or_else(PoisonError::into_inner), Arc::clone(&new));
        lock(&self.loaded).clear();
        let generation = self.generation.fetch_add(1, Ordering::AcqRel) + 1;
        history.record(generation, &old, &new);
    }

    /// The shared index, and the view of `scope` when the scope has faces.
    fn lookup(&self, scope: Scope) -> (Arc<Index>, Option<Arc<View>>) {
        let base = self.snapshot();
        let mut added = lock(&self.added);
        let Some(scoped) = added.scopes.get_mut(&scope).filter(|s| !s.faces.is_empty()) else { return (base, None) };
        if let Some(view) = &scoped.view
            && Arc::ptr_eq(&view.base, &base)
        {
            return (base, Some(Arc::clone(view)));
        }
        let view = Arc::new(View::build(&scoped.faces, Arc::clone(&base)));
        scoped.view = Some(Arc::clone(&view));
        (base, Some(view))
    }

    /// The family names, sorted ignoring case.
    pub fn families(&self) -> Arc<[String]> {
        Arc::clone(&self.snapshot().families)
    }

    /// The family names in `scope` ([`Catalog::families`] plus the scope's), sorted ignoring case.
    pub fn families_in(&self, scope: Scope) -> Arc<[String]> {
        match self.lookup(scope) {
            (_, Some(view)) => Arc::clone(&view.families),
            (base, None) => Arc::clone(&base.families),
        }
    }

    /// Every face added without a scope or installed, added faces first.
    pub fn faces(&self) -> Arc<[FaceInfo]> {
        Arc::clone(&self.snapshot().infos)
    }

    /// The faces added to `scope`, in the order they were added; not the unscoped or installed
    /// faces. Of the faces with the same family, style, PostScript name and instance, the first
    /// added is listed, as lookups in the scope see it. It never waits for a scan.
    pub fn faces_in(&self, scope: Scope) -> Vec<FaceInfo> {
        let added = lock(&self.added);
        let mut seen = HashSet::new();
        added.scopes.get(&scope).map_or_else(Vec::new, |s| s.faces.iter().filter(|f| seen.insert(identity(f))).map(|f| f.info.clone()).collect())
    }

    /// The family `name` names: its own name ignoring ASCII case, or any other of its names
    /// (another language, a legacy name) ignoring case and everything but letters and digits.
    pub fn family(&self, name: &str) -> Option<String> {
        family_name(&self.snapshot(), name)
    }

    /// [`Catalog::family`] in `scope`.
    pub fn family_in(&self, scope: Scope, name: &str) -> Option<String> {
        let (base, view) = self.lookup(scope);
        view.filter(|v| v.answers(name)).and_then(|v| family_name(&v.index, name)).or_else(|| family_name(&base, name))
    }

    /// The faces of the family `name` names ([`Catalog::family`]).
    pub fn styles(&self, family: &str) -> Vec<FaceInfo> {
        styles_of(&self.snapshot(), family).unwrap_or_default()
    }

    /// [`Catalog::styles`] in `scope`: for a family the scope has faces of, the scope's faces and
    /// the shared faces of other styles.
    pub fn styles_in(&self, scope: Scope, family: &str) -> Vec<FaceInfo> {
        let (base, view) = self.lookup(scope);
        view.filter(|v| v.answers(family)).and_then(|v| styles_of(&v.index, family)).or_else(|| styles_of(&base, family)).unwrap_or_default()
    }

    /// The face for `name` in `style`, and how it matched. `name` is a family name (any of its
    /// names, as [`Catalog::family`]), a legacy family name (`Roboto Light`, matched with its
    /// legacy style), a full name or a PostScript name (which name one face; `style` is then
    /// ignored). Within the family, `style` matches by name; otherwise the nearest weight wins,
    /// italic differing counting as 1000, then the nearest width. An empty style is `Regular`.
    /// `None` when nothing has that name.
    pub fn resolve(&self, name: &str, style: &str) -> Option<(FaceInfo, Match)> {
        resolve_face(&self.snapshot(), name, style)
    }

    /// [`Catalog::resolve`] in `scope`: a name the scope's faces answer to resolves among them
    /// and the shared faces of their families, any other name as [`Catalog::resolve`].
    pub fn resolve_in(&self, scope: Scope, name: &str, style: &str) -> Option<(FaceInfo, Match)> {
        let (base, view) = self.lookup(scope);
        view.filter(|v| v.answers(name)).and_then(|v| resolve_face(&v.index, name, style)).or_else(|| resolve_face(&base, name, style))
    }

    /// The face with this PostScript name (a named instance's too), ignoring case and everything
    /// but letters and digits.
    pub fn by_postscript_name(&self, name: &str) -> Option<FaceInfo> {
        postscript_face(&self.snapshot(), name)
    }

    /// [`Catalog::by_postscript_name`] in `scope`, the scope's faces first.
    pub fn by_postscript_name_in(&self, scope: Scope, name: &str) -> Option<FaceInfo> {
        let (base, view) = self.lookup(scope);
        match view.and_then(|v| Some((v.postscript(name)?, v))) {
            Some((i, v)) => v.index.face(i).map(|f| f.info.clone()),
            None => postscript_face(&base, name),
        }
    }

    /// The data of `face`, read from its file (shared with other faces of the file still loaded).
    pub fn load(&self, face: &FaceInfo) -> Result<FontData, FontFileError> {
        if let Some(blob) = face.blob {
            let bytes = lock(&self.added).blobs.get(&blob).cloned().ok_or(FontFileError::Gone)?;
            return Ok(FontData { bytes, index: face.index });
        }
        let path = face.path.as_deref().ok_or(FontFileError::Gone)?;
        if crate::suitcase::is_suitcase(path) {
            // The scan gives a confined folder's suitcases no faces ([`scan::opened_file_faces`]).
            if face.confined {
                return Err(FontFileError::NotAFont);
            }
            let font = crate::suitcase::read_fonts(path).into_iter().nth(face.index as usize).ok_or(FontFileError::NotAFont)?;
            return Ok(FontData { bytes: font.into(), index: 0 });
        }
        Ok(FontData { bytes: self.file_bytes(path, face.confined)?, index: face.index })
    }

    /// The bytes of the file at `path`, shared while anything holds them. With `no_follow`
    /// (a confined file), a link in its place is refused, and so are bytes cached by a read that
    /// followed links.
    fn file_bytes(&self, path: &Path, no_follow: bool) -> Result<Arc<[u8]>, FontFileError> {
        // A file replaced in place since it was read is read again.
        let meta = if no_follow { std::fs::symlink_metadata(path) } else { std::fs::metadata(path) };
        if let Some(meta) = meta.ok().filter(std::fs::Metadata::is_file)
            && let Some(bytes) =
                lock(&self.files).get(path).filter(|c| c.stamp == file_stamp(&meta) && (c.no_follow || !no_follow)).and_then(|c| c.bytes.upgrade())
        {
            return Ok(bytes);
        }
        use std::io::Read;
        let file = scan::open_regular(path, no_follow)?;
        let meta = file.metadata()?;
        let size = meta.len();
        if size > MAX_LOAD {
            return Err(FontFileError::TooLarge { size, max: MAX_LOAD });
        }
        let mut data = Vec::new();
        file.take(MAX_LOAD).read_to_end(&mut data)?;
        let bytes: Arc<[u8]> = data.into();
        let mut files = lock(&self.files);
        files.retain(|_, c| c.bytes.strong_count() > 0);
        files.insert(path.to_path_buf(), Loaded { stamp: file_stamp(&meta), no_follow, bytes: Arc::downgrade(&bytes) });
        Ok(bytes)
    }

    /// Every face of the family `family` names ([`Catalog::family`]) with its data. The first
    /// call reads the family's files; calls for the same family on other threads wait for it and
    /// share the result, while other families load at the same time. Faces whose files can't be
    /// read are left out. After a rescan, the next call reads the files again.
    pub fn load_family(&self, family: &str) -> Arc<[LoadedFace]> {
        let Some(name) = self.family(family) else { return Arc::from([]) };
        let cell = Arc::clone(lock(&self.loaded).entry((None, name.to_ascii_lowercase())).or_default());
        Arc::clone(
            cell.get_or_init(|| self.styles(&name).into_iter().filter_map(|info| Some(LoadedFace { data: self.load(&info).ok()?, info })).collect()),
        )
    }

    /// [`Catalog::load_family`] in `scope`. A family the scope has no faces of is shared with
    /// unscoped loads.
    pub fn load_family_in(&self, scope: Scope, family: &str) -> Arc<[LoadedFace]> {
        let (_, view) = self.lookup(scope);
        let Some(name) = view.filter(|v| v.answers(family)).and_then(|v| family_name(&v.index, family)) else { return self.load_family(family) };
        let cell = Arc::clone(lock(&self.loaded).entry((Some(scope), name.to_ascii_lowercase())).or_default());
        Arc::clone(cell.get_or_init(|| {
            self.styles_in(scope, &name).into_iter().filter_map(|info| Some(LoadedFace { data: self.load(&info).ok()?, info })).collect()
        }))
    }

    /// A new, empty scope, for one document's fonts. Faces added to it with
    /// [`Catalog::add_bytes_in`] and [`Catalog::add_file_in`] are seen only by lookups in it
    /// ([`Catalog::resolve_in`] and the other `_in` lookups) until [`Catalog::remove_scope`].
    pub fn new_scope(&self) -> Scope {
        Scope(self.next_scope.fetch_add(1, Ordering::Relaxed))
    }

    /// Add the faces of a font file's `bytes` (a font or a collection) for this session, as
    /// `source`, seen by every lookup. Returns the faces added. It never waits for a scan: faces
    /// added before the first scan join it. The catalog keeps `bytes` as an `Arc<[u8]>`; pass
    /// one to share it without a copy.
    pub fn add_bytes(&self, bytes: impl Into<Arc<[u8]>>, source: Source, limits: &Limits) -> Result<Vec<FaceInfo>, FontFileError> {
        self.add_bytes_to(None, bytes.into(), source, limits)
    }

    /// [`Catalog::add_bytes`] for several fonts, with one rebuild of the lookups for all of them
    /// (an app's bundled fonts). Returns each font's faces or why it wasn't added, in order.
    pub fn add_bytes_many<B: Into<Arc<[u8]>>>(
        &self,
        fonts: impl IntoIterator<Item = B>,
        source: Source,
        limits: &Limits,
    ) -> Vec<Result<Vec<FaceInfo>, FontFileError>> {
        let parsed: Vec<_> = fonts.into_iter().map(|b| bytes_faces(b.into(), source, limits)).collect();
        let mut added = lock(&self.added);
        let mut all = Vec::new();
        let results = parsed
            .into_iter()
            .map(|p| {
                let (bytes, faces) = p?;
                let faces = added.store(None, bytes, faces, all.len(), limits)?;
                let infos = faces.iter().map(|f| f.info.clone()).collect();
                all.extend(faces);
                Ok(infos)
            })
            .collect();
        if !all.is_empty() {
            self.push_added(added, None, all);
        }
        results
    }

    /// [`Catalog::add_bytes`] into `scope`: only lookups in the scope see the faces.
    pub fn add_bytes_in(&self, scope: Scope, bytes: impl Into<Arc<[u8]>>, source: Source, limits: &Limits) -> Result<Vec<FaceInfo>, FontFileError> {
        self.add_bytes_to(Some(scope), bytes.into(), source, limits)
    }

    fn add_bytes_to(&self, scope: Option<Scope>, bytes: Arc<[u8]>, source: Source, limits: &Limits) -> Result<Vec<FaceInfo>, FontFileError> {
        let (bytes, faces) = bytes_faces(bytes, source, limits)?;
        let mut added = lock(&self.added);
        let faces = added.store(scope, bytes, faces, 0, limits)?;
        Ok(self.push_added(added, scope, faces))
    }

    /// Add the faces of the font file at `path` for this session, as `source`, read in place and
    /// seen by every lookup. Returns the faces added. A path that isn't a regular file (after
    /// following links) is refused unopened.
    pub fn add_file(&self, path: &Path, source: Source, limits: &Limits) -> Result<Vec<FaceInfo>, FontFileError> {
        self.add_file_to(None, path, source, limits)
    }

    /// [`Catalog::add_file`] into `scope`: only lookups in the scope see the faces.
    pub fn add_file_in(&self, scope: Scope, path: &Path, source: Source, limits: &Limits) -> Result<Vec<FaceInfo>, FontFileError> {
        self.add_file_to(Some(scope), path, source, limits)
    }

    fn add_file_to(&self, scope: Option<Scope>, path: &Path, source: Source, limits: &Limits) -> Result<Vec<FaceInfo>, FontFileError> {
        let meta = std::fs::metadata(path)?;
        if !meta.is_file() {
            return Err(not_a_regular_file());
        }
        let size = meta.len();
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
        added.room(scope, faces.len(), limits)?;
        Ok(self.push_added(added, scope, faces))
    }

    /// Unscoped faces join the shared index (a new generation); a scope's faces only its view.
    fn push_added(&self, mut added: MutexGuard<'_, Added>, scope: Option<Scope>, faces: Vec<Face>) -> Vec<FaceInfo> {
        let infos = faces.iter().map(|f| f.info.clone()).collect();
        match scope {
            None => {
                added.faces.extend(faces);
                self.rebuild(&added);
            }
            Some(scope) => {
                added.changed(scope).faces.extend(faces);
                // Under the lock, as `publish` does: no load reads the old faces after the stamp.
                lock(&self.loaded).retain(|(s, _), _| *s != Some(scope));
            }
        }
        infos
    }

    /// Remove every face of `source` added without a scope (a closed document's fonts). Returns
    /// how many. Scopes keep theirs ([`Catalog::remove_scope`]).
    pub fn remove_added(&self, source: Source) -> usize {
        let mut added = lock(&self.added);
        let before = added.faces.len();
        added.faces.retain(|f| f.info.source != source);
        added.drop_unused_blobs();
        let removed = before - added.faces.len();
        if removed > 0 {
            self.rebuild(&added);
        }
        removed
    }

    /// Remove `scope`'s faces (its document closed). Returns how many. Other scopes and unscoped
    /// lookups don't change, nor does [`Catalog::generation`]. The scope keeps a few bytes, so
    /// [`Catalog::generation_in`] never repeats a value.
    pub fn remove_scope(&self, scope: Scope) -> usize {
        let mut added = lock(&self.added);
        let removed = added.scopes.get(&scope).map_or(0, |s| s.faces.len());
        if removed == 0 {
            return 0;
        }
        added.changed(scope).faces.clear();
        added.drop_unused_blobs();
        lock(&self.loaded).retain(|(s, _), _| *s != Some(scope));
        removed
    }

    /// Remove the faces [`Catalog::add_file_in`] added to `scope` from the file at `path` (the path
    /// as it was passed; the document deleted or replaced one of its font files). Returns how
    /// many. A change of the scope's faces, as an add is, when there were any.
    pub fn remove_file_in(&self, scope: Scope, path: &Path) -> usize {
        let mut added = lock(&self.added);
        let from_file = |f: &Face| f.info.blob.is_none() && f.info.path.as_deref() == Some(path);
        let removed = added.scopes.get(&scope).map_or(0, |s| s.faces.iter().filter(|f| from_file(f)).count());
        if removed == 0 {
            return 0;
        }
        added.changed(scope).faces.retain(|f| !from_file(f));
        lock(&self.loaded).retain(|(s, _), _| *s != Some(scope));
        removed
    }

    /// Publish the added faces with the last scan's; before the first scan, which does that,
    /// nothing.
    fn rebuild(&self, added: &Added) {
        if let Some(scanned) = &added.scanned {
            self.publish(Index::build(&added.faces, scanned.iter().flat_map(|f| f.iter())));
        }
    }

    fn scan_now(&self, refresh: bool) {
        let mut state = lock(&self.scan);
        let (app_dirs, confined_dirs, listers, service) = {
            let c = self.read_config();
            (c.app_dirs.clone(), c.confined_dirs.clone(), c.listers.clone(), c.font_service)
        };
        let ignore_case = self.env.os == Os::Windows;
        // Each folder with its source and whether the walk stays inside it.
        let mut roots: Vec<(PathBuf, Source, bool)> = font_dirs(&self.env).into_iter().map(|d| (d, Source::System, false)).collect();
        roots.extend(font_store_dir(&self.env).map(|d| (d, Source::Store, false)));
        roots.extend(app_dirs.iter().map(|d| (d.clone(), Source::App, false)));
        roots.extend(confined_dirs.iter().map(|d| (d.clone(), Source::User, true)));

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

        let mut walk = Walk { ignore_case, seen_dirs: HashSet::new(), seen_files: HashSet::new(), entries: 0, dirs: Vec::new(), files: Vec::new() };
        for (root, source, confined) in &roots {
            let inside = confined.then(|| walk.key(root));
            walk.dir(root, *source, 0, inside.as_deref());
        }
        let mut parents = HashSet::new();
        for file in named {
            if file.is_dir() {
                walk.dir(&file, Source::System, 0, None);
                continue;
            }
            if let Some(parent) = file.parent()
                && parents.insert(parent.to_path_buf())
            {
                walk.dirs.push((parent.to_path_buf(), modified(parent)));
            }
            walk.file(file, Source::System, true, None);
        }

        let mut files = HashMap::new();
        for (path, source, by_name, real) in walk.files {
            let Ok(meta) = std::fs::metadata(&path) else { continue };
            // A FIFO or device named like a font would block the scan or never end.
            if !meta.is_file() {
                continue;
            }
            let stamp = file_stamp(&meta);
            let cached = state.files.remove(&path).filter(|c| c.stamp == stamp);
            let mut faces = match cached {
                Some(c) => c.faces,
                None if by_name || scan::is_font_file(&path) || crate::suitcase::is_suitcase(&path) => {
                    Arc::new(scan::opened_file_faces(&path, MAX_SCAN_READ, source, real))
                }
                None => continue,
            };
            if faces.iter().any(|f| f.info.source != source || f.info.confined != real || f.file != Some(stamp)) {
                for f in Arc::make_mut(&mut faces) {
                    f.info.source = source;
                    f.info.confined = real;
                    f.file = Some(stamp);
                }
            }
            files.insert(path, Cached { stamp, faces });
        }
        let mut scanned: Vec<(&PathBuf, &Cached)> = files.iter().collect();
        scanned.sort_by(|a, b| a.0.cmp(b.0));
        let scanned: Vec<Arc<Vec<Face>>> = scanned.into_iter().map(|(_, c)| Arc::clone(&c.faces)).collect();
        state.files = files;
        drop(state);

        *lock(&self.stamp) = Stamp { app_dirs, confined_dirs, dirs: walk.dirs, registered };
        let mut added = lock(&self.added);
        added.scanned = Some(scanned);
        self.rebuild(&added);
    }
}

static GLOBAL: OnceLock<Arc<Catalog>> = OnceLock::new();

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
    seen_dirs: HashSet<PathBuf>,
    seen_files: HashSet<PathBuf>,
    entries: usize,
    dirs: Vec<(PathBuf, Option<SystemTime>)>,
    /// Each file with where it was found, whether a lister named it, and whether it is a
    /// confined folder's file recorded by its real path (opened without following a link).
    files: Vec<(PathBuf, Source, bool, bool)>,
}

impl Walk {
    fn key(&self, path: &Path) -> PathBuf {
        let real = std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
        if self.ignore_case { PathBuf::from(real.to_string_lossy().to_lowercase()) } else { real }
    }

    /// Walk `dir` and its subfolders. With `inside` (a confined folder's [`Walk::key`]), folders
    /// and files whose real path is outside it are skipped.
    fn dir(&mut self, dir: &Path, source: Source, depth: usize, inside: Option<&Path>) {
        let key = self.key(dir);
        if inside.is_some_and(|root| !key.starts_with(root)) || !self.seen_dirs.insert(key) {
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
            let is_dir = e.file_type().is_ok_and(|t| t.is_dir() || (t.is_symlink() && path.is_dir()));
            paths.push((path, is_dir));
        }
        paths.sort();
        for (path, is_dir) in paths {
            if is_dir {
                if depth < MAX_DEPTH {
                    self.dir(&path, source, depth + 1, inside);
                }
            } else {
                self.file(path, source, false, inside);
            }
        }
    }

    /// Record `path`. In a confined folder its real path is recorded, so a link swapped in after
    /// this check can't lead the scan outside the folder.
    fn file(&mut self, path: PathBuf, source: Source, by_name: bool, inside: Option<&Path>) {
        let key = self.key(&path);
        if inside.is_some_and(|root| !key.starts_with(root)) || !self.seen_files.insert(key) {
            return;
        }
        let (path, real) = match inside {
            // On Windows the real path is a `\\?\` path, which apps would show.
            Some(_) if cfg!(unix) => match std::fs::canonicalize(&path) {
                Ok(real) => (real, true),
                Err(_) => return,
            },
            _ => (path, false),
        };
        self.files.push((path, source, by_name, real));
    }
}
