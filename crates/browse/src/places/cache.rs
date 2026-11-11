//! `FolderCache`: roots and subfolder listings made in jobs, kept until they go stale.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use craft_host::queue::{JobQueue, Priority, QueueOptions};
use craft_host::{Jobs, Wake};

use super::{Directories, Folder, StandardFolder, Volume};

/// The standard folders and volumes, from one job.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Roots {
    /// [`Directories::standard`].
    pub standard: Vec<(StandardFolder, PathBuf)>,
    /// [`Directories::volumes`].
    pub volumes: Vec<Volume>,
}

/// One folder's subfolders as the cache has them.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Listing<'a> {
    /// Not listed yet; a job is queued.
    Loading,
    /// The subfolders. While a new listing of a stale folder runs, the old one stays here.
    Ready(&'a [Folder]),
    /// The folder couldn't be listed; the system's message.
    Failed(&'a str),
}

/// What a listing job returns.
enum Found {
    Roots(Roots),
    Subfolders(Result<Vec<Folder>, String>),
    Unlisted(Vec<PathBuf>),
}

/// Which listing a job makes.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
enum Job {
    Roots,
    Subfolders(PathBuf),
    Unlisted(PathBuf),
}

/// Folder listings for a places tree, made in jobs on a [`JobQueue`] (lanes from
/// `QueueOptions::default`, name `"folders"`) over the app's `Jobs`; each finished job calls
/// `wake`. Nothing is listed in the pass: an unknown folder answers `Loading` and queues a job
/// (`Priority::VISIBLE` for asks, `NEAR` for refreshes), and the result shows in the first pass
/// after it lands. Lives in the app's state, one per tree.
///
/// Refreshing (risk 8): `craft_platform::watch::FileWatch` watches regular files only
/// (`FileStamp::at` filters `is_file`), so folders are not watched. Instead every listing goes
/// stale on [`refresh`](Self::refresh), which `PlacesTree` calls when the window regains focus,
/// and on [`refresh_folder`](Self::refresh_folder) (a row opened, the app made or moved a folder).
/// A stale listing keeps showing and is listed again the next time it is asked for. There is no
/// timed refresh.
pub struct FolderCache {
    directories: Arc<dyn Directories>,
    queue: JobQueue<Job, (u64, Found)>,
    roots: Option<Roots>,
    listings: HashMap<PathBuf, (Result<Vec<Folder>, String>, bool)>,
    unlisted: HashMap<PathBuf, Vec<PathBuf>>,
    stale_roots: bool,
    /// What this pass asked for, for [`finish_pass`](Self::finish_pass).
    asked: HashSet<Job>,
    /// Per listing, the generation a refresh last asked for. A job carries the generation current
    /// when it was queued; a result of an older generation started before the refresh, so it
    /// shows but stays stale and is listed again.
    generations: HashMap<Job, u64>,
}

impl FolderCache {
    /// A cache over `directories`, running its jobs on `jobs` and calling `wake` when one ends.
    pub fn new(directories: Arc<dyn Directories>, jobs: Arc<dyn Jobs>, wake: Wake) -> Self {
        // lightcraft: crates/ui-egui/src/panels/left/local.rs:241 (`fs_cached`: listings on a worker, read in the pass).
        let options = QueueOptions { name: "folders", ..QueueOptions::default() };
        Self {
            directories,
            queue: JobQueue::new(jobs, wake, options),
            roots: None,
            listings: HashMap::new(),
            unlisted: HashMap::new(),
            stale_roots: false,
            asked: HashSet::new(),
            generations: HashMap::new(),
        }
    }

    /// The roots, once listed; queues the job the first time and after a refresh.
    pub fn roots(&mut self) -> Option<&Roots> {
        // lightcraft: crates/ui-egui/src/panels/left/local.rs:28 (the is-dir checks run off the UI thread).
        self.drain();
        self.asked.insert(Job::Roots);
        if self.roots.is_none() || self.stale_roots {
            let priority = if self.roots.is_none() { Priority::VISIBLE } else { Priority::NEAR };
            self.submit(Job::Roots, priority);
        }
        self.roots.as_ref()
    }

    /// `folder`'s subfolders (drains finished jobs first).
    pub fn subfolders(&mut self, folder: &Path) -> Listing<'_> {
        // lightcraft: crates/ui-egui/src/panels/left/local.rs:207 (`subfolders`, empty until the first listing).
        self.drain();
        let job = Job::Subfolders(folder.to_path_buf());
        self.asked.insert(job.clone());
        match self.listings.get(folder).map(|(_, stale)| *stale) {
            None => self.submit(job, Priority::VISIBLE),
            Some(true) => self.submit(job, Priority::NEAR),
            Some(false) => {}
        }
        match self.listings.get(folder) {
            None => Listing::Loading,
            Some((Ok(folders), _)) => Listing::Ready(folders),
            Some((Err(message), _)) => Listing::Failed(message),
        }
    }

    /// The hidden folders on the way to `path` ([`Directories::unlisted_ancestors`]); `None` until
    /// listed.
    pub fn unlisted(&mut self, path: &Path) -> Option<&[PathBuf]> {
        // lightcraft: crates/ui-egui/src/panels/left/local.rs:38 (`unlisted` through `fs_cached`).
        self.drain();
        let job = Job::Unlisted(path.to_path_buf());
        self.asked.insert(job.clone());
        if !self.unlisted.contains_key(path) {
            self.submit(job, Priority::VISIBLE);
        }
        self.unlisted.get(path).map(Vec::as_slice)
    }

    /// Marks every listing and the roots stale (the window regained focus).
    pub fn refresh(&mut self) {
        // new: replaces LightCraft's 2 s refresh (lead call 26).
        self.stale_roots = true;
        for generation in self.generations.values_mut() {
            *generation += 1;
        }
        for (_, stale) in self.listings.values_mut() {
            *stale = true;
        }
    }

    /// Marks one folder's listing stale.
    pub fn refresh_folder(&mut self, folder: &Path) {
        // new: the tree calls it when a row opens.
        if let Some(generation) = self.generations.get_mut(&Job::Subfolders(folder.to_path_buf())) {
            *generation += 1;
        }
        if let Some((_, stale)) = self.listings.get_mut(folder) {
            *stale = true;
        }
    }

    /// Drops the listings of folders not asked for since the last call, and cancels their jobs
    /// (`JobQueue::reprioritize`). `PlacesTree` calls it at the end of its pass.
    pub fn finish_pass(&mut self) {
        // new: a hidden tree lists nothing.
        let asked = std::mem::take(&mut self.asked);
        self.listings.retain(|path, _| asked.contains(&Job::Subfolders(path.clone())));
        self.unlisted.retain(|path, _| asked.contains(&Job::Unlisted(path.clone())));
        self.queue.reprioritize(|job, priority| asked.contains(job).then_some(priority));
        self.generations.retain(|job, _| *job == Job::Roots || asked.contains(job));
    }

    /// Queues `job` unless one for it is waiting or running.
    fn submit(&mut self, job: Job, priority: Priority) {
        if self.queue.contains(&job) {
            return;
        }
        let directories = Arc::clone(&self.directories);
        let key = job.clone();
        let generation = *self.generations.entry(job.clone()).or_insert(0);
        self.queue.submit(key, priority, move |cancel| {
            let found = match job {
                Job::Roots => Found::Roots(Roots { standard: directories.standard(), volumes: directories.volumes() }),
                Job::Subfolders(folder) => Found::Subfolders(directories.subfolders(&folder, cancel).map_err(|e| e.to_string())),
                Job::Unlisted(path) => Found::Unlisted(directories.unlisted_ancestors(&path)),
            };
            (generation, found)
        });
    }

    /// Takes in the jobs that finished since the last call.
    fn drain(&mut self) {
        // Without lanes (wasm32) the jobs run here, one per pass.
        if QueueOptions::default_lanes() == 0 {
            self.queue.run_inline(1);
        }
        while let Some(done) = self.queue.try_recv() {
            let (generation, found) = done.result;
            let current = self.generations.get(&done.key).is_none_or(|&latest| latest == generation);
            match (done.key, found) {
                (Job::Roots, Found::Roots(roots)) => {
                    self.roots = Some(roots);
                    self.stale_roots = !current;
                }
                (Job::Subfolders(folder), Found::Subfolders(result)) => {
                    self.listings.insert(folder, (result, !current));
                }
                (Job::Unlisted(path), Found::Unlisted(found)) => {
                    self.unlisted.insert(path, found);
                }
                _ => {}
            }
        }
    }
}
