//! Installs, removals, preview downloads and copies to the OS user font folder, one at a time on
//! a worker thread.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::mpsc::{Receiver, Sender, channel};

use crate::Catalog;
use crate::index::Index;
use crate::store::{Conflict, Downloaded, Installer, MadeAvailable, Progress, RemoveRequest, StoreError};

type Task = Box<dyn FnOnce() + Send>;

/// Where jobs run.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Runner {
    /// On a worker thread.
    Thread,
    /// Right away, on the calling thread (tests).
    Inline,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Install,
    Remove,
    Preview,
    /// Copying to or withdrawing from the OS user font folder: a family's switch, or the start-up
    /// copy, which has no family.
    SystemFonts,
}

pub struct Running {
    /// The family's source; empty for a removal, which goes by the installed files.
    pub source: String,
    pub family: String,
    pub kind: Kind,
    /// One file's index path or store path, when the job is for one style.
    pub file: Option<String>,
    /// An install's choice of the OS user font folder (see [`Request::Install`]).
    pub share: Option<bool>,
    pub progress: Progress,
}

impl Running {
    /// How far along, when the size is known.
    pub fn fraction(&self) -> Option<f32> {
        (self.progress.total > 0).then(|| (self.progress.done as f32 / self.progress.total as f32).clamp(0.0, 1.0))
    }
}

pub enum Finished {
    /// With what copying the family into the OS user font folder did, when it was copied.
    Installed(Option<Result<MadeAvailable, StoreError>>),
    Removed,
    Previewed(Vec<Downloaded>),
    /// What a family's switch did: copied it, with the outcome, or withdrew it (`None`).
    Switched(Option<MadeAvailable>),
    /// The families a conflict kept out of the OS user font folder at start.
    SharedAtStart(Vec<(String, Vec<Conflict>)>),
}

enum Msg {
    Progress(u64, Progress),
    Done(u64, Result<Finished, StoreError>),
}

pub enum Request {
    /// A family of the index, by position, and whether to copy it into the OS user font folder
    /// (and record that as its choice); `None` keeps its recorded choice, else takes the default.
    Install(Arc<Index>, usize, Option<bool>),
    /// One file of a family of the index, by its path in the index.
    InstallFile(Arc<Index>, usize, String),
    Remove(RemoveRequest),
    /// A family of the index, by position.
    Preview(Arc<Index>, usize),
    /// Records a store family's choice, by name and store files, then copies it into the OS user
    /// font folder (`true`) or withdraws it.
    AvailableToAll(String, Vec<PathBuf>, bool),
    /// Copies the store families whose choice is the OS user font folder and that aren't all
    /// there yet: those installed before installs copied, or whose copy failed.
    ShareAtStart,
}

pub struct Jobs {
    worker: Option<Sender<Task>>,
    tx: Sender<Msg>,
    rx: Receiver<Msg>,
    next: u64,
    running: BTreeMap<u64, Running>,
}

impl Jobs {
    pub fn new(runner: Runner) -> Jobs {
        let (tx, rx) = channel();
        let worker = match runner {
            Runner::Inline => None,
            Runner::Thread => {
                let (worker, tasks) = channel::<Task>();
                let spawned = std::thread::Builder::new().name("font-jobs".into()).spawn(move || {
                    for task in tasks {
                        task();
                    }
                });
                if let Err(e) = &spawned {
                    log::warn!("no worker thread ({e}); font jobs run on the interface thread");
                }
                spawned.ok().map(|_| worker)
            }
        };
        Jobs { worker, tx, rx, next: 0, running: BTreeMap::new() }
    }

    /// The job running for `family` of `source` (any source when empty), if any.
    pub fn of(&self, source: &str, family: &str) -> Option<&Running> {
        self.running.values().find(|r| r.family.eq_ignore_ascii_case(family) && (source.is_empty() || r.source.is_empty() || r.source == source))
    }

