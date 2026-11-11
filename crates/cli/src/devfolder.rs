//! Developer mode: watch a folder of `.wasm` plugins and reload one when its file changes.
//!
//! `plugin-host` has the install-or-replace step (`PluginManager::install` with `Source::DevFolder`
//! reloads in place) but no folder watcher. This module is that watcher. It talks to the host
//! through [`DevTarget`], so an app can run it over its own manager. [`crate::run::Headless`]
//! implements `DevTarget`, and `craft dev` uses that.
//!
//! The watcher is `notify` events plus a periodic rescan. A rescan compares each file's modified
//! time and length with what the last scan saw, so a missed event, a network folder with no
//! inotify, or a half-written file that is finished later all end in a correct reload.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::time::{Duration, SystemTime};

use notify::{RecursiveMode, Watcher};

use crate::error::{CliError, Result, io_at};
use crate::run::Installed;

/// What the watcher needs from a plugin host.
pub trait DevTarget {
    /// Install or replace the plugin in `bytes`. Returns what the host loaded.
    fn install(&self, bytes: &[u8]) -> Result<Installed>;
    /// Remove an installed plugin.
    fn remove(&self, plugin_id: &str) -> Result<()>;
}

#[derive(Debug, PartialEq, Eq)]
pub enum DevEvent {
    /// The file loaded. `installed.replaced` is the generation it took over from, if any.
    Loaded { path: PathBuf, installed: Installed },
    /// The file did not load. The previous version of that plugin, if any, keeps running.
    Rejected { path: PathBuf, reason: String },
    /// The file is gone, and the plugin it carried was removed.
    Removed { path: PathBuf, plugin_id: String },
    /// The file is gone, and removing its plugin failed.
    RemoveFailed { path: PathBuf, plugin_id: String, reason: String },
}

#[derive(Clone, Copy, PartialEq, Eq)]
struct Stamp {
    modified: SystemTime,
    len: u64,
}

struct Known {
    stamp: Stamp,
    /// Set once the file loaded.
    plugin_id: Option<String>,
}

pub struct DevFolder<T: DevTarget> {
    dir: PathBuf,
    target: T,
    known: BTreeMap<PathBuf, Known>,
}

impl<T: DevTarget> DevFolder<T> {
    pub fn new(dir: impl Into<PathBuf>, target: T) -> Result<DevFolder<T>> {
        let dir = dir.into();
        if !dir.is_dir() {
            return Err(CliError::new(format!("{} is not a directory", dir.display())));
        }
        Ok(DevFolder { dir, target, known: BTreeMap::new() })
    }

    pub fn target(&self) -> &T {
        &self.target
    }

    fn wasm_files(&self) -> Result<Vec<(PathBuf, Stamp)>> {
        let mut files = Vec::new();
        for entry in std::fs::read_dir(&self.dir).map_err(|e| io_at(&self.dir, e))? {
            let entry = entry.map_err(|e| io_at(&self.dir, e))?;
            let path = entry.path();
            if path.extension().is_none_or(|e| e != "wasm") {
                continue;
            }
            // A file can vanish between the listing and the stat. That is a removal, seen on the next scan.
            let Ok(meta) = entry.metadata() else { continue };
            if !meta.is_file() {
                continue;
            }
            let modified = meta.modified().map_err(|e| io_at(&path, e))?;
            files.push((path, Stamp { modified, len: meta.len() }));
        }
        files.sort_by(|a, b| a.0.cmp(&b.0));
        Ok(files)
    }

    /// Load every new or changed file and remove the plugins of deleted files.
    pub fn scan(&mut self) -> Result<Vec<DevEvent>> {
        let mut events = Vec::new();
        let files = self.wasm_files()?;
        for (path, stamp) in &files {
            if self.known.get(path).is_some_and(|k| k.stamp == *stamp) {
                continue;
            }
            let previous_id = self.known.get(path).and_then(|k| k.plugin_id.clone());
            match std::fs::read(path).map_err(|e| io_at(path, e)).and_then(|bytes| self.target.install(&bytes)) {
                Ok(installed) => {
                    self.known.insert(path.clone(), Known { stamp: *stamp, plugin_id: Some(installed.plugin_id.clone()) });
                    events.push(DevEvent::Loaded { path: path.clone(), installed });
                }
                Err(e) => {
                    // Keep the stamp: the same bytes are not retried every scan. The next save changes it.
                    self.known.insert(path.clone(), Known { stamp: *stamp, plugin_id: previous_id });
                    events.push(DevEvent::Rejected { path: path.clone(), reason: e.to_string() });
                }
            }
        }
        let present: Vec<&PathBuf> = files.iter().map(|(p, _)| p).collect();
        let gone: Vec<PathBuf> = self.known.keys().filter(|p| !present.contains(p)).cloned().collect();
        for path in gone {
            let Some(known) = self.known.remove(&path) else { continue };
            if let Some(plugin_id) = known.plugin_id {
                events.push(match self.target.remove(&plugin_id) {
                    Ok(()) => DevEvent::Removed { path, plugin_id },
                    Err(e) => DevEvent::RemoveFailed { path, plugin_id, reason: e.to_string() },
                });
            }
        }
        Ok(events)
    }

    /// Scan now, then again on every file system event and every `poll`, until `stop` is set.
    /// `on_events` gets each non-empty batch.
    pub fn watch(&mut self, stop: &AtomicBool, poll: Duration, mut on_events: impl FnMut(Vec<DevEvent>)) -> Result<()> {
        let (tx, rx) = mpsc::channel();
        let mut watcher = notify::recommended_watcher(move |event: notify::Result<notify::Event>| {
            // The receiver is gone only when `watch` returned, so a failed send loses nothing.
            let _ = tx.send(event);
        })
        .map_err(|e| CliError::new(format!("cannot start the file watcher: {e}")))?;
        watcher.watch(&self.dir, RecursiveMode::NonRecursive).map_err(|e| CliError::new(format!("cannot watch {}: {e}", self.dir.display())))?;
        loop {
            let events = self.scan()?;
            if !events.is_empty() {
                on_events(events);
            }
            if stop.load(Ordering::Relaxed) {
                return Ok(());
            }
            match rx.recv_timeout(poll) {
                Ok(Err(e)) => return Err(CliError::new(format!("file watcher: {e}"))),
                Ok(Ok(_)) | Err(mpsc::RecvTimeoutError::Timeout) => {}
                Err(mpsc::RecvTimeoutError::Disconnected) => return Err(CliError::new("file watcher stopped")),
            }
            // Let a burst of writes settle, so a half-written module is read once, whole.
            std::thread::sleep(Duration::from_millis(60));
            while rx.try_recv().is_ok() {}
        }
    }
}

/// Copy `from` into `folder` under its own file name. The file appears whole: it is written under a
/// name the watcher ignores, then renamed.
pub fn publish_atomic(from: &Path, folder: &Path) -> Result<PathBuf> {
    let name = from.file_name().ok_or_else(|| CliError::new(format!("{} has no file name", from.display())))?;
    let dest = folder.join(name);
    let tmp = folder.join(format!(".{}.partial", name.to_string_lossy()));
    std::fs::copy(from, &tmp).map_err(|e| io_at(&tmp, e))?;
    std::fs::rename(&tmp, &dest).map_err(|e| io_at(&dest, e))?;
    Ok(dest)
}
