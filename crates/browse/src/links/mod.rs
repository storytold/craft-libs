//! `LinkMonitor` and `LinkChange`: link statuses kept current through `FileWatch`, recomputed in
//! jobs and reported once per change; `SearchRun`: a relink search run as a cancellable job.
//!
//! ```ignore
//! // App state: one monitor per document.
//! let mut monitor = LinkMonitor::new(Arc::new(SystemLinkFiles), host.jobs_shared().clone(), wake);
//! // Each pass: the document's links, then what changed since the last pass.
//! monitor.set(&doc.links(), doc.path());
//! for change in monitor.take_changes() {
//!     if change.status == LinkStatus::Modified { prefs.update_links.act(change.key) }   // Automatically, Ask, Manually
//! }
//! LinkList::new("links", &links, monitor.statuses(), &mut state.links).show(ui);
//! ```

#[cfg(test)]
mod tests;

use std::collections::HashMap;
use std::hash::Hash;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, PoisonError};

use craft_docpath::links::{Candidate, Link, LinkFiles, LinkSearch, LinkStatus, SearchEnd, SearchUpdate};
use craft_host::queue::{Cancel, JobQueue, Priority, QueueOptions};
use craft_host::{Jobs, Wake};
use craft_platform::watch::FileWatch;

/// A link whose status changed, reported once.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LinkChange<K> {
    /// The link.
    pub key: K,
    /// Its status before; `None` the first time it was known.
    pub before: Option<LinkStatus>,
    /// Its status now.
    pub status: LinkStatus,
}

/// Keeps the statuses of a document's links current without a timed poll (lead call 39).
///
/// - The resolved path of every link that is not embedded or refused is watched with a
///   `craft_platform::watch::FileWatch` (`ReadPolicy::Stamp`, which reads no bytes) whose wake
///   is the monitor's `wake`.
/// - A link is checked with `craft_docpath::links::status` in a job on a `JobQueue` (keyed by the
///   link, `Priority::BACKGROUND`, lanes from `QueueOptions::default`, name `"links"`) when it is
///   first [`set`](Self::set), when its stored path or identity changes, when the watch reports
///   its file, and for every link on [`window_focused`](Self::window_focused).
/// - A finished check whose status differs from the known one is a [`LinkChange`], reported once
///   by [`take_changes`](Self::take_changes); an unchanged one reports nothing.
///
/// Replaces VectorCraft's 2 s scan (`link_watch.rs`) and DesignCraft's text-link checks. On wasm32
/// nothing is watched (`FileWatch` has no paths there) and statuses come from the first check.
pub struct LinkMonitor<K> {
    files: Arc<dyn LinkFiles>,
    queue: JobQueue<K, (u64, LinkStatus)>,
    watch: FileWatch<K>,
    links: HashMap<K, Link<K>>,
    document: Option<PathBuf>,
    statuses: HashMap<K, LinkStatus>,
    changes: Vec<LinkChange<K>>,
    /// The generation of the latest check queued per link; a result of an older one is dropped.
    generations: HashMap<K, u64>,
    /// The last generation handed out; never reused, so a forgotten and re-added link can't match an old result.
    last_generation: u64,
}

/// The file a link points at, when it has one to watch.
fn watched_path<K>(link: &Link<K>, document: Option<&Path>) -> Option<PathBuf> {
    if link.embedded {
        return None;
    }
    craft_docpath::resolve_absolute(&link.stored, document).ok()
}