    /// Runs `request` through `installer`, then rescans `catalog` after an install or removal. An
    /// install also records the family's choice and copies it into the OS user font folder when
    /// that is the choice.
    pub fn start(&mut self, ctx: &egui::Context, installer: Arc<dyn Installer>, catalog: Arc<Catalog>, request: Request) {
        let id = self.next;
        self.next += 1;
        let share = if let Request::Install(_, _, share) = &request { *share } else { None };
        let (source, family, kind, file, total) = match &request {
            Request::Install(index, i, _) | Request::Preview(index, i) => {
                let Some(f) = index.families().get(*i) else { return };
                let kind = if matches!(request, Request::Install(..)) { Kind::Install } else { Kind::Preview };
                (f.source.clone(), f.family.clone(), kind, None, f.size())
            }
            Request::InstallFile(index, i, path) => {
                let Some(f) = index.families().get(*i) else { return };
                let size = f.files.iter().find(|x| &x.path == path).map_or(0, |x| x.size);
                (f.source.clone(), f.family.clone(), Kind::Install, Some(path.clone()), size)
            }
            Request::Remove(r) => {
                let file = if r.whole { None } else { r.files.first().map(|p| p.display().to_string()) };
                (String::new(), r.family.clone(), Kind::Remove, file, 0)
            }
            Request::AvailableToAll(family, ..) => (String::new(), family.clone(), Kind::SystemFonts, None, 0),
            Request::ShareAtStart => (String::new(), String::new(), Kind::SystemFonts, None, 0),
        };
        self.running.insert(id, Running { source, family, kind, file, share, progress: Progress { done: 0, total } });
        let tx = self.tx.clone();
        let ctx = ctx.clone();
        let task: Task = Box::new(move || {
            let mut progress = |p: Progress| {
                let _ = tx.send(Msg::Progress(id, p));
                ctx.request_repaint();
            };
            let family = |index: &Index, i: usize| index.families().get(i).cloned().ok_or_else(|| StoreError("no such family".into()));
            let copy = |name: &str, share: Option<bool>| {
                if !installer.available_to_all_supported() {
                    return Ok(Finished::Installed(None));
                }
                let share = share.unwrap_or_else(|| installer.shares(name, &[]));
                installer.set_available_to_all_choice(name, &[], share)?;
                Ok(Finished::Installed(share.then(|| installer.make_available(name, &[]))))
            };
            let result = match request {
                Request::Install(index, i, share) => {
                    family(&index, i).and_then(|f| installer.install(&index, &f, &mut progress).and_then(|()| copy(&f.family, share)))
                }
                Request::InstallFile(index, i, path) => {
                    family(&index, i).and_then(|f| installer.install_file(&index, &f, &path, &mut progress).and_then(|()| copy(&f.family, None)))
                }
                Request::Remove(r) => installer.remove(&r).map(|()| Finished::Removed),
                Request::Preview(index, i) => family(&index, i).and_then(|f| installer.download(&index, &f, &mut progress)).map(Finished::Previewed),
                Request::AvailableToAll(name, files, on) => installer.set_available_to_all_choice(&name, &files, on).and_then(|()| {
                    if on { installer.make_available(&name, &files).map(Some) } else { installer.withdraw(&name, &files).map(|_| None) }
                        .map(Finished::Switched)
                }),
                Request::ShareAtStart => share_at_start(installer.as_ref()).map(Finished::SharedAtStart),
            };
            if matches!(result, Ok(Finished::Installed(_) | Finished::Removed)) {
                catalog.rescan();
            }
            let _ = tx.send(Msg::Done(id, result));
            ctx.request_repaint();
        });
        match &self.worker {
            Some(worker) => {
                if let Err(e) = worker.send(task) {
                    (e.0)();
                }
            }
            None => task(),
        }
    }

    /// Takes the progress reports, and returns the jobs that ended.
    pub fn poll(&mut self) -> Vec<(Running, Result<Finished, StoreError>)> {
        let mut done = Vec::new();
        while let Ok(msg) = self.rx.try_recv() {
            match msg {
                Msg::Progress(id, p) => {
                    if let Some(job) = self.running.get_mut(&id) {
                        job.progress = p;
                    }
                }
                Msg::Done(id, result) => {
                    if let Some(job) = self.running.remove(&id) {
                        done.push((job, result));
                    }
                }
            }
        }
        done
    }
}

/// The work of [`Request::ShareAtStart`]: returns the families a conflict refused.
fn share_at_start(installer: &dyn Installer) -> Result<Vec<(String, Vec<Conflict>)>, StoreError> {
    let mut refused = Vec::new();
    for (family, files) in installer.store_families() {
        let there = installer.is_available_to_all(&family, &files) && installer.not_yet_available(&family, &files).is_empty();
        if there || !installer.shares(&family, &files) {
            continue;
        }
        if let MadeAvailable::Refused(conflicts) = installer.make_available(&family, &files)? {
            refused.push((family, conflicts));
        }
    }
    Ok(refused)
}
