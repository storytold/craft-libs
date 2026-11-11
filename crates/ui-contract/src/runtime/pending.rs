//! Background results (14): one result per key in runtime memory, produced by a job on a `host::queue::JobQueue` per context.

use crate::runtime::egui::memory::Service;
use crate::runtime::egui::{pass, repaint};
use crate::runtime::env::Env;
use crate::runtime::frame;
use crate::runtime::memory::{self, MemoryStage};
use craft_host::queue::{Cancel, JobQueue, Priority, QueueOptions};
use craft_host::{Jobs, Wake};
use egui::{Context, Id, ViewportId};
use std::any::TypeId;
use std::collections::HashMap;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex, PoisonError};

/// A result that may not have arrived.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub enum Pending<T, E> {
    /// Still running.
    Loading {
        /// The job's last report, 0 to 1.
        progress: Option<f32>,
    },
    /// The job succeeded.
    Ready(T),
    /// The job failed.
    Failed(E),
    /// The job ended without delivering, [`LOST_LIMIT`] times in a row (it panics, or the host never runs it): it is
    /// not submitted again.
    Lost,
}

/// How many times in a row a job may end without delivering before its key shows [`Pending::Lost`].
pub const LOST_LIMIT: u32 = 3;

impl<T, E> Default for Pending<T, E> {
    fn default() -> Self {
        Pending::Loading { progress: None }
    }
}

/// A job's report of how far it has come. A report asks for a pass, so the progress shows.
#[derive(Clone)]
pub struct Progress {
    bits: Arc<AtomicU32>,
    ctx: Context,
}

/// The stored bits of "no report yet": a NaN, which a report never writes.
const NO_REPORT: u32 = u32::MAX;

impl Progress {
    /// Reports `fraction` (0 to 1) done.
    pub fn report(&self, fraction: f32) {
        self.bits.store(fraction.clamp(0.0, 1.0).to_bits(), Ordering::Release);
        repaint::request(&self.ctx);
    }

    fn get(&self) -> Option<f32> {
        match self.bits.load(Ordering::Acquire) {
            NO_REPORT => None,
            bits => Some(f32::from_bits(bits)),
        }
    }
}

type Run<T, E> = Box<dyn FnOnce(&Progress, &Cancel) -> Result<T, E> + Send>;

/// Background work for `Remember::pending`: runs on the context's job queue, stops early when its owner is
/// evicted or stops asking for its key.
pub struct Job<T, E> {
    priority: Priority,
    run: Run<T, E>,
}

impl<T, E> Job<T, E> {
    /// A job at `Priority::VISIBLE`. `run` checks `Cancel::is_cancelled` where it can stop early.
    pub fn new(run: impl FnOnce(&Progress, &Cancel) -> Result<T, E> + Send + 'static) -> Self {
        Job { priority: Priority::VISIBLE, run: Box::new(run) }
    }

    /// Runs before waiting jobs of lower priority.
    #[must_use]
    pub fn priority(mut self, priority: Priority) -> Self {
        self.priority = priority;
        self
    }
}

/// What the job has left for the next pass.
enum Landing<T, E> {
    Running,
    Done(Result<T, E>),
    /// The job ended without delivering: it panicked, was cancelled, or the host never ran it.
    Lost,
}

/// What a pass reads of a [`Landing`].
enum Landed<T, E> {
    Waiting,
    Result(Result<T, E>),
    Lost,
}

struct Shared<T, E> {
    landing: Mutex<Landing<T, E>>,
    progress: Progress,
}

impl<T: Clone, E: Clone> Shared<T, E> {
    /// A real pass takes a landed result; a sizing pass reads a copy, so the next real pass still finds it.
    fn read(&self, take: bool) -> Landed<T, E> {
        let mut landing = self.landing.lock().unwrap_or_else(PoisonError::into_inner);
        match &*landing {
            Landing::Running => Landed::Waiting,
            Landing::Lost => Landed::Lost,
            Landing::Done(result) if !take => Landed::Result(result.clone()),
            Landing::Done(_) => match std::mem::replace(&mut *landing, Landing::Running) {
                Landing::Done(result) => Landed::Result(result),
                Landing::Running | Landing::Lost => Landed::Waiting,
            },
        }
    }
}

