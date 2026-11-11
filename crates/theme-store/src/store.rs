//! The store: the suite selection with the lock/revision protocol, and the theme library.

use std::{
    fmt, io,
    path::PathBuf,
    sync::{Arc, OnceLock},
    time::Duration,
};

use craft_theme::{
    Appearance, Contrast, ContrastIntent, Mode, Size, TextSize, ThemeDef, ThemeId, ThemePair, Variants, builtins,
    format::{self, ErrorKind, FormatError, FormatErrors, MAX_THEME_BYTES},
    resolve,
};
use serde_json::Map;

use crate::{
    SuiteDir,
    backend::{Backend, FileStamp, LibraryFile, Lock, LockError, Memory, WriteError},
    copy::{copy_id, replace_top_level_id},
    selection::{Accepted, MAX_SELECTION_BYTES, SelectionChange, SelectionDoc, SelectionError, merge, parse_selection, render_selection, salvage},
};

/// How long a write waits for another app's lock before giving up with [`StoreError::Busy`].
pub const LOCK_TIMEOUT: Duration = Duration::from_secs(2);

/// How the copies an "import as copy" tries are numbered: `<id>-2` up to `<id>-<MAX_COPIES>`.
const MAX_COPIES: u32 = 999;

/// How a [`Store`] writes.
pub struct StoreOptions {
    written_by: String,
    lock_timeout: Duration,
    last: Option<Accepted>,
    app_theme: Option<ThemePair>,
    #[cfg(not(target_arch = "wasm32"))]
    fs: Option<Box<dyn craft_atomic::AtomicFs + Send + Sync>>,
}

impl StoreOptions {
    /// `written_by` names the app in `selection.json`, such as `"pdfcraft 0.15.0"`.
    pub fn new(written_by: impl Into<String>) -> Self {
        StoreOptions {
            written_by: written_by.into(),
            lock_timeout: LOCK_TIMEOUT,
            last: None,
            app_theme: None,
            #[cfg(not(target_arch = "wasm32"))]
            fs: None,
        }
    }

    /// How long to wait for the suite lock before failing with [`StoreError::Busy`].
    pub fn lock_timeout(mut self, timeout: Duration) -> Self {
        self.lock_timeout = timeout;
        self
    }

    /// The themes the app uses for its own choice ([`AppChoice::theme`](craft_theme::AppChoice)),
    /// from its preferences. [`SuiteWatch::open`](crate::SuiteWatch::open) reads their library
    /// files (and the suite's themes in [`last_accepted`](Self::last_accepted)) before
    /// returning, so the first snapshot already has them. A plain `Store` ignores it.
    pub fn app_theme(mut self, themes: Option<ThemePair>) -> Self {
        self.app_theme = themes;
        self
    }

    /// The state to show until `selection.json` has been read, and while it is damaged or from a
    /// newer build: the copy the app saved in its own preferences.
    pub fn last_accepted(mut self, last: Accepted) -> Self {
        self.last = Some(last);
        self
    }

    /// The file system writes go through, for fault injection in tests.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn file_system(mut self, fs: Box<dyn craft_atomic::AtomicFs + Send + Sync>) -> Self {
        self.fs = Some(fs);
        self
    }
}

/// Why a write or a library operation didn't happen. Nothing on disk changed, apart from a
/// damaged `selection.json` moved aside.
#[derive(Clone, Debug)]
pub enum StoreError {
    /// Another app held the lock for the whole timeout, or wrote `selection.json` while this
    /// write was in progress. Retry.
    Busy,
    /// The selection changed in another app on a field this update also changed. Show `disk` and
    /// offer to apply the user's choice on top of it.
    Conflict {
        /// The state now on disk.
        disk: Accepted,
        /// The change that wasn't applied; "Apply mine" sends it again with `disk` as base.
        mine: SelectionChange,
    },
    /// `selection.json` is from a newer build (the store is read-only) or too large.
    Selection(SelectionError),
    /// The suite folder could not be read or written.
    Io(Arc<io::Error>),
    /// The theme file was refused.
    Format(FormatErrors),
    /// A library file whose `id` isn't its file name (`<id>.json`).
    FileName {
        /// The id the file declares.
        id: ThemeId,
    },
    /// A library theme has this id already; import it as a copy to keep both.
    IdExists(ThemeId),
    /// The suite selection uses this theme, in either mode.
    InUse(ThemeId),
    /// No library theme has this id.
    NotFound(ThemeId),
    /// Built-in themes can't be deleted.
    BuiltIn(ThemeId),
    /// The watch no longer had the result of this change when the client looked, and the
    /// suite's state doesn't show it. Retry.
    Lost,
}

