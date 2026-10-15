//! The in-memory store: load once, read every frame, save while the application runs.

use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use toml_edit::DocumentMut;

use crate::Settings;
use crate::document::{self, merge_into, set_schema_version};
use crate::error::{LoadReport, ReadOnlyReason, SaveStatus, SettingsError, Source, TickResult};
use crate::migrate::{self, LegacyFile, Upgrade};
use crate::paths::{self, Location, Portable, Probe};
use crate::recovery;

/// The settings file name.
pub const CONFIG_FILE: &str = "config.toml";

/// How long after the last change a save is attempted.
pub const DEFAULT_DEBOUNCE: Duration = Duration::from_millis(500);

/// First retry delay after a failed save; each failure doubles it, up to [`MAX_RETRY_DELAY`].
pub const FIRST_RETRY_DELAY: Duration = Duration::from_secs(1);

/// Longest wait between retries.
pub const MAX_RETRY_DELAY: Duration = Duration::from_secs(30);

/// How to open a store.
pub struct OpenOptions<T> {
    /// Where the settings live.
    pub location: Location,
    /// Older files to import when there is no `config.toml` yet.
    pub legacy: Vec<LegacyFile<T>>,
    /// Quiet period after a change before saving.
    pub debounce: Duration,
    /// What the folder rules look at.
    pub probe: Probe,
}

impl<T> Default for OpenOptions<T> {
    fn default() -> Self {
        Self { location: Location::Auto, legacy: Vec::new(), debounce: DEFAULT_DEBOUNCE, probe: Probe::default() }
    }
}

impl<T> OpenOptions<T> {
    /// Options for `location`.
    pub fn new(location: Location) -> Self {
        Self { location, ..Self::default() }
    }

    /// Older files to import on first run.
    pub fn legacy(mut self, files: Vec<LegacyFile<T>>) -> Self {
        self.legacy = files;
        self
    }

    /// Quiet period after a change before saving.
    pub fn debounce(mut self, debounce: Duration) -> Self {
        self.debounce = debounce;
        self
    }

    /// What the folder rules look at.
    pub fn probe(mut self, probe: Probe) -> Self {
        self.probe = probe;
        self
    }
}

/// An application's settings, with the file they came from.
///
/// With no folder (an empty `<APP>_CONFIG_DIR`, or [`Location::Memory`]) the store keeps
/// everything in memory: `get`/`update` work as usual, nothing is read or written.
pub struct SettingsStore<T: Settings> {
    dir: Option<PathBuf>,
    path: Option<PathBuf>,
    doc: DocumentMut,
    value: T,
    read_only: Option<String>,
    dirty: bool,
    written: bool,
    last_change: Option<Instant>,
    debounce: Duration,
    failures: u32,
    retry_at: Option<Instant>,
    /// The file's contents as last read or written. Compared by content, not mtime, so an edit
    /// within the same second (coarse timestamps on FAT, HFS+, network shares) is still seen.
    seen_text: Option<String>,
    /// Files that were damaged on disk at save time and moved aside; see [`SettingsStore::take_moved_aside`].
    moved_aside: Vec<PathBuf>,
}

impl<T: Settings> std::fmt::Debug for SettingsStore<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SettingsStore")
            .field("path", &self.path)
            .field("read_only", &self.read_only)
            .field("dirty", &self.dirty)
            .finish_non_exhaustive()
    }
}

impl<T: Settings> SettingsStore<T> {
    /// Opens the store for `location`. Never fails: problems come back in the report.
    pub fn open(location: Location) -> (Self, LoadReport) {
        Self::open_with(OpenOptions::new(location))
    }

    /// Opens a store in `dir`, for settings that travel with a folder such as a library.
    pub fn open_at(dir: impl Into<PathBuf>) -> (Self, LoadReport) {
        Self::open(Location::Dir(dir.into()))
    }

    /// Opens the store with full options.
    pub fn open_with(options: OpenOptions<T>) -> (Self, LoadReport) {
        let portable = Portable { markers: T::PORTABLE_MARKERS, dir: T::PORTABLE_DIR };
        let resolved = paths::resolve(T::APP_ID, &portable, &options.location, &options.probe);
        let path = resolved.dir.as_ref().map(|d| d.join(CONFIG_FILE));
        let mut report = LoadReport {
            dir: resolved.dir.clone(),
            rule: resolved.rule.to_string(),
            source: Source::Defaults,
            broken_moved_to: None,
            section_errors: Vec::new(),
            migrated_from: None,
            read_only: None,
        };
        let mut store = Self {
            dir: resolved.dir,
            path,
            doc: document::new_document(T::SCHEMA_VERSION),
            value: T::default(),
            read_only: None,
            dirty: false,
            written: false,
            last_change: None,
            debounce: options.debounce,
            failures: 0,
            retry_at: None,
            seen_text: None,
            moved_aside: Vec::new(),
        };

        let (Some(dir), Some(path)) = (store.dir.clone(), store.path.clone()) else {
            return (store, report);
        };
        if let Err(e) = std::fs::create_dir_all(&dir) {
            store.load_best_effort(&mut report);
            store.set_read_only(&mut report, ReadOnlyReason::DirUnavailable(e));
            return (store, report);
        }
        if let Err(e) = recovery::check_writable(&dir) {
            store.load_best_effort(&mut report);
            store.set_read_only(&mut report, ReadOnlyReason::DirNotWritable(e));
            return (store, report);
        }

        match std::fs::read_to_string(&path) {
            Ok(text) => store.load_text(&path, &text, &mut report),
            Err(e) if e.kind() == io::ErrorKind::NotFound => {
                if let Some((value, from)) = migrate::import_legacy(&options.legacy) {
                    store.value = value;
                    store.dirty = true;
                    report.source = Source::Legacy(from);
                }
            }
            Err(e) => store.set_read_only(&mut report, ReadOnlyReason::Unreadable(e)),
        }
        (store, report)
    }