/// Held by the queued job: dropped before it delivers (a panic, a cancelled or dropped job), it marks the landing
/// lost so the next pass submits again.
struct Delivery<T, E>(Arc<Shared<T, E>>, bool);

impl<T, E> Delivery<T, E> {
    fn land(&mut self, result: Result<T, E>) {
        *self.0.landing.lock().unwrap_or_else(PoisonError::into_inner) = Landing::Done(result);
        self.1 = true;
    }
}

impl<T, E> Drop for Delivery<T, E> {
    fn drop(&mut self) {
        if !self.1 {
            *self.0.landing.lock().unwrap_or_else(PoisonError::into_inner) = Landing::Lost;
        }
    }
}

/// Cancels the job when the last slot holding it is dropped, which is when the owner is evicted.
struct CancelOnDrop(Cancel);

impl Drop for CancelOnDrop {
    fn drop(&mut self) {
        self.0.cancel();
    }
}

struct Started<T, E> {
    shared: Arc<Shared<T, E>>,
    _owner: Arc<CancelOnDrop>,
}

impl<T, E> Clone for Started<T, E> {
    fn clone(&self) -> Self {
        Started { shared: self.shared.clone(), _owner: self._owner.clone() }
    }
}

/// The memory slot of one key.
pub(crate) struct PendingSlot<T, E> {
    shown: Pending<T, E>,
    started: Option<Started<T, E>>,
    /// Jobs of this key that ended without delivering, in a row.
    lost: u32,
}

impl<T: Clone, E: Clone> Clone for PendingSlot<T, E> {
    fn clone(&self) -> Self {
        PendingSlot { shown: self.shown.clone(), started: self.started.clone(), lost: self.lost }
    }
}

impl<T, E> Default for PendingSlot<T, E> {
    fn default() -> Self {
        PendingSlot { shown: Pending::default(), started: None, lost: 0 }
    }
}

/// A slot's last poll, so the end hook can drop the slots its owner stopped asking for.
struct Tracked {
    owner: Id,
    viewport: ViewportId,
    pass: u64,
    dispose: fn(&Context, Id),
}

/// The queue that runs a context's pending jobs, with the `Jobs` it spawns lanes on.
type Queue = (Arc<dyn Jobs>, Arc<JobQueue<Id, ()>>);

/// The context's queue and the slots it runs for. The queue is rebuilt when the host's `Jobs` changes.
#[derive(Default)]
struct PendingState {
    queue: Option<Queue>,
    tracked: HashMap<Id, Tracked>,
}

fn queue_of(ctx: &Context, jobs: &Arc<dyn Jobs>) -> Arc<JobQueue<Id, ()>> {
    Service::<PendingState>::run(ctx, |state| match &state.queue {
        Some((current, queue)) if Arc::ptr_eq(current, jobs) => queue.clone(),
        _ => {
            let wake_ctx = ctx.clone();
            let wake = Wake::new(move || repaint::request(&wake_ctx));
            let queue = Arc::new(JobQueue::new(jobs.clone(), wake, QueueOptions { name: "craft-pending", ..QueueOptions::default() }));
            state.queue = Some((jobs.clone(), queue.clone()));
            queue
        }
    })
}

/// Notes that `id` was asked for in this pass of the running viewport.
fn track<T: Clone + Send + Sync + 'static, E: Clone + Send + Sync + 'static>(ctx: &Context, id: Id, owner: Id) {
    let viewport = pass::current_viewport(ctx);
    let pass = pass::pass_nr(ctx, viewport);
    frame::register_end_sweep(ctx, TypeId::of::<PendingState>(), sweep);
    Service::<PendingState>::run(ctx, |state| {
        state.tracked.insert(id, Tracked { owner, viewport, pass, dispose: memory::dispose_shown::<PendingSlot<T, E>> });
    });
}