impl From<io::Error> for StoreError {
    fn from(e: io::Error) -> Self {
        StoreError::Io(Arc::new(e))
    }
}

/// Something the app may want to show; the store keeps working with its last accepted state.
#[derive(Clone, Debug)]
pub enum Problem {
    /// `selection.json` is damaged (moved aside on the next write), from a newer build
    /// (read-only) or too large.
    Selection(SelectionError),
    /// `selection.json` could not be read.
    SelectionUnreadable(Arc<io::Error>),
    /// The library folder can't be listed.
    LibraryUnreadable(Arc<io::Error>),
    /// The watcher's worker thread isn't running as it should: it couldn't start, stopped, or
    /// failed during a check. Operations still run, on the calling thread if need be.
    Watcher(&'static str),
    /// A library file that can't be used; `stem` is its name without `.json`.
    Theme {
        /// The file name without `.json`.
        stem: String,
        /// Why the file was refused.
        error: StoreError,
    },
}

/// A parsed theme file and the bytes it came from.
#[derive(Clone, Debug)]
pub struct ThemeRecord {
    /// The parsed theme.
    pub theme: ThemeDef,
    /// Computed on first use and shared with clones and with the [`ThemeEntry`] made from this
    /// record, so parsing and library scans don't resolve every theme.
    warnings: Arc<OnceLock<usize>>,
    /// The file exactly as stored, unknown keys included.
    pub bytes: Arc<[u8]>,
}

impl ThemeRecord {
    /// Parses a user theme file ([`format::parse`]). Its id must be one users can create: the
    /// `craft.` prefix is reserved for built-ins.
    pub fn parse(bytes: &[u8]) -> Result<ThemeRecord, FormatErrors> {
        let file = format::parse(bytes)?;
        if let Err(e) = ThemeId::parse_user(file.theme.id.as_str()) {
            return Err(id_error(ErrorKind::BadId(e)));
        }
        Ok(ThemeRecord { theme: file.theme, warnings: Arc::default(), bytes: Arc::from(bytes) })
    }

    /// Contrast pairs below their minimum in standard contrast, over the modes the theme has.
    /// Advisory: they never block an import or a selection.
    pub fn warnings(&self) -> usize {
        *self.warnings.get_or_init(|| contrast_warnings(&self.theme))
    }
}

fn contrast_warnings(theme: &ThemeDef) -> usize {
    let modes: &[Mode] = match theme.variants {
        Variants::Both => &[Mode::Light, Mode::Dark],
        Variants::LightOnly => &[Mode::Light],
        Variants::DarkOnly => &[Mode::Dark],
    };
    let appearance =
        |mode| Appearance { mode, contrast: Contrast::Standard, size: Size::default(), text_size: TextSize::default(), reduced_motion: false };
    modes.iter().map(|&mode| resolve(theme, appearance(mode), None).warnings.len()).sum()
}

fn id_error(kind: ErrorKind) -> FormatErrors {
    FormatErrors(vec![FormatError { path: vec!["id".to_owned()], kind }])
}

/// A file in another theme format, such as the pilot's `craft-theme`: the library skips it.
pub(crate) fn is_other_schema(error: &StoreError) -> bool {
    matches!(error, StoreError::Format(e) if matches!(e.file_level(), Some(ErrorKind::OtherSchema { .. })))
}

/// A built-in theme by id.
pub(crate) fn builtin(id: &ThemeId) -> Option<&'static ThemeDef> {
    builtins().iter().find(|t| t.id == *id)
}

