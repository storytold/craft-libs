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

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::time::{Duration, SystemTime};

use notify::{RecursiveMode, Watcher};

use crate::error::{CliError, Result, io_at};
use crate::gate;
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
    /// The plugin this path runs. Set once the file loaded, and kept when a later save is rejected,
    /// because the host keeps the previous version running.
    plugin_id: Option<String>,
    /// The plugin id this file asks for while another path owns it. The file is read again on every
    /// scan, so it loads as soon as the owner is gone.
    waiting_for: Option<String>,
}

/// One present file during a scan.
struct Candidate {
    path: PathBuf,
    stamp: Stamp,
    /// The plugin this path ran before this scan.
    previous: Option<String>,
    /// What to do with the file this scan.
    work: Work,
    /// The plugin id the file's manifest asks for, when the gate can read one.
    wanted: Option<String>,
    /// The plugin id this path owns after the scan, before any install result: what the file
    /// asks for, else what the path ran before.
    claim: Option<String>,
    /// Set when another path owns the id this file asks for. The path keeps its previous plugin.
    conflict: Option<String>,
}

enum Work {
    /// The file and its stamp are as the last scan left them.
    Keep,
    /// Install these bytes. `announce` is false when the file only waits for an id to free up again.
    Install { bytes: Vec<u8>, announce: bool },
    /// The file could not be read.
    Unreadable(String),
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

    /// Load every new or changed file and remove the plugins nobody owns any more.
    ///
    /// The scan first works out which path owns which plugin id, from the whole folder. Then it
    /// installs, and last it removes the ids that have no owner path left. So a rename that keeps
    /// the id, or a save that changes the id, never removes a plugin that a path still owns. Paths
    /// are visited in sorted order, so the result does not depend on directory order.
    ///
    /// Two paths cannot own one id. A path that already owns the id keeps it. Otherwise the earlier
    /// path wins, and the later file is rejected until the id is free.
    pub fn scan(&mut self) -> Result<Vec<DevEvent>> {
        let mut candidates = self.candidates()?;
        resolve_claims(&mut candidates);
        let mut events = Vec::new();
        let owned_before: Vec<(PathBuf, String)> = self.known.iter().filter_map(|(p, k)| Some((p.clone(), k.plugin_id.clone()?))).collect();

        for c in &candidates {
            let Candidate { path, stamp, previous, work, wanted, claim, conflict } = c;
            let (path, stamp) = (path.clone(), *stamp);
            if let Some(reason) = conflict {
                // The loser keeps the plugin it ran before, if any. It waits for the id to free up.
                self.known.insert(path.clone(), Known { stamp, plugin_id: claim.clone(), waiting_for: wanted.clone() });
                if matches!(work, Work::Install { announce: true, .. }) {
                    events.push(DevEvent::Rejected { path, reason: reason.clone() });
                }
                continue;
            }
            match work {
                Work::Keep => {}
                Work::Unreadable(reason) => {
                    self.known.insert(path.clone(), Known { stamp, plugin_id: previous.clone(), waiting_for: None });
                    events.push(DevEvent::Rejected { path, reason: reason.clone() });
                }
                Work::Install { bytes, .. } => match self.target.install(bytes) {
                    Ok(installed) => {
                        self.known.insert(path.clone(), Known { stamp, plugin_id: Some(installed.plugin_id.clone()), waiting_for: None });
                        events.push(DevEvent::Loaded { path, installed });
                    }
                    Err(e) => {
                        // Keep the stamp: the same bytes are not retried every scan. The next save changes it.
                        // The previous plugin keeps running, unless another path took its id in this scan.
                        let kept = previous.clone().filter(|id| !candidates.iter().any(|o| o.path != path && o.claim.as_ref() == Some(id)));
                        self.known.insert(path.clone(), Known { stamp, plugin_id: kept, waiting_for: None });
                        events.push(DevEvent::Rejected { path, reason: e.to_string() });
                    }
                },
            }
        }

        let present: BTreeSet<&Path> = candidates.iter().map(|c| c.path.as_path()).collect();
        self.known.retain(|p, _| present.contains(p.as_path()));
        let live: BTreeSet<&str> = self.known.values().filter_map(|k| k.plugin_id.as_deref()).collect();
        let mut removed: BTreeSet<&str> = BTreeSet::new();
        for (path, plugin_id) in &owned_before {
            if live.contains(plugin_id.as_str()) || !removed.insert(plugin_id.as_str()) {
                continue;
            }
            let (path, plugin_id) = (path.clone(), plugin_id.clone());
            events.push(match self.target.remove(&plugin_id) {
                Ok(()) => DevEvent::Removed { path, plugin_id },
                Err(e) => DevEvent::RemoveFailed { path, plugin_id, reason: e.to_string() },
            });
        }
        Ok(events)
    }

    /// Every `.wasm` file in sorted path order, with its bytes when it needs an install.
    fn candidates(&self) -> Result<Vec<Candidate>> {
        let mut out = Vec::new();
        for (path, stamp) in self.wasm_files()? {
            let known = self.known.get(&path);
            let previous = known.and_then(|k| k.plugin_id.clone());
            let changed = known.is_none_or(|k| k.stamp != stamp);
            let waiting = known.is_some_and(|k| k.waiting_for.is_some());
            let work = if !changed && !waiting {
                Work::Keep
            } else {
                match std::fs::read(&path) {
                    Ok(bytes) => Work::Install { bytes, announce: changed },
                    Err(e) if changed => Work::Unreadable(io_at(&path, e).to_string()),
                    Err(_) => Work::Keep,
                }
            };
            // A file the gate cannot read has no id to claim. It still goes to the host, which says why it fails.
            let wanted = match &work {
                Work::Install { bytes, .. } => gate::check(bytes).manifest.map(|m| m.id),
                Work::Keep | Work::Unreadable(_) => None,
            };
            let claim = wanted.clone().or_else(|| previous.clone());
            out.push(Candidate { path, stamp, previous, work, claim, wanted, conflict: None });
        }
        Ok(out)
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

/// Decide which path owns which plugin id. A path that owned the id before keeps it, then the
/// earlier path wins. A loser gets a `conflict` and falls back to the plugin it ran before. That
/// fallback is a claim too, so the loop runs again until no id has two claimants. Each pass moves
/// at least one claim from its new id to its previous id, or from its previous id to none, so it ends.
fn resolve_claims(candidates: &mut [Candidate]) {
    loop {
        let mut order: Vec<usize> = (0..candidates.len()).filter(|&i| candidates[i].claim.is_some()).collect();
        // `candidates` is in path order, so the index breaks ties by path.
        order.sort_by_key(|&i| (candidates[i].claim != candidates[i].previous, i));
        let mut owner: BTreeMap<String, usize> = BTreeMap::new();
        let mut losers: Vec<(usize, usize)> = Vec::new();
        for i in order {
            let Some(id) = candidates[i].claim.clone() else { continue };
            match owner.get(&id) {
                Some(&winner) => losers.push((i, winner)),
                None => {
                    owner.insert(id, i);
                }
            }
        }
        if losers.is_empty() {
            return;
        }
        for (i, winner) in losers {
            let id = candidates[i].claim.take().unwrap_or_default();
            let owner_path = candidates[winner].path.display().to_string();
            // Only the id the file asks for falls back. A second loss ends in no claim.
            let fallback = candidates[i].previous.clone().filter(|prev| candidates[i].wanted.as_ref() == Some(&id) && *prev != id);
            candidates[i].conflict.get_or_insert_with(|| format!("plugin id {id} is already provided by {owner_path}"));
            candidates[i].claim = fallback;
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
