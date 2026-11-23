//! Installs, removals and preview downloads, one at a time on a worker thread.

use std::collections::BTreeMap;
use std::sync::Arc;
use std::sync::mpsc::{Receiver, Sender, channel};

use crate::Catalog;
use crate::index::Index;
use crate::store::{Downloaded, Installer, Progress, RemoveRequest, StoreError};

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
}

pub struct Running {
    /// The family's source; empty for a removal, which goes by the installed files.
    pub source: String,
    pub family: String,
    pub kind: Kind,
    /// One file's index path or store path, when the job is for one style.
    pub file: Option<String>,
    pub progress: Progress,
}

impl Running {
    /// How far along, when the size is known.
    pub fn fraction(&self) -> Option<f32> {
        (self.progress.total > 0).then(|| (self.progress.done as f32 / self.progress.total as f32).clamp(0.0, 1.0))
    }
}

pub enum Finished {
    Installed,
    Removed,
    Previewed(Vec<Downloaded>),
}

enum Msg {
    Progress(u64, Progress),
    Done(u64, Result<Finished, StoreError>),
}

pub enum Request {
    /// A family of the index, by position.
    Install(Arc<Index>, usize),
    /// One file of a family of the index, by its path in the index.
    InstallFile(Arc<Index>, usize, String),
    Remove(RemoveRequest),
    /// A family of the index, by position.
    Preview(Arc<Index>, usize),
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

    /// Runs `request` through `installer`, then rescans `catalog` after an install or removal.
    pub fn start(&mut self, ctx: &egui::Context, installer: Arc<dyn Installer>, catalog: Arc<Catalog>, request: Request) {
        let id = self.next;
        self.next += 1;
        let (source, family, kind, file, total) = match &request {
            Request::Install(index, i) | Request::Preview(index, i) => {
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
        };
        self.running.insert(id, Running { source, family, kind, file, progress: Progress { done: 0, total } });
        let tx = self.tx.clone();
        let ctx = ctx.clone();
        let task: Task = Box::new(move || {
            let mut progress = |p: Progress| {
                let _ = tx.send(Msg::Progress(id, p));
                ctx.request_repaint();
            };
            let family = |index: &Index, i: usize| index.families().get(i).cloned().ok_or_else(|| StoreError("no such family".into()));
            let result = match request {
                Request::Install(index, i) => {
                    family(&index, i).and_then(|f| installer.install(&index, &f, &mut progress)).map(|()| Finished::Installed)
                }
                Request::InstallFile(index, i, path) => {
                    family(&index, i).and_then(|f| installer.install_file(&index, &f, &path, &mut progress)).map(|()| Finished::Installed)
                }
                Request::Remove(r) => installer.remove(&r).map(|()| Finished::Removed),
                Request::Preview(index, i) => family(&index, i).and_then(|f| installer.download(&index, &f, &mut progress)).map(Finished::Previewed),
            };
            if matches!(result, Ok(Finished::Installed | Finished::Removed)) {
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