    /// The configuration folder; `None` when the store is in memory only.
    pub fn dir(&self) -> Option<&Path> {
        self.dir.as_deref()
    }

    /// The `config.toml` path; `None` when the store is in memory only.
    pub fn path(&self) -> Option<&Path> {
        self.path.as_deref()
    }

    /// True when the store reads and writes a file; false when it is in memory only.
    pub fn is_persistent(&self) -> bool {
        self.path.is_some()
    }

    /// The current settings. In memory; cheap to call every frame.
    pub fn get(&self) -> &T {
        &self.value
    }

    /// True when the store will not write the file; the load report says why.
    pub fn is_read_only(&self) -> bool {
        self.read_only.is_some()
    }

    /// Damaged `config.toml` files found and moved aside while saving (someone broke the file
    /// while the app was running). Drains the list; show these to the user like
    /// [`LoadReport::broken_moved_to`].
    pub fn take_moved_aside(&mut self) -> Vec<PathBuf> {
        std::mem::take(&mut self.moved_aside)
    }

    /// True when changes are waiting to be saved.
    pub fn is_dirty(&self) -> bool {
        self.dirty
    }

    /// Changes the settings. Returns true, and schedules a save, only if the value changed.
    pub fn update(&mut self, f: impl FnOnce(&mut T)) -> bool {
        let mut next = self.value.clone();
        f(&mut next);
        self.set(next)
    }

    /// Replaces the settings. Returns true, and schedules a save, only if the value changed.
    pub fn set(&mut self, value: T) -> bool {
        if value == self.value {
            return false;
        }
        self.value = value;
        self.mark_changed(Instant::now());
        true
    }

    /// Like [`SettingsStore::update`] with an explicit clock, for tests.
    pub fn update_at(&mut self, now: Instant, f: impl FnOnce(&mut T)) -> bool {
        let mut next = self.value.clone();
        f(&mut next);
        if next == self.value {
            return false;
        }
        self.value = next;
        self.mark_changed(now);
        true
    }

    /// Saves pending changes once the debounce or retry delay has passed. Call every frame
    /// and again at `next_deadline`, so an idle application still saves.
    pub fn tick(&mut self, now: Instant) -> TickResult {
        if !self.dirty || self.read_only.is_some() {
            return TickResult { status: SaveStatus::Idle, next_deadline: None };
        }
        let due = match (self.retry_at, self.last_change) {
            (Some(retry), _) => retry,
            (None, Some(changed)) => changed + self.debounce,
            (None, None) => now,
        };
        if now < due {
            return TickResult { status: SaveStatus::Pending, next_deadline: Some(due) };
        }
        match self.save() {
            Ok(()) => TickResult { status: SaveStatus::Saved, next_deadline: None },
            Err(e) => {
                self.failures = self.failures.saturating_add(1);
                let delay = retry_delay(self.failures);
                self.retry_at = Some(now + delay);
                TickResult { status: SaveStatus::Failed(e), next_deadline: Some(now + delay) }
            }
        }
    }

    /// Saves now. Writes the file even when nothing changed if it does not exist yet, so users
    /// can find it. Call on exit.
    pub fn flush(&mut self) -> Result<(), SettingsError> {
        if let Some(why) = &self.read_only {
            return Err(SettingsError::ReadOnly(why.clone()));
        }
        let Some(path) = &self.path else {
            return Ok(());
        };
        if !self.dirty && (self.written || path.exists()) {
            return Ok(());
        }
        self.save()
    }

    fn mark_changed(&mut self, now: Instant) {
        // An in-memory store has nothing to save, so it never becomes dirty.
        self.dirty = self.path.is_some();
        self.last_change = Some(now);
    }

    fn set_read_only(&mut self, report: &mut LoadReport, reason: ReadOnlyReason) {
        self.read_only = Some(reason.to_string());
        report.read_only = Some(reason);
    }