impl<K: Clone + Eq + Hash + Send + Sync + 'static> LinkMonitor<K> {
    /// A monitor reading files through `files`, running checks on `jobs`, calling `wake` when a
    /// check ends or the watch sees a change.
    pub fn new(files: Arc<dyn LinkFiles>, jobs: Arc<dyn Jobs>, wake: Wake) -> Self {
        // vectorcraft: crates/engine/src/link_watch.rs:66 (`start_link_scan`: stamps on a worker thread).
        let watch = FileWatch::new(wake.clone());
        let queue = JobQueue::new(jobs, wake, QueueOptions { name: "links", ..QueueOptions::default() });
        Self {
            files,
            queue,
            watch,
            links: HashMap::new(),
            document: None,
            statuses: HashMap::new(),
            changes: Vec::new(),
            generations: HashMap::new(),
            last_generation: 0,
        }
    }

    /// The links to keep current, with the document's path (`None` for an unsaved document):
    /// new and changed links are checked, links left out are forgotten (their statuses and
    /// watches dropped). Allocates nothing when the links are the ones set last.
    pub fn set(&mut self, links: &[Link<K>], document: Option<&Path>) {
        // vectorcraft: crates/engine/src/link_watch.rs:50 (`linked_files`, the files watched per document).
        if self.document.as_deref() == document && self.links.len() == links.len() && links.iter().all(|l| self.links.get(&l.key) == Some(l)) {
            return;
        }
        let document_changed = self.document.as_deref() != document;
        if document_changed {
            self.document = document.map(Path::to_path_buf);
        }
        let kept: std::collections::HashSet<&K> = links.iter().map(|l| &l.key).collect();
        let gone: Vec<K> = self.links.keys().filter(|k| !kept.contains(k)).cloned().collect();
        for key in gone {
            self.links.remove(&key);
            self.statuses.remove(&key);
            self.generations.remove(&key);
            self.watch.forget(&key);
        }
        for link in links {
            let moved = match self.links.get(&link.key) {
                Some(old) => document_changed || old.stored != link.stored || old.identity != link.identity || old.embedded != link.embedded,
                None => true,
            };
            if moved {
                self.links.insert(link.key.clone(), link.clone());
                self.check(link.clone());
            } else if let Some(old) = self.links.get_mut(&link.key) {
                old.clone_from(link);
            }
        }
        let document = self.document.clone();
        self.watch.set(links.iter().filter_map(|l| Some((l.key.clone(), watched_path(l, document.as_deref())?))));
    }

    /// The window came back to the front: every link is checked again.
    pub fn window_focused(&mut self) {
        // vectorcraft: crates/engine/src/link_watch.rs:9 (looked at again when the window comes back to the front).
        for link in self.links.values().cloned().collect::<Vec<_>>() {
            self.check(link);
        }
    }

    /// The known statuses (links not checked yet are absent), for `LinkList`.
    pub fn statuses(&self) -> &HashMap<K, LinkStatus> {
        // new: the list reads statuses by key.
        &self.statuses
    }

    /// The status of one link, once known.
    pub fn status(&self, key: &K) -> Option<LinkStatus> {
        // new: a lookup for the host's commands (Update enabled when Modified).
        self.statuses.get(key).copied()
    }

    /// Drains the watch's changes into checks and the finished checks into statuses, and returns
    /// the changes since the last call, oldest first. Call it once per pass.
    pub fn take_changes(&mut self) -> Vec<LinkChange<K>> {
        // vectorcraft: crates/engine/src/link_watch.rs:89 (`poll_link_scan`; a change is acted on once).
        for changed in self.watch.take_changes() {
            if let Some(link) = self.links.get(&changed.key).cloned() {
                self.check(link);
            }
        }
        while let Some(done) = self.queue.try_recv() {
            let (generation, status) = done.result;
            if !self.links.contains_key(&done.key) || self.generations.get(&done.key) != Some(&generation) {
                continue;
            }
            let before = self.statuses.insert(done.key.clone(), status);
            if before != Some(status) {
                self.changes.push(LinkChange { key: done.key, before, status });
            }
        }
        std::mem::take(&mut self.changes)
    }

    /// The app just relinked, updated or embedded `key`: its status is checked again and the
    /// result is reported as a change only if it differs from what the app now expects
    /// (`expected`).
    pub fn known_now(&mut self, key: &K, expected: LinkStatus) {
        // vectorcraft: crates/engine/src/link_watch.rs:16 (undoing an update leaves the link modified until the file changes).
        let Some(link) = self.links.get(key).cloned() else { return };
        if let Some(path) = watched_path(&link, self.document.as_deref()) {
            self.watch.known_now(key.clone(), &path);
        }
        self.statuses.insert(key.clone(), expected);
        self.check(link);
    }

    /// Queues the status check of `link`, replacing a waiting one; it becomes the link's latest
    /// generation, so the result of a check that started earlier is dropped.
    fn check(&mut self, link: Link<K>) {
        self.last_generation += 1;
        let generation = self.last_generation;
        self.generations.insert(link.key.clone(), generation);
        let files = Arc::clone(&self.files);
        let document = self.document.clone();
        self.queue
            .submit(link.key.clone(), Priority::BACKGROUND, move |_| (generation, craft_docpath::links::status(&link, document.as_deref(), &*files)));
    }
}

