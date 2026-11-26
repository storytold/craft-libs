// SPDX-License-Identifier: MIT OR Apache-2.0
//! What one evaluation runs with, chosen by its host: when to stop ([`Cancel`]), what to print ([`Logging`]), who
//! measures it ([`crate::profile::Profiler`]) and where its memory is counted ([`LiveBytes`]). Nothing here changes what
//! is computed.
//!
//! The evaluation entry points (`Graph::evaluate_in` and the others taking an [`EvalContext`]) carry the context
//! through the evaluation. The node code below them (the image operations and their parallel loops) does not take it
//! as an argument: the entry points make it the *current* context of the evaluating thread for the duration of the
//! call ([`scoped`]), and the parallel helpers in [`crate::tex`] and [`crate::par`] hand it on to the threads they
//! use. So two evaluations on different threads never see each other's context, and nothing is shared process-wide.
//!
//! Cancelling is a flag the host sets ([`Cancel::cancel`]); the engine never reads a clock to decide it. A host with a
//! time limit sets the flag from a timer of its own.
//!
//! Files a graph names but does not carry (a bitmap, a font, an SVG) are read through the host's [`FileSource`]
//! ([`EvalContext::files`]); evaluation never opens a file itself.

use std::cell::RefCell;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering::Relaxed};

/// A flag the host sets to stop an evaluation. Clones share the flag. An evaluation that sees it stops at the next
/// node or the next few rows of a parallel loop, and returns `Error::Cancelled` (its partial results must be thrown
/// away).
#[derive(Clone, Debug, Default)]
pub struct Cancel(Arc<AtomicBool>);

impl Cancel {
    /// A flag not set yet.
    pub fn new() -> Cancel {
        Cancel::default()
    }

    /// Asks every evaluation holding this flag to stop.
    pub fn cancel(&self) {
        self.0.store(true, Relaxed);
    }

    pub fn is_cancelled(&self) -> bool {
        self.0.load(Relaxed)
    }
}

/// Diagnostics an evaluation prints to stderr. They never change what is computed.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Logging {
    /// One line per evaluation: node count, its own peak and what the evaluations sharing its [`LiveBytes`] hold.
    pub memory: bool,
    /// One line per node computed, with its time (read from the system clock; on the web, without the time).
    pub node_times: bool,
}

/// Bytes of node results held now and the most held at once, by every evaluation given this counter. Clones share
/// the counts: a host that wants one figure for all its evaluations gives them all the same counter.
#[derive(Clone, Debug, Default)]
pub struct LiveBytes(Arc<(AtomicUsize, AtomicUsize)>);

impl LiveBytes {
    pub fn new() -> LiveBytes {
        LiveBytes::default()
    }

    /// (held now, most held at once).
    pub fn get(&self) -> (usize, usize) {
        (self.0.0.load(Relaxed), self.0.1.load(Relaxed))
    }

    pub(crate) fn add(&self, b: usize) {
        let now = self.0.0.fetch_add(b, Relaxed) + b;
        self.0.1.fetch_max(now, Relaxed);
    }

    pub(crate) fn sub(&self, b: usize) {
        // a compare-exchange loop: `fetch_update` is deprecated on newer toolchains and `try_update` is not on older ones
        let mut now = self.0.0.load(Relaxed);
        while let Err(seen) = self.0.0.compare_exchange_weak(now, now.saturating_sub(b), Relaxed, Relaxed) {
            now = seen;
        }
    }
}

/// Where an evaluation reads the files a graph names but does not carry in its resources. Paths are as the graph
/// writes them (graphs written on Windows use backslashes).
pub trait FileSource: Send + Sync {
    /// The bytes of the file, or why there are none (shown in the node's error).
    fn read(&self, path: &str) -> Result<Vec<u8>, String>;
    /// Something that changes when the file does (on disk: its length and modification time), for caches that keep
    /// results between evaluations; `None` when there is no such file.
    fn stamp(&self, path: &str) -> Option<(u64, u64)>;
}

/// No files at all: every read fails.
#[derive(Clone, Copy, Debug, Default)]
pub struct NoFiles;