    /// Reads the file if it happens to be readable, so a read-only store still shows the
    /// user's values instead of defaults.
    fn load_best_effort(&mut self, report: &mut LoadReport) {
        if let Some(path) = &self.path
            && let Ok(text) = std::fs::read_to_string(path)
            && let Ok(mut doc) = document::parse(&text)
        {
            // A failed migration leaves the document half-upgraded; show defaults instead.
            let Ok(upgraded) = migrate::upgrade::<T>(&mut doc) else {
                return;
            };
            let (value, errors) = document::deserialize_sections::<T>(&doc);
            self.value = value;
            self.doc = doc;
            self.seen_text = Some(text);
            report.source = Source::File;
            report.section_errors = errors;
            if let Upgrade::Migrated { from } = upgraded {
                report.migrated_from = Some(from);
            }
        }
    }

    fn load_text(&mut self, path: &Path, text: &str, report: &mut LoadReport) {
        let mut doc = match document::parse(text) {
            Ok(doc) => doc,
            Err(_) => {
                match recovery::move_aside(path) {
                    Ok(moved) => report.broken_moved_to = Some(moved),
                    Err(e) => self.set_read_only(report, ReadOnlyReason::CannotMoveAside(e)),
                }
                return;
            }
        };
        match migrate::upgrade::<T>(&mut doc) {
            Ok(Upgrade::Current) => {}
            Ok(Upgrade::Migrated { from }) => {
                report.migrated_from = Some(from);
                self.dirty = true;
            }
            Ok(Upgrade::Newer { file }) => self.set_read_only(report, ReadOnlyReason::NewerSchema { file, app: T::SCHEMA_VERSION }),
            Err(e) => {
                self.set_read_only(report, ReadOnlyReason::Migration(e));
                return;
            }
        }
        let (value, errors) = document::deserialize_sections::<T>(&doc);
        self.value = value;
        self.doc = doc;
        self.seen_text = Some(text.to_owned());
        report.source = Source::File;
        report.section_errors = errors;
    }

    fn save(&mut self) -> Result<(), SettingsError> {
        if let Some(why) = &self.read_only {
            return Err(SettingsError::ReadOnly(why.clone()));
        }
        let Some(path) = self.path.clone() else {
            self.dirty = false;
            return Ok(());
        };
        if let Some(on_disk) = self.check_disk(&path)? {
            recovery::write_backup(&path, &on_disk)?;
        }
        merge_into(&mut self.doc, &self.value).map_err(|e| SettingsError::Serialize(e.to_string()))?;
        set_schema_version(&mut self.doc, T::SCHEMA_VERSION);
        let text = document::render(&self.doc);
        craft_atomic::atomic_write_with(&path, |file| {
            restrict_permissions(file)?;
            file.write_all(text.as_bytes())
        })?;
        self.seen_text = Some(text);
        self.dirty = false;
        self.written = true;
        self.failures = 0;
        self.retry_at = None;
        Ok(())
    }

    /// Looks at the file on disk just before a save, and returns its text when it is a good
    /// backup (it parses and is not blank).
    ///
    /// The contract is best effort, not multi-process locking: if another instance or a hand
    /// edit changed the file since this store last read or wrote it, the new document is adopted
    /// so its comments and unknown keys survive, and this store's values then win on known keys.
    /// A file that no longer parses is moved aside (the edit is kept, the previous backup stays),
    /// and a file from a newer build, or one whose migration fails, makes the store read-only
    /// instead of overwriting it, the same as at startup.
    fn check_disk(&mut self, path: &Path) -> Result<Option<String>, SettingsError> {
        let text = match std::fs::read_to_string(path) {
            Ok(text) => text,
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(None),
            Err(e) => return Err(e.into()),
        };
        let Ok(mut doc) = document::parse(&text) else {
            self.moved_aside.push(recovery::move_aside(path)?);
            return Ok(None);
        };
        if self.seen_text.as_deref() != Some(text.as_str()) {
            match migrate::upgrade::<T>(&mut doc) {
                Ok(Upgrade::Newer { file }) => {
                    let why = ReadOnlyReason::NewerSchema { file, app: T::SCHEMA_VERSION }.to_string();
                    self.read_only = Some(why.clone());
                    return Err(SettingsError::ReadOnly(why));
                }
                Ok(_) => self.doc = doc,
                // Same as at startup: leave the outside file untouched and stop saving.
                Err(e) => {
                    let why = ReadOnlyReason::Migration(e).to_string();
                    self.read_only = Some(why.clone());
                    return Err(SettingsError::ReadOnly(why));
                }
            }
        }
        Ok((!text.trim().is_empty()).then_some(text))
    }
}

/// Exponential backoff: 1 s, 2 s, 4 s, … capped at [`MAX_RETRY_DELAY`].
pub fn retry_delay(failures: u32) -> Duration {
    let exp = failures.saturating_sub(1).min(16);
    (FIRST_RETRY_DELAY * 2u32.saturating_pow(exp)).min(MAX_RETRY_DELAY)
}

/// New files are owner-only; on a replace `craft-atomic` keeps the existing file's mode.
#[cfg(unix)]
fn restrict_permissions(file: &std::fs::File) -> io::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    file.set_permissions(std::fs::Permissions::from_mode(0o600))
}

#[cfg(not(unix))]
fn restrict_permissions(_file: &std::fs::File) -> io::Result<()> {
    Ok(())
}