/// The app's check of a search candidate: returns it with its problems added and `ok` and `score`
/// adjusted (`LinkSearch::run`'s `checker`).
pub type LinkChecker<K> = dyn Fn(&Link<K>, Candidate) -> Candidate + Send + Sync;

/// A [`LinkSearch`] running as one cancellable job on a `JobQueue` (one lane, name
/// `"relink-search"`) over the app's `Jobs`. The job reports through a shared inbox and calls
/// `wake` after each `Progress` and each `Hit`; the dialog drains it once per pass. Dropping the
/// run cancels it.
pub struct SearchRun<K> {
    queue: JobQueue<u64, SearchEnd>,
    updates: Arc<Mutex<Vec<SearchUpdate<K>>>>,
    cancel: Cancel,
    end: Option<SearchEnd>,
}

impl<K: Clone + Send + 'static> SearchRun<K> {
    /// Starts `search` for `links` (the missing ones) over `files`; `checker` is the app's own
    /// check of each candidate, run in the job before the hits are ranked.
    pub fn start(
        search: LinkSearch,
        links: Vec<Link<K>>,
        files: Arc<dyn LinkFiles>,
        checker: Arc<LinkChecker<K>>,
        jobs: Arc<dyn Jobs>,
        wake: Wake,
    ) -> Self {
        // filmcraft: crates/ui-egui/src/panels/media_dialogs.rs:146 (`link_media` ran `media.search` on the UI thread).
        let queue = JobQueue::new(jobs, wake.clone(), QueueOptions { lanes: 1, name: "relink-search" });
        let updates = Arc::new(Mutex::new(Vec::new()));
        let cancel = Cancel::new();
        let inbox = Arc::clone(&updates);
        let own = cancel.clone();
        queue.submit(0, Priority::VISIBLE, move |queue_cancel| {
            let cancelled = || own.is_cancelled() || queue_cancel.is_cancelled();
            search.run(&links, &*files, &*checker, &cancelled, &mut |update| {
                inbox.lock().unwrap_or_else(PoisonError::into_inner).push(update);
                wake.wake();
            })
        });
        Self { queue, updates, cancel, end: None }
    }

    /// The updates since the last call, in order; empty once the run is cancelled.
    pub fn take_updates(&mut self) -> Vec<SearchUpdate<K>> {
        // new: the dialog fills its candidate list from these.
        if self.cancel.is_cancelled() {
            self.updates.lock().unwrap_or_else(PoisonError::into_inner).clear();
            return Vec::new();
        }
        std::mem::take(&mut *self.updates.lock().unwrap_or_else(PoisonError::into_inner))
    }

    /// How the search ended; `None` while it runs.
    pub fn end(&mut self) -> Option<SearchEnd> {
        // new: the dialog swaps Cancel back to Search.
        if self.end.is_none() {
            self.end = self.queue.try_recv().map(|done| done.result);
        }
        self.end
    }

    /// Stops the search: the job sees the cancel between folders and checks, and nothing more is
    /// reported.
    pub fn cancel(&mut self) {
        // new: the dialog's Cancel and Esc.
        self.end();
        self.end.get_or_insert(SearchEnd::Cancelled);
        self.cancel.cancel();
        self.updates.lock().unwrap_or_else(PoisonError::into_inner).clear();
    }
}

impl<K> Drop for SearchRun<K> {
    fn drop(&mut self) {
        self.cancel.cancel();
    }
}