/// The end hook: a slot whose owner ran this pass without asking for it is dropped, which cancels its job
/// (a preview whose selection changed), and finished jobs' markers are cleared out of the queue.
fn sweep(ctx: &Context) {
    let viewport = pass::current_viewport(ctx);
    let now = pass::pass_nr(ctx, viewport);
    let service = Service::<PendingState>::of(ctx);
    let (dropped, queue) = service.with(|state| {
        let mut dropped = Vec::new();
        state.tracked.retain(|id, t| {
            if t.viewport != viewport {
                return true;
            }
            match memory::owner_pass(ctx, t.owner) {
                None => false,
                Some(owner_pass) if owner_pass != now || t.pass == now => true,
                Some(_) => {
                    dropped.push((*id, t.dispose));
                    false
                }
            }
        });
        (dropped, state.queue.as_ref().map(|(_, queue)| queue.clone()))
    });
    let Some(queue) = queue else { return };
    while queue.try_recv().is_some() {}
    if dropped.is_empty() {
        return;
    }
    for (id, dispose) in &dropped {
        dispose(ctx, *id);
    }
    queue.reprioritize(|key, priority| (!dropped.iter().any(|(id, _)| id == key)).then_some(priority));
}

/// The result under `id`: submits `start()` once, takes a finished result into the slot and returns it. Without
/// a host there is no spawner and the result stays loading; a sizing pass keeps nothing, so it starts and tracks
/// nothing and leaves a landed result for the next real pass. A job that ended without delivering is submitted
/// again on the next pass.
pub(crate) fn poll<T: Clone + Send + Sync + 'static, E: Clone + Send + Sync + 'static>(
    stage: &mut MemoryStage,
    id: Id,
    start: impl FnOnce() -> Job<T, E>,
) -> &Pending<T, E> {
    let ctx = stage.ctx().clone();
    let sizing = stage.is_peek();
    let host = Env::host_of(&ctx);
    if !sizing {
        track::<T, E>(&ctx, id, stage.owner());
    }
    let slot = stage.shown::<PendingSlot<T, E>>(id);
    if slot.started.is_none()
        && matches!(slot.shown, Pending::Loading { .. })
        && !sizing
        && let Some(host) = &host
    {
        let Job { priority, run } = start();
        let progress = Progress { bits: Arc::new(AtomicU32::new(NO_REPORT)), ctx: ctx.clone() };
        let shared = Arc::new(Shared { landing: Mutex::new(Landing::Running), progress: progress.clone() });
        let cancel = Cancel::new();
        let (mut delivery, token) = (Delivery(shared.clone(), false), cancel.clone());
        let queue = queue_of(&ctx, host.jobs_shared());
        slot.started = Some(Started { shared, _owner: Arc::new(CancelOnDrop(cancel)) });
        queue.submit(id, priority, move |queued| {
            if queued.is_cancelled() || token.is_cancelled() {
                return;
            }
            let result = run(&progress, &token);
            if !token.is_cancelled() {
                delivery.land(result);
            }
        });
    }
    if let Some(started) = &slot.started
        && matches!(slot.shown, Pending::Loading { .. })
    {
        match started.shared.read(!sizing) {
            Landed::Result(Ok(value)) => {
                slot.lost = 0;
                slot.shown = Pending::Ready(value);
            }
            Landed::Result(Err(error)) => {
                slot.lost = 0;
                slot.shown = Pending::Failed(error);
            }
            Landed::Lost if !sizing => {
                slot.started = None;
                slot.lost += 1;
                if slot.lost >= LOST_LIMIT {
                    slot.shown = Pending::Lost;
                }
            }
            Landed::Waiting | Landed::Lost => {
                slot.shown = Pending::Loading { progress: started.shared.progress.get() };
                if !sizing && let Some(host) = &host {
                    queue_of(&ctx, host.jobs_shared()).resume();
                }
            }
        }
    }
    &slot.shown
}