impl FileSource for NoFiles {
    fn read(&self, path: &str) -> Result<Vec<u8>, String> {
        Err(format!("no file system to read '{path}' from"))
    }

    fn stamp(&self, _path: &str) -> Option<(u64, u64)> {
        None
    }
}

/// The file system of the machine, for native hosts: a path with backslashes that isn't found is tried again with
/// slashes.
#[cfg(not(target_family = "wasm"))]
#[derive(Clone, Copy, Debug, Default)]
pub struct DiskFiles;

#[cfg(not(target_family = "wasm"))]
impl FileSource for DiskFiles {
    fn read(&self, path: &str) -> Result<Vec<u8>, String> {
        std::fs::read(path).or_else(|e| if path.contains('\\') { std::fs::read(path.replace('\\', "/")) } else { Err(e) }).map_err(|e| e.to_string())
    }

    fn stamp(&self, path: &str) -> Option<(u64, u64)> {
        let m = std::fs::metadata(path).or_else(|_| std::fs::metadata(path.replace('\\', "/"))).ok()?;
        let modified = m.modified().ok().and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok()).map(|d| d.as_nanos() as u64).unwrap_or(0);
        Some((m.len(), modified))
    }
}

/// The files an evaluation reads ([`FileSource`]). The default is the machine's file system ([`DiskFiles`]) on native
/// targets and no files ([`NoFiles`]) on the web.
#[derive(Clone)]
pub struct Files(pub Arc<dyn FileSource>);

impl Files {
    /// No files: every read fails.
    pub fn none() -> Files {
        Files(Arc::new(NoFiles))
    }

    /// The machine's file system.
    #[cfg(not(target_family = "wasm"))]
    pub fn disk() -> Files {
        Files(Arc::new(DiskFiles))
    }
}

impl Default for Files {
    fn default() -> Files {
        #[cfg(not(target_family = "wasm"))]
        return Files::disk();
        #[cfg(target_family = "wasm")]
        return Files::none();
    }
}

impl std::ops::Deref for Files {
    type Target = dyn FileSource;
    fn deref(&self) -> &(dyn FileSource + 'static) {
        &*self.0
    }
}

/// Everything an evaluation takes from its host besides the graph. The default runs to the end, prints nothing,
/// measures nothing and counts its memory on its own.
#[derive(Clone, Default)]
pub struct EvalContext {
    pub cancel: Cancel,
    pub logging: Logging,
    /// Measures named spans around nodes and stages; `None` measures nothing.
    pub profiler: Option<Arc<dyn crate::profile::Profiler>>,
    pub accelerator: Option<Arc<dyn crate::accelerator::Accelerator>>,
    pub live: LiveBytes,
    /// The threads the evaluation's parallel loops run on; `None`: the pool the caller is in (rayon's global pool
    /// outside any). A host whose own parallel work waits on evaluations gives them a pool of their own, so that its
    /// waiting threads can never be the ones an evaluation needs.
    pub pool: Option<Arc<rayon::ThreadPool>>,
    /// Where files the graph names but does not carry are read from.
    pub files: Files,
    /// Counts the graph's own nodes as they finish (not those inside subgraphs), for a host showing a render's progress
    /// against [`crate::graph::Graph::work_nodes`]; `None` counts nothing. Nothing computed depends on it.
    pub nodes_done: Option<Arc<std::sync::atomic::AtomicUsize>>,
}

impl std::fmt::Debug for EvalContext {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("EvalContext")
            .field("cancel", &self.cancel)
            .field("logging", &self.logging)
            .field("profiler", &self.profiler.is_some())
            .field("accelerator", &self.accelerator.is_some())
            .field("live", &self.live)
            .field("pool", &self.pool.as_ref().map(|p| p.current_num_threads()))
            .field("files", &"..")
            .field("nodes_done", &self.nodes_done)
            .finish()
    }
}

impl EvalContext {
    /// The default context with `cancel` as its flag.
    pub fn with_cancel(cancel: Cancel) -> EvalContext {
        EvalContext { cancel, ..EvalContext::default() }
    }