/// One library theme, as a theme picker lists it, with what a preview card resolves.
#[derive(Clone, Debug)]
pub struct ThemeEntry {
    /// The theme's id.
    pub id: ThemeId,
    /// The theme's display name.
    pub name: String,
    /// The theme's own version number.
    pub version: u32,
    /// The parsed theme; [`ThemeDef::variants`] says which modes it has.
    pub theme: ThemeDef,
    warnings: Arc<OnceLock<usize>>,
    /// A low-contrast theme is labelled so in the picker.
    pub contrast_intent: ContrastIntent,
    /// The theme file's stamp when it was read.
    pub stamp: FileStamp,
}

impl ThemeEntry {
    /// [`ThemeRecord::warnings`], computed on first use.
    pub fn warnings(&self) -> usize {
        *self.warnings.get_or_init(|| contrast_warnings(&self.theme))
    }

    /// An entry for `record`, read with `stamp`.
    pub fn new(record: &ThemeRecord, stamp: FileStamp) -> Self {
        let t = &record.theme;
        ThemeEntry {
            id: t.id.clone(),
            name: t.name.clone(),
            version: t.version,
            theme: t.clone(),
            warnings: Arc::clone(&record.warnings),
            contrast_intent: t.contrast_intent,
            stamp,
        }
    }
}

/// The user themes in the library folder, by id. Built-ins come from [`craft_theme::builtins`];
/// files in another theme format are skipped.
#[derive(Clone, Debug, Default)]
pub struct Library {
    /// The valid themes.
    pub entries: Vec<ThemeEntry>,
    /// Files that could not be used.
    pub problems: Vec<Problem>,
}

/// A successful import.
#[derive(Clone, Debug)]
pub struct Imported {
    /// The stored theme; for a copy, with its new id.
    pub record: ThemeRecord,
    /// The selection after the import.
    pub accepted: Accepted,
}

/// How many times the store has parsed each kind of file, for tests that check work is not
/// repeated.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ParseCounts {
    /// Parses of `selection.json`.
    pub selection: u64,
    /// Parses of theme files.
    pub themes: u64,
    /// Theme files in another format (the pilot's `craft-theme`), counted in `themes` too.
    pub other_schema: u64,
}

/// The last bytes of `selection.json` this store saw and what they parsed to, so unchanged bytes
/// are never parsed twice.
struct Seen {
    bytes: Option<Vec<u8>>,
    parsed: Parsed,
}

/// What `selection.json` parsed to; `None` for a missing file.
type Parsed = Result<Option<SelectionDoc>, SelectionError>;

/// `selection.json` as read under the lock.
struct DiskState {
    raw: Map<String, serde_json::Value>,
    accepted: Accepted,
    bytes: Option<Vec<u8>>,
    /// Damaged or too large: moved aside by the write that replaces it, if one happens.
    damaged: bool,
}

/// The suite folder (or its in-memory stand-in): the suite selection and the theme library.
///
/// Reads take no lock. Every write takes `selection.lock`, re-reads `selection.json`, merges and
/// writes it with a revision one higher than on disk. Methods block on file I/O and the lock, so
/// apps call them off the UI thread.
pub struct Store {
    backend: Backend,
    written_by: String,
    lock_timeout: Duration,
    accepted: Accepted,
    seen: Option<Seen>,
    too_new: Option<u64>,
    moved_aside: Vec<PathBuf>,
    counts: ParseCounts,
    /// From [`StoreOptions::app_theme`], for the watch.
    pub(crate) app_theme: Option<ThemePair>,
}

impl Store {
    /// Opens the store without touching the disk; call [`Store::load_selection`] next. Nothing is
    /// created until the first write.
    ///
    /// In memory, each store has its own state: open one per process and share it (a
    /// [`SuiteWatch`](crate::SuiteWatch) owns it), or apps in the same process won't see each
    /// other's changes.
    pub fn open(dir: &SuiteDir, options: StoreOptions) -> Store {
        let backend = match dir.path() {
            None => Backend::Memory(Memory::default()),
            #[cfg(not(target_arch = "wasm32"))]
            Some(path) => {
                Backend::Disk(crate::disk::Disk { dir: path.to_path_buf(), fs: options.fs.unwrap_or_else(|| Box::new(craft_atomic::RealFs)) })
            }
            #[cfg(target_arch = "wasm32")]
            Some(_) => Backend::Memory(Memory::default()),
        };
        Store {
            backend,
            written_by: options.written_by,
            lock_timeout: options.lock_timeout,
            accepted: options.last.unwrap_or_default(),
            seen: None,
            too_new: None,
            moved_aside: Vec::new(),
            counts: ParseCounts::default(),
            app_theme: options.app_theme,
        }
    }

