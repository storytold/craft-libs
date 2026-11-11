//! The service traits of a [`Host`](crate::Host) and their default implementations.

use std::collections::HashMap;
use std::fmt;
use std::sync::{Arc, Mutex};

use serde::Serialize;
use serde::de::DeserializeOwned;

/// Wakes the app's event loop so it runs a frame (`ctx.request_repaint()` in an egui app).
/// Services call it from any thread when work they did for the UI is ready. Cloning shares the
/// callback.
#[derive(Clone)]
pub struct Wake(Arc<dyn Fn() + Send + Sync>);

impl Wake {
    /// A wake that runs `wake`; it may be called from any thread.
    pub fn new(wake: impl Fn() + Send + Sync + 'static) -> Self {
        Self(Arc::new(wake))
    }

    /// A wake that does nothing (tests, headless runs).
    pub fn none() -> Self {
        Self::new(|| {})
    }

    /// Asks the event loop for a frame.
    pub fn wake(&self) {
        (self.0)();
    }
}

impl fmt::Debug for Wake {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Wake")
    }
}

/// Who the app is: shown by About, the updater and recovery.
#[derive(Clone, Debug)]
pub struct AppInfo {
    /// The settings folder name, as craft-settings' `APP_ID` (`"photocraft"`).
    pub id: &'static str,
    /// The display name (`"PhotoCraft"`).
    pub name: &'static str,
    /// `env!("CARGO_PKG_VERSION")`.
    pub version: &'static str,
}

/// Library features' settings sections in the app's settings file, one TOML table per section.
/// [`MemorySections`] keeps them in memory (tests, wasm without storage).
pub trait SectionStore: Send + Sync {
    /// The table stored under `key`, if any.
    fn read(&self, key: &str) -> Option<toml::Table>;
    /// Stores `table` under `key`, replacing the previous one.
    fn write(&self, key: &str, table: toml::Table);
}

/// A library feature's settings: one serde struct, saved under `KEY`. Not craft-settings'
/// `Settings` trait, which is the app's whole settings struct.
pub trait Section: Serialize + DeserializeOwned + Default {
    /// The table name in the settings file (`"fonts_panel"`).
    const KEY: &'static str;
}

/// Sections kept in memory.
#[derive(Default)]
pub struct MemorySections(Mutex<HashMap<String, toml::Table>>);

impl SectionStore for MemorySections {
    fn read(&self, key: &str) -> Option<toml::Table> {
        self.0.lock().unwrap_or_else(|e| e.into_inner()).get(key).cloned()
    }

    fn write(&self, key: &str, table: toml::Table) {
        self.0.lock().unwrap_or_else(|e| e.into_inner()).insert(key.to_owned(), table);
    }
}

/// Command words by command id (`"file.save"`). craft-keymap implements it over the app's
/// catalog; [`NoCommands`] returns the English label it is given.
pub trait Commands: Send + Sync {
    /// The label of command `id`; `english` is the library's default wording.
    fn label(&self, id: &str, english: &str) -> String;
    /// The shortcut hint of command `id`, if it has one. The default is none.
    fn hint(&self, _id: &str) -> Option<String> {
        None
    }
}

/// English command labels.
pub struct NoCommands;

impl Commands for NoCommands {
    fn label(&self, _id: &str, english: &str) -> String {
        english.to_owned()
    }
}

/// Background work.
pub trait Jobs: Send + Sync {
    /// Runs `job` in the background; `name` labels the thread where there is one.
    fn spawn(&self, name: &str, job: Box<dyn FnOnce() + Send>);
}

/// Spawns a thread per job.
#[cfg(not(target_arch = "wasm32"))]
pub struct ThreadJobs;

#[cfg(not(target_arch = "wasm32"))]
impl Jobs for ThreadJobs {
    fn spawn(&self, name: &str, job: Box<dyn FnOnce() + Send>) {
        // A failed spawn drops the job: background work is never required for correctness.
        let _ = std::thread::Builder::new().name(name.to_owned()).spawn(job);
    }
}

/// Runs each job at once on the calling thread (tests, wasm).
pub struct InlineJobs;

impl Jobs for InlineJobs {
    fn spawn(&self, _name: &str, job: Box<dyn FnOnce() + Send>) {
        job();
    }
}

/// A [`Jobs`] that keeps every job until a test or story runs it: [`run_next`](Self::run_next)
/// runs the oldest one on the calling thread. Stories show loading, stand-in and failure states
/// one step at a time ("Finish next"), and tests order background work exactly, without threads.
/// A job that spawns another while it runs (a [`JobQueue`](crate::queue::JobQueue) lane starting
/// its next run) queues it behind the others; the lock is not held while a job runs.
#[derive(Default)]
pub struct ManualJobs {
    waiting: Mutex<std::collections::VecDeque<Box<dyn FnOnce() + Send>>>,
}

impl ManualJobs {
    /// A queue with no waiting jobs.
    pub fn new() -> Self {
        Self::default()
    }

    /// Runs the oldest waiting job; false when none waits.
    pub fn run_next(&self) -> bool {
        // new: the stories' "Finish next" button and the queue tests.
        let job = self.waiting.lock().unwrap_or_else(|e| e.into_inner()).pop_front();
        job.map(|job| job()).is_some()
    }

    /// Runs jobs until none waits, including the ones they spawn, and returns how many ran.
    pub fn run_all(&self) -> usize {
        // new: a test that only needs every job done.
        let mut ran = 0;
        while self.run_next() {
            ran += 1;
        }
        ran
    }

    /// How many jobs wait.
    pub fn waiting(&self) -> usize {
        // new: stories show it beside the "Finish next" button.
        self.waiting.lock().unwrap_or_else(|e| e.into_inner()).len()
    }
}

impl Jobs for ManualJobs {
    /// Queues `job` behind the waiting ones; `name` is not kept.
    fn spawn(&self, _name: &str, job: Box<dyn FnOnce() + Send>) {
        // new: see `ManualJobs`.
        self.waiting.lock().unwrap_or_else(|e| e.into_inner()).push_back(job);
    }
}