    pub fn is_cancelled(&self) -> bool {
        self.cancel.is_cancelled()
    }
}

thread_local! {
    static CURRENT: RefCell<Option<EvalContext>> = const { RefCell::new(None) };
}

/// Runs `f` with `ec` as this thread's current context, and puts the previous one back afterwards (also when `f`
/// panics). The engine's entry points do this themselves; a host calls it to run its own work on the engine's
/// parallel helpers under a context (so that the work stops when the context is cancelled).
pub fn scoped<R>(ec: &EvalContext, f: impl FnOnce() -> R) -> R {
    struct Restore(Option<EvalContext>);
    impl Drop for Restore {
        fn drop(&mut self) {
            let previous = self.0.take();
            CURRENT.with(|c| *c.borrow_mut() = previous);
        }
    }
    let _restore = Restore(CURRENT.with(|c| c.replace(Some(ec.clone()))));
    f()
}

/// A pool of its own for evaluations ([`EvalContext::pool`]), as many threads as rayon's default, named `name-N`.
/// `None` when the system gives no threads.
pub fn own_pool(name: &str) -> Option<Arc<rayon::ThreadPool>> {
    let name = name.to_string();
    rayon::ThreadPoolBuilder::new().thread_name(move |i| format!("{name}-{i}")).build().ok().map(Arc::new)
}

/// Runs `f` under `ec` when there is one, as it is otherwise: how the parallel helpers hand the caller's context to
/// their threads.
pub(crate) fn scoped_opt<R>(ec: Option<&EvalContext>, f: impl FnOnce() -> R) -> R {
    match ec {
        Some(ec) => scoped(ec, f),
        None => f(),
    }
}

/// Runs `f` on the pool of `ec` when it names one, else where it is called.
pub(crate) fn in_pool<R: Send>(ec: Option<&EvalContext>, f: impl FnOnce() -> R + Send) -> R {
    match ec.and_then(|ec| ec.pool.as_ref()) {
        Some(pool) => pool.install(f),
        None => f(),
    }
}

/// This thread's current context, if an evaluation (or [`scoped`]) set one.
pub fn current() -> Option<EvalContext> {
    CURRENT.with(|c| c.borrow().clone())
}

/// Calls `f` with this thread's current context, without cloning it.
pub(crate) fn with_current<R>(f: impl FnOnce(Option<&EvalContext>) -> R) -> R {
    CURRENT.with(|c| f(c.borrow().as_ref()))
}

/// The flag of this thread's current context, for a parallel loop to check on its threads.
pub(crate) fn current_cancel() -> Option<Cancel> {
    with_current(|ec| ec.map(|ec| ec.cancel.clone()))
}

/// True when `cancel` (a flag from [`current_cancel`]) is set.
#[inline]
pub(crate) fn is_set(cancel: &Option<Cancel>) -> bool {
    cancel.as_ref().is_some_and(Cancel::is_cancelled)
}

/// True when this thread's current context has been cancelled.
pub fn cancelled() -> bool {
    with_current(|ec| ec.is_some_and(EvalContext::is_cancelled))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scoped_contexts_nest_and_are_put_back() {
        assert!(current().is_none());
        let outer = EvalContext::default();
        let inner = EvalContext::with_cancel(Cancel::new());
        inner.cancel.cancel();
        scoped(&outer, || {
            assert!(!cancelled());
            scoped(&inner, || assert!(cancelled()));
            assert!(!cancelled());
        });
        assert!(current().is_none());
        let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| scoped(&inner, || panic!("unwinds"))));
        assert!(current().is_none(), "a panic must not leave the context behind");
    }

    #[test]
    fn live_bytes_are_shared_by_clones_and_keep_their_peak() {
        let a = LiveBytes::new();
        let b = a.clone();
        a.add(100);
        b.add(50);
        a.sub(120);
        assert_eq!(b.get(), (30, 150));
        a.sub(1000);
        assert_eq!(a.get().0, 0, "never below zero");
    }
}