    /// `None` in memory.
    pub fn dir(&self) -> Option<PathBuf> {
        self.backend.dir()
    }

    /// The selection as last read or written.
    pub fn accepted(&self) -> &Accepted {
        &self.accepted
    }

    /// True while `selection.json` is from a newer build: every write is refused.
    pub fn is_read_only(&self) -> bool {
        self.too_new.is_some()
    }

    /// Damaged `selection.json` files moved aside since the last call.
    pub fn take_moved_aside(&mut self) -> Vec<PathBuf> {
        std::mem::take(&mut self.moved_aside)
    }

    /// How many files of each kind the store has parsed.
    pub fn parse_counts(&self) -> ParseCounts {
        self.counts
    }

    /// Reads `selection.json` (parsing it only when its bytes changed) and returns whether the
    /// accepted state changed. A missing file means no suite selection. A damaged, too new or
    /// too large file keeps the last accepted state and comes back as a problem.
    pub fn load_selection(&mut self) -> Result<bool, Problem> {
        let before = self.accepted.clone();
        match self.read_disk() {
            Err(e) => Err(Problem::SelectionUnreadable(Arc::new(e))),
            Ok((_, Ok(doc))) => {
                self.too_new = None;
                self.accepted = doc.map(|d| d.accepted).unwrap_or_default();
                Ok(self.accepted != before)
            }
            Ok((_, Err(e))) => {
                if let SelectionError::TooNew { schema_version } = e {
                    self.too_new = Some(schema_version);
                }
                Err(Problem::Selection(e))
            }
        }
    }

    /// `selection.json`'s stamp, for cheap change checks; `None` when it doesn't exist.
    pub fn selection_stamp(&self) -> io::Result<Option<FileStamp>> {
        self.backend.selection_stamp()
    }

    /// Writes the fields of the suite choice the user changed. `base` is the accepted state the
    /// change was made on. Fields merge as docs/suite-themes.md describes; on success returns the
    /// new accepted state. A change with no fields, or one that matches the file, writes nothing.
    /// A library theme that no longer exists is [`StoreError::NotFound`].
    pub fn update(&mut self, base: &Accepted, change: SelectionChange) -> Result<Accepted, StoreError> {
        if change.is_empty() {
            return Ok(self.accepted.clone());
        }
        let _lock = self.lock()?;
        let disk = self.read_for_write(base)?;
        let Some(merged) = merge(&base.suite, &disk.accepted.suite, &change) else {
            self.accepted = disk.accepted.clone();
            return Err(StoreError::Conflict { disk: disk.accepted, mine: change });
        };
        if merged == disk.accepted.suite {
            self.accepted = disk.accepted.clone();
            return Ok(disk.accepted);
        }
        for theme in change.picked() {
            if !theme.is_builtin() && self.theme_stamp(theme)?.is_none() {
                return Err(StoreError::NotFound(theme.clone()));
            }
        }
        let next = Accepted { suite: merged, revision: disk.accepted.revision.saturating_add(1), ..disk.accepted.clone() };
        self.commit(&disk, next)
    }

