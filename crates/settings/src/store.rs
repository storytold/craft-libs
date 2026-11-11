//! The in-memory store: load once, read every frame, save while the application runs.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use toml_edit::DocumentMut;

use crate::Settings;
use crate::document::{self, merge_changes, merge_into, set_schema_version};
use crate::error::{LoadReport, ReadOnlyReason, SaveStatus, SettingsError, Source, TickResult};
use crate::files::{DiskFiles, SettingsFiles};
use crate::migrate::{self, LegacyFile, Upgrade};
use crate::paths::{self, Location, Portable, Probe};

mod sections;

pub use sections::SharedSettings;

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
    /// How the files are read and written ([`DiskFiles`] by default). The store reads and writes
    /// `config.toml`, its backup and the damaged-file rename only through it.
    pub files: Arc<dyn SettingsFiles>,
}

impl<T> Default for OpenOptions<T> {
    fn default() -> Self {
        Self { location: Location::Auto, legacy: Vec::new(), debounce: DEFAULT_DEBOUNCE, probe: Probe::default(), files: Arc::new(DiskFiles) }
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

    /// Routes the store's file access through `files` (an app's own I/O, browser storage).
    pub fn files(mut self, files: Arc<dyn SettingsFiles>) -> Self {
        self.files = files;
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
    /// The document as this store last read it from disk or wrote it, before any migration run
    /// here; the common ancestor of the three-way merge in `save`.
    base: DocumentMut,
    files: Arc<dyn SettingsFiles>,
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
    /// Set when a save merged another instance's changes into this store's value; see
    /// [`SettingsStore::take_merged_change`].
    merged_change: bool,
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
        let resolved = paths::resolve_named(T::APP_ID, T::FOLDER_NAME, &portable, &options.location, &options.probe);
        let path = resolved.dir.as_ref().map(|d| d.join(CONFIG_FILE));
        let mut report = LoadReport {
            dir: resolved.dir.clone(),
            rule: resolved.rule.to_string(),
            warning: resolved.warning.clone(),
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
            base: default_document::<T>(),
            files: options.files,
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
            merged_change: false,
        };

        let (Some(dir), Some(path)) = (store.dir.clone(), store.path.clone()) else {
            return (store, report);
        };
        if let Err(e) = store.files.create_dir(&dir) {
            store.load_best_effort(&mut report);
            store.set_read_only(&mut report, ReadOnlyReason::DirUnavailable(e));
            return (store, report);
        }
        if let Err(e) = store.files.check_writable(&dir) {
            store.load_best_effort(&mut report);
            store.set_read_only(&mut report, ReadOnlyReason::DirNotWritable(e));
            return (store, report);
        }

        match store.files.read(&path) {
            Ok(Some(text)) => store.load_text(&path, &text, &mut report),
            Ok(None) => {
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

    /// True once a save merged another instance's changes into [`SettingsStore::get`]; drains
    /// the flag. Apply the new value where it shows (for a preferences window, `reload` it).
    pub fn take_merged_change(&mut self) -> bool {
        std::mem::take(&mut self.merged_change)
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
        if !self.dirty && (self.written || self.files.read(path).is_ok_and(|text| text.is_some())) {
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
            && let Ok(Some(text)) = self.files.read(path)
            && let Ok(mut doc) = document::parse(&text)
        {
            let base = doc.clone();
            // A failed migration leaves the document half-upgraded; show defaults instead.
            let Ok(upgraded) = migrate::upgrade::<T>(&mut doc) else {
                return;
            };
            let (value, errors) = document::deserialize_sections::<T>(&doc);
            self.value = value;
            self.doc = doc;
            self.base = base;
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
                match self.files.move_aside(path) {
                    Ok(moved) => report.broken_moved_to = Some(moved),
                    Err(e) => self.set_read_only(report, ReadOnlyReason::CannotMoveAside(e)),
                }
                return;
            }
        };
        let base = doc.clone();
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
        self.base = base;
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
        let (backup, theirs) = self.check_disk(&path)?;
        if let Some(on_disk) = backup {
            self.files.write_backup(&path, &on_disk)?;
        }
        merge_into(&mut self.doc, &self.value).map_err(|e| SettingsError::Serialize(e.to_string()))?;
        set_schema_version(&mut self.doc, T::SCHEMA_VERSION);
        if let Some(mut theirs) = theirs {
            let (ours, _) = document::deserialize_sections::<T>(&self.doc);
            merge_changes::<T>(&self.base, &self.doc, &mut theirs);
            self.doc = theirs;
            // The merged document is what is on disk now. Taking the value from it keeps the next
            // save from writing this store's old values over the other instance's keys.
            let (merged, _) = document::deserialize_sections::<T>(&self.doc);
            if merged != ours {
                self.merged_change = true;
            }
            self.value = merged;
        }
        let text = document::render(&self.doc);
        self.files.write(&path, &text)?;
        self.base = self.doc.clone();
        self.seen_text = Some(text);
        self.dirty = false;
        self.written = true;
        self.failures = 0;
        self.retry_at = None;
        Ok(())
    }

    /// Looks at the file on disk just before a save. Returns its text when it is a good backup
    /// (it parses and is not blank), and its document, upgraded to this build's schema, when
    /// another instance or a hand edit changed it since this store last read or wrote it.
    ///
    /// The contract is best effort, not multi-process locking: `save` merges this store's changes
    /// since its base into that document ([`merge_changes`]), so the other writer's comments,
    /// unknown keys and settings survive. A file that no longer parses is moved aside (the edit
    /// is kept, the previous backup stays), and a file from a newer build, or one whose migration
    /// fails, makes the store read-only instead of overwriting it, the same as at startup.
    fn check_disk(&mut self, path: &Path) -> Result<(Option<String>, Option<DocumentMut>), SettingsError> {
        let Some(text) = self.files.read(path)? else {
            return Ok((None, None));
        };
        // A blank file still parses, so adopting it below would replace this store's document --
        // and with it every comment and unknown key the app does not manage -- with an empty one;
        // blank text is never written as a backup either, so the content would be lost. A blank
        // file is not an edit to adopt: keep what this store loaded and leave the file alone.
        if text.trim().is_empty() {
            return Ok((None, None));
        }
        let Ok(mut doc) = document::parse(&text) else {
            self.moved_aside.push(self.files.move_aside(path)?);
            return Ok((None, None));
        };
        let changed = self.seen_text.as_deref() != Some(text.as_str());
        if changed {
            match migrate::upgrade::<T>(&mut doc) {
                Ok(Upgrade::Newer { file }) => {
                    let why = ReadOnlyReason::NewerSchema { file, app: T::SCHEMA_VERSION }.to_string();
                    self.read_only = Some(why.clone());
                    return Err(SettingsError::ReadOnly(why));
                }
                Ok(_) => {}
                // Same as at startup: leave the outside file untouched and stop saving.
                Err(e) => {
                    let why = ReadOnlyReason::Migration(e).to_string();
                    self.read_only = Some(why.clone());
                    return Err(SettingsError::ReadOnly(why));
                }
            }
        }
        Ok(((!text.trim().is_empty()).then_some(text), changed.then_some(doc)))
    }
}

/// What a store with no file holds as its base: the defaults, so only a change from them counts
/// as this store's change when another instance creates the file first.
fn default_document<T: Settings>() -> DocumentMut {
    let mut doc = document::new_document(T::SCHEMA_VERSION);
    // Defaults that don't serialize leave the base empty, which makes every key ours.
    let _ = merge_into(&mut doc, &T::default());
    doc
}

/// Exponential backoff: 1 s, 2 s, 4 s, … capped at [`MAX_RETRY_DELAY`].
pub fn retry_delay(failures: u32) -> Duration {
    let exp = failures.saturating_sub(1).min(16);
    (FIRST_RETRY_DELAY * 2u32.saturating_pow(exp)).min(MAX_RETRY_DELAY)
}
