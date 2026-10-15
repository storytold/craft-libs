//! Reports and errors returned by the store.

use std::fmt;
use std::io;
use std::path::PathBuf;
use std::time::Instant;

/// Why a schema migration step failed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MigrateError {
    /// The `schema_version` the step started from.
    pub from: u32,
    /// What went wrong.
    pub message: String,
}

impl MigrateError {
    /// A failure migrating from version `from`.
    pub fn new(from: u32, message: impl Into<String>) -> Self {
        Self { from, message: message.into() }
    }
}

impl fmt::Display for MigrateError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "migration from schema version {} failed: {}", self.from, self.message)
    }
}

impl std::error::Error for MigrateError {}

/// A `[section]` that parsed as TOML but did not fit the settings type; its defaults were used.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SectionError {
    /// The top-level key.
    pub section: String,
    /// The deserialization error.
    pub message: String,
}

/// Why the store will not write `config.toml`.
#[derive(Debug)]
pub enum ReadOnlyReason {
    /// The configuration folder could not be created.
    DirUnavailable(io::Error),
    /// The configuration folder exists but is not writable.
    DirNotWritable(io::Error),
    /// `config.toml` exists but could not be read.
    Unreadable(io::Error),
    /// `config.toml` is not valid TOML and could not be moved aside.
    CannotMoveAside(io::Error),
    /// The file was written by a newer build.
    NewerSchema {
        /// Version in the file.
        file: u32,
        /// Version this build knows.
        app: u32,
    },
    /// A schema migration step failed; the file is left untouched.
    Migration(MigrateError),
}

impl fmt::Display for ReadOnlyReason {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::DirUnavailable(e) => write!(f, "settings folder cannot be created: {e}"),
            Self::DirNotWritable(e) => write!(f, "settings folder is not writable: {e}"),
            Self::Unreadable(e) => write!(f, "config.toml cannot be read: {e}"),
            Self::CannotMoveAside(e) => write!(f, "config.toml is damaged and cannot be moved aside: {e}"),
            Self::NewerSchema { file, app } => write!(f, "config.toml has schema version {file}, newer than this build's {app}"),
            Self::Migration(e) => write!(f, "{e}"),
        }
    }
}

/// Where the loaded values came from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Source {
    /// Parsed from `config.toml`.
    File,
    /// `config.toml` did not exist or was unusable; defaults were used.
    Defaults,
    /// Imported from an older per-app file (left in place).
    Legacy(PathBuf),
}

/// What happened while opening the store. Nothing in here is fatal; the store always opens.
#[derive(Debug)]
pub struct LoadReport {
    /// The configuration folder; `None` when nothing is read or written (isolated or in memory).
    pub dir: Option<PathBuf>,
    /// The rule that chose `dir` (see [`crate::Rule`]), as text so the report stays `Debug`-only.
    pub rule: String,
    /// Where the values came from.
    pub source: Source,
    /// Set when a damaged `config.toml` was renamed; the new name.
    pub broken_moved_to: Option<PathBuf>,
    /// Sections whose values were replaced by defaults.
    pub section_errors: Vec<SectionError>,
    /// Set when the file was upgraded from an older `schema_version`.
    pub migrated_from: Option<u32>,
    /// Set when the store will not save.
    pub read_only: Option<ReadOnlyReason>,
}

impl LoadReport {
    /// True when something should be shown to the user.
    pub fn has_problems(&self) -> bool {
        self.broken_moved_to.is_some() || !self.section_errors.is_empty() || self.read_only.is_some()
    }
}

/// A save failure.
#[derive(Debug)]
pub enum SettingsError {
    /// The store is read-only; see the reason in the load report.
    ReadOnly(String),
    /// The settings could not be turned into TOML.
    Serialize(String),
    /// Writing `config.toml` failed.
    Io(io::Error),
}

impl fmt::Display for SettingsError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ReadOnly(why) => write!(f, "settings are read-only: {why}"),
            Self::Serialize(e) => write!(f, "settings cannot be serialized: {e}"),
            Self::Io(e) => write!(f, "settings could not be saved: {e}"),
        }
    }
}

impl std::error::Error for SettingsError {}

impl From<io::Error> for SettingsError {
    fn from(e: io::Error) -> Self {
        Self::Io(e)
    }
}

/// Outcome of one [`crate::SettingsStore::tick`].
#[derive(Debug)]
pub enum SaveStatus {
    /// Nothing to save.
    Idle,
    /// Changes are waiting for the debounce or retry delay.
    Pending,
    /// A save just succeeded.
    Saved,
    /// A save just failed; it will be retried.
    Failed(SettingsError),
}

/// What [`crate::SettingsStore::tick`] returns.
#[derive(Debug)]
pub struct TickResult {
    /// What happened this tick.
    pub status: SaveStatus,
    /// When to call `tick` again so a pending save happens even if the app is idle.
    /// Pass it to `egui::Context::request_repaint_after`.
    pub next_deadline: Option<Instant>,
}