    /// Adds a theme file to the library, keeping its bytes exactly. An id that is taken gives
    /// [`StoreError::IdExists`]; with `as_copy` the theme gets the first free `<id>-<n>` instead,
    /// and only its `id` changes. Contrast findings never block an import.
    pub fn import(&mut self, bytes: &[u8], as_copy: bool) -> Result<Imported, StoreError> {
        let record = self.parse_theme_counted(bytes).map_err(StoreError::Format)?;
        let _lock = self.lock()?;
        let base = self.accepted.clone();
        let disk = self.read_for_write(&base)?;
        let record = self.place(record, as_copy)?;
        let next = library_bump(&disk.accepted);
        match self.commit(&disk, next) {
            Ok(accepted) => Ok(Imported { record, accepted }),
            Err(e) => {
                let _ = self.backend.remove_theme(record.theme.id.as_str());
                Err(e)
            }
        }
    }

    fn place(&mut self, record: ThemeRecord, as_copy: bool) -> Result<ThemeRecord, StoreError> {
        let id = record.theme.id.clone();
        match self.backend.create_theme(id.as_str(), &record.bytes) {
            Ok(()) => return Ok(record),
            Err(e) if e.kind() != io::ErrorKind::AlreadyExists => return Err(e.into()),
            Err(_) if !as_copy => return Err(StoreError::IdExists(id)),
            Err(_) => {}
        }
        for n in 2..=MAX_COPIES {
            let copy = copy_id(&id, n).map_err(|e| StoreError::Format(id_error(ErrorKind::BadId(e))))?;
            let bytes = replace_top_level_id(&record.bytes, &copy).ok_or_else(|| StoreError::Format(id_error(ErrorKind::Missing)))?;
            let copied = self.parse_theme_counted(&bytes).map_err(StoreError::Format)?;
            match self.backend.create_theme(copy.as_str(), &bytes) {
                Ok(()) => return Ok(copied),
                Err(e) if e.kind() == io::ErrorKind::AlreadyExists => continue,
                Err(e) => return Err(e.into()),
            }
        }
        Err(StoreError::IdExists(id))
    }

    /// Removes a library theme. A theme in either slot of the suite selection can't be deleted;
    /// per-app choices that name it fall back to the app default when they resolve.
    ///
    /// The file is renamed to a hidden name first and removed only once `selection.json` records
    /// the deletion, so a failed write puts it back.
    pub fn delete(&mut self, id: &ThemeId) -> Result<Accepted, StoreError> {
        if id.is_builtin() {
            return Err(StoreError::BuiltIn(id.clone()));
        }
        let stem = user_stem(id)?;
        let _lock = self.lock()?;
        let base = self.accepted.clone();
        let disk = self.read_for_write(&base)?;
        let suite = &disk.accepted.suite;
        if suite.theme_light.as_ref() == Some(id) || suite.theme_dark.as_ref() == Some(id) {
            return Err(StoreError::InUse(id.clone()));
        }
        match self.backend.hide_theme(stem) {
            Ok(()) => {}
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Err(StoreError::NotFound(id.clone())),
            Err(e) => return Err(e.into()),
        }
        match self.commit(&disk, library_bump(&disk.accepted)) {
            Ok(accepted) => {
                // A leftover hidden file is never listed; the next delete of this id replaces it.
                let _ = self.backend.purge_hidden_theme(stem);
                Ok(accepted)
            }
            Err(e) => {
                let _ = self.backend.unhide_theme(stem);
                Err(e)
            }
        }
    }

    /// A theme file for sharing: a library theme's stored bytes, or a built-in written out.
    pub fn export(&self, id: &ThemeId) -> Result<Arc<[u8]>, StoreError> {
        if let Some(theme) = builtin(id) {
            let text = format::to_file(theme).and_then(|f| format::write(&f)).map_err(StoreError::Format)?;
            return Ok(Arc::from(text.into_bytes()));
        }
        let bytes = self.backend.read_theme(user_stem(id)?, MAX_THEME_BYTES)?.ok_or_else(|| StoreError::NotFound(id.clone()))?;
        if bytes.len() > MAX_THEME_BYTES {
            let too_large = FormatError { path: Vec::new(), kind: ErrorKind::TooLarge { len: bytes.len() } };
            return Err(StoreError::Format(FormatErrors(vec![too_large])));
        }
        Ok(Arc::from(bytes))
    }

    /// The library's files and their stamps, without reading them.
    pub fn library_files(&self) -> io::Result<Vec<LibraryFile>> {
        self.backend.list()
    }

    /// Reads and parses one library theme.
    pub fn load_theme(&mut self, id: &ThemeId) -> Result<ThemeRecord, StoreError> {
        self.load_stem(user_stem(id)?).and_then(|r| r.ok_or_else(|| StoreError::NotFound(id.clone())))
    }

    /// A library theme's stamp; `None` when it doesn't exist.
    pub fn theme_stamp(&self, id: &ThemeId) -> io::Result<Option<FileStamp>> {
        match user_stem(id) {
            Ok(stem) => self.backend.theme_stamp(stem),
            Err(_) => Ok(None),
        }
    }

    /// A built-in or library theme by id, for
    /// [`resolve_selection`](craft_theme::resolve_selection)'s `lookup`
    /// (`&mut |id| store.theme(id)`).
    pub fn theme(&mut self, id: &ThemeId) -> Option<ThemeDef> {
        match builtin(id) {
            Some(t) => Some(t.clone()),
            None if id.is_builtin() => None,
            None => self.load_theme(id).ok().map(|r| r.theme),
        }
    }

    /// Lists and parses every library file. Files that can't be used are problems, not errors;
    /// files in another theme format are skipped.
    pub fn library(&mut self) -> Library {
        let files = match self.backend.list() {
            Ok(files) => files,
            Err(e) => return Library { entries: Vec::new(), problems: vec![Problem::LibraryUnreadable(Arc::new(e))] },
        };
        let mut library = Library::default();
        for file in files {
            match self.load_stem(&file.stem) {
                Ok(Some(record)) => library.entries.push(ThemeEntry::new(&record, file.stamp)),
                // Removed since the listing.
                Ok(None) => {}
                Err(error) if is_other_schema(&error) => {}
                Err(error) => library.problems.push(Problem::Theme { stem: file.stem, error }),
            }
        }
        library
    }

    pub(crate) fn load_stem(&mut self, stem: &str) -> Result<Option<ThemeRecord>, StoreError> {
        let Some(bytes) = self.backend.read_theme(stem, MAX_THEME_BYTES)? else {
            return Ok(None);
        };
        let record = self.parse_theme_counted(&bytes).map_err(StoreError::Format)?;
        if record.theme.id.as_str() != stem {
            return Err(StoreError::FileName { id: record.theme.id });
        }
        Ok(Some(record))
    }

    fn parse_theme_counted(&mut self, bytes: &[u8]) -> Result<ThemeRecord, FormatErrors> {
        self.counts.themes += 1;
        let parsed = ThemeRecord::parse(bytes);
        if let Err(e) = &parsed
            && matches!(e.file_level(), Some(ErrorKind::OtherSchema { .. }))
        {
            self.counts.other_schema += 1;
        }
        parsed
    }

    fn lock(&self) -> Result<Lock, StoreError> {
        self.backend.lock(self.lock_timeout).map_err(|e| match e {
            LockError::Busy => StoreError::Busy,
            LockError::Io(e) => e.into(),
        })
    }

    /// `selection.json`'s bytes and what they parse to (`None` for a missing file).
    fn read_disk(&mut self) -> io::Result<(Option<Vec<u8>>, Parsed)> {
        let bytes = self.backend.read_selection(MAX_SELECTION_BYTES)?;
        if let Some(seen) = &self.seen
            && seen.bytes == bytes
        {
            return Ok((bytes, seen.parsed.clone()));
        }
        let parsed = match &bytes {
            None => Ok(None),
            Some(b) => {
                self.counts.selection += 1;
                parse_selection(b).map(Some)
            }
        };
        self.seen = Some(Seen { bytes: bytes.clone(), parsed: parsed.clone() });
        Ok((bytes, parsed))
    }

    /// Step 2 of the protocol, under the lock. A damaged or oversized file is left for the write
    /// to move aside, with `base` and the file's salvaged counters standing in for it; a newer
    /// one stops the write.
    fn read_for_write(&mut self, base: &Accepted) -> Result<DiskState, StoreError> {
        let (bytes, parsed) = self.read_disk()?;
        match parsed {
            Ok(Some(doc)) => {
                self.too_new = None;
                Ok(DiskState { raw: doc.raw, accepted: doc.accepted, bytes, damaged: false })
            }
            Ok(None) => {
                self.too_new = None;
                Ok(DiskState { raw: Map::new(), accepted: Accepted::default(), bytes: None, damaged: false })
            }
            Err(SelectionError::Corrupt(_) | SelectionError::TooLarge { .. }) => {
                self.too_new = None;
                Ok(DiskState { raw: Map::new(), accepted: salvage(bytes.as_deref(), base), bytes, damaged: true })
            }
            Err(e) => {
                if let SelectionError::TooNew { schema_version } = e {
                    self.too_new = Some(schema_version);
                }
                Err(StoreError::Selection(e))
            }
        }
    }

    /// Step 4: writes `next` over what `disk` read, keeping keys this build doesn't know. A
    /// damaged file is moved aside only now, and put back if the write fails.
    fn commit(&mut self, disk: &DiskState, next: Accepted) -> Result<Accepted, StoreError> {
        let mut raw = disk.raw.clone();
        let bytes = render_selection(&mut raw, &next, &self.written_by);
        if bytes.len() > MAX_SELECTION_BYTES {
            return Err(StoreError::Selection(SelectionError::TooLarge { len: bytes.len() }));
        }
        let moved = if disk.damaged { self.backend.move_aside_selection()? } else { None };
        let expected = if disk.damaged { None } else { disk.bytes.as_deref() };
        if let Err(e) = self.backend.write_selection(&bytes, expected) {
            if let Some(moved) = &moved {
                let _ = self.backend.restore_selection(moved);
            }
            return Err(match e {
                WriteError::Raced => StoreError::Busy,
                WriteError::Io(e) => e.into(),
            });
        }
        self.moved_aside.extend(moved);
        self.seen = Some(Seen { bytes: Some(bytes), parsed: Ok(Some(SelectionDoc { raw, accepted: next.clone() })) });
        self.accepted = next.clone();
        Ok(next)
    }
}

/// A library theme's file name without `.json`. Ids a user couldn't create (built-ins, Windows
/// device names) are never looked up on disk.
fn user_stem(id: &ThemeId) -> Result<&str, StoreError> {
    match ThemeId::parse_user(id.as_str()) {
        Ok(_) => Ok(id.as_str()),
        Err(_) => Err(StoreError::NotFound(id.clone())),
    }
}

fn library_bump(disk: &Accepted) -> Accepted {
    Accepted { revision: disk.revision.saturating_add(1), library_revision: disk.library_revision.saturating_add(1), suite: disk.suite.clone() }
}

impl fmt::Display for StoreError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            StoreError::Busy => f.write_str("another Craft app is changing the themes; try again"),
            StoreError::Conflict { .. } => f.write_str("changed in another Craft app; your choice wasn't applied"),
            StoreError::Selection(e) => e.fmt(f),
            StoreError::Io(e) => e.fmt(f),
            StoreError::Format(e) => e.fmt(f),
            StoreError::FileName { id } => write!(f, "the theme's id {id} doesn't match its file name; it should be {id}.json"),
            StoreError::IdExists(id) => write!(f, "a theme with the id {id} is already installed"),
            StoreError::InUse(id) => write!(f, "{id} is a suite theme; choose another theme before deleting it"),
            StoreError::NotFound(id) => write!(f, "there is no theme {id}"),
            StoreError::BuiltIn(id) => write!(f, "{id} is a built-in theme and can't be deleted"),
            StoreError::Lost => f.write_str("the result of this change was lost; it may not have been saved"),
        }
    }
}

impl std::error::Error for StoreError {}

impl fmt::Display for Problem {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Problem::Selection(e) => e.fmt(f),
            Problem::SelectionUnreadable(e) => write!(f, "can't read the suite theme selection: {e}"),
            Problem::LibraryUnreadable(e) => write!(f, "can't list the theme library: {e}"),
            Problem::Theme { stem, error } => write!(f, "{stem}.json: {error}"),
            Problem::Watcher(what) => f.write_str(what),
        }
    }
}
