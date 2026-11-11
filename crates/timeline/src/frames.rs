//! Background frame rendering for monitors, thumbnails and playback prefetch.
//!
//! A small pool of worker threads takes jobs by priority: the frame on screen first (priority 0),
//! then the frames playback needs next (1, 2, …), then thumbnails ([`BACKGROUND_PRIORITY`] and
//! up). Results land in a [`FrameCache`]; the UI shows the exact frame when it is ready and the
//! nearest earlier one meanwhile ([`FrameServer::nearest`]), so scrubbing never blocks and never
//! flashes black. On wasm there are no threads: jobs wait in the same queue and
//! [`FrameServer::pump`] runs them on the UI thread within a time budget.
//!
//! The renderer is the app's ([`FrameRenderer`]): FilmCraft composites a sequence, EffectCraft a
//! comp from a project snapshot, SoundCraft decodes a movie frame.

use std::collections::VecDeque;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex, MutexGuard, PoisonError};
use std::time::Duration;

use web_time::Instant;

use crate::cache::{FrameCache, FrameCacheKey, FramePlayhead, FrameSize};

/// The priority from which a job is background work (thumbnails); a new on-screen request never
/// cancels it (FilmCraft `frames.rs:45`).
pub const BACKGROUND_PRIORITY: u32 = 40;

/// Playback starts its clock once this many frames from the playhead are ready, or after
/// [`crate::clock::PREROLL_TIMEOUT_S`]: otherwise the first frames after Play are due before a
/// decoder had a chance to produce them (FilmCraft `frames.rs:437`).
pub const PREROLL_FRAMES: i64 = 6;

/// The app's frame renderer. `render` runs on a worker thread (on the UI thread on wasm).
pub trait FrameRenderer: Send + Sync + 'static {
    /// The key that identifies a rendered frame.
    type Key: FrameCacheKey + Send + Sync + 'static;
    /// The rendered frame type; its [`FrameSize`] counts against the cache budget.
    type Frame: FrameSize + Send + Sync + 'static;
    /// What a render needs besides the key: a document snapshot (`Arc<Project>`), the time, the
    /// scale. Cloned into each job.
    type Job: Clone + Send + 'static;

    /// Render one frame. `None` when `cancel` was set (the frame is no longer wanted) or the
    /// source is not ready yet (media still loading): the job goes back to the queue on wasm and
    /// is dropped on native, where the next request asks again.
    fn render(&self, key: &Self::Key, job: &Self::Job, cancel: &CancelToken) -> Option<Self::Frame>;
}

/// Set when a job is no longer wanted; a renderer polls it between expensive steps.
#[derive(Clone, Debug, Default)]
pub struct CancelToken(Arc<AtomicBool>);

impl CancelToken {
    /// Whether the job's frame is no longer wanted.
    pub fn is_cancelled(&self) -> bool {
        self.0.load(Ordering::Relaxed)
    }

    /// Mark the job's frame as unwanted. Every clone of the token sees it.
    pub fn cancel(&self) {
        self.0.store(true, Ordering::Relaxed);
    }
}

/// Counters since the server started, cheap enough to keep on (an app's `perf.stats`).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct FrameStats {
    /// Requests already cached, queued or running.
    pub request_hits: u64,
    /// Requests that queued a job.
    pub request_misses: u64,
    /// Jobs finished, cancelled ones included.
    pub jobs_done: u64,
    /// Jobs that ended because their token was cancelled.
    pub jobs_cancelled: u64,
    /// The running estimate of one playback frame's render time, seconds.
    pub render_cost: f64,
}

/// One pass of playback scheduling: the frame due now and how playback moves.
#[derive(Clone, Debug)]
pub struct PlaybackRequest<K> {
    /// The frame due now.
    pub key: K,
    /// Frames per second of the sequence.
    pub fps: f64,
    /// Playback speed; negative plays backwards, above 1 skips frames (`speed as i64` per step).
    pub speed: f64,
    /// The clock waits for preroll: only the [`PREROLL_FRAMES`] that gate the start are asked
    /// for, not a prefetch run that walks a long GOP and evicts the playhead's pictures.
    pub preroll: bool,
    /// The last frame of the sequence; frames past it are not asked for.
    pub last: i64,
}

/// Frames queued ahead of the playhead while playing: 24 above 1.5× speed, else 14. Also how
/// far ahead a queued job may be before it is dropped as stale.
// filmcraft: crates/ui-egui/src/frames.rs:430.
pub fn prefetch_depth(speed: f64) -> i64 {
    if speed.abs() > 1.5 { 24 } else { 14 }
}

/// Which frames playback asks for, given the per-frame render cost (s), the shown frame rate and
/// the worker count: `(lead, stride)`. `(1, 1)` while the workers keep up (load
/// `cost × fps × 1.15 / workers` ≤ 1). Otherwise `lead = ceil(cost × fps)` (nearer frames would
/// be late however soon they start) and `stride = ceil(load)`: the frames in between are dropped
/// on purpose and evenly, instead of every frame arriving late.
// filmcraft: crates/ui-egui/src/frames.rs:450.
pub fn playback_plan(render_cost: f64, fps: f64, workers: usize) -> (i64, i64) {
    let frames_per_job = render_cost * fps;
    let load = frames_per_job * 1.15 / workers.max(1) as f64;
    if load <= 1.0 {
        return (1, 1);
    }
    (frames_per_job.ceil() as i64, load.ceil() as i64)
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

/// The step between the frames playback shows: `speed` as whole frames, signed by direction.
fn playback_step(speed: f64) -> i64 {
    (speed.abs().max(1.0) as i64) * if speed < 0.0 { -1 } else { 1 }
}

struct Job<R: FrameRenderer> {
    key: R::Key,
    job: R::Job,
    priority: u32,
    /// Queued by playback prefetch (dropped when playback stops).
    prefetch: bool,
    cancel: CancelToken,
}

struct Shared<R: FrameRenderer> {
    renderer: Arc<R>,
    queue: Mutex<VecDeque<Job<R>>>,
    wake: Condvar,
    cache: Mutex<FrameCache<R::Key, R::Frame>>,
    /// Running jobs: key, cancel token, prefetch.
    running: Mutex<Vec<(R::Key, CancelToken, bool)>>,
    stats: Mutex<FrameStats>,
    repaint: Mutex<Option<Box<dyn Fn() + Send + Sync>>>,
    workers: usize,
    /// Set when the server is dropped: the workers return.
    stop: AtomicBool,
}

/// The frame server: a priority queue, worker threads and the cache. `Send + Sync`; the app keeps
/// one for all monitors and thumbnails.
///
/// ```ignore
/// let frames = FrameServer::new(Arc::new(SequenceRenderer::new(pool)), FrameServer::<SequenceRenderer>::default_workers(), 2 << 30);
/// frames.set_repaint(move || ctx.request_repaint());
/// // Each pass, for the Program monitor:
/// frames.request(key.clone(), job, 0);
/// let shown = frames.get(&key).or_else(|| frames.nearest(&key, 30).map(|(_, f)| f));
/// ```
pub struct FrameServer<R: FrameRenderer> {
    shared: Arc<Shared<R>>,
}

impl<R: FrameRenderer> FrameServer<R> {
    /// A server with `workers` threads (none on wasm) and a cache of `budget` bytes.
    // filmcraft: crates/ui-egui/src/frames.rs:478 `FrameServer::new` (named threads
    // `frames-<n>`, each pulling the lowest priority, oldest first).
    pub fn new(renderer: Arc<R>, workers: usize, budget: usize) -> Self {
        let workers = if cfg!(target_arch = "wasm32") { 0 } else { workers };
        let shared = Arc::new(Shared::new(renderer, workers, budget));
        #[cfg(not(target_arch = "wasm32"))]
        for i in 0..workers {
            let shared = shared.clone();
            std::thread::Builder::new().name(format!("frames-{i}")).spawn(move || shared.work()).ok();
        }
        Self { shared }
    }

    /// The available parallelism, from 2 to 6 workers (more only compete with the UI thread and
    /// the decoders for memory bandwidth); 3 when it is unknown.
    // filmcraft: crates/ui-egui/src/frames.rs:508.
    pub fn default_workers() -> usize {
        std::thread::available_parallelism().map(|n| n.get().clamp(2, 6)).unwrap_or(3)
    }

    /// Called from a worker when a frame is ready (the app asks egui for a repaint).
    pub fn set_repaint(&self, repaint: impl Fn() + Send + Sync + 'static) {
        if let Ok(mut r) = self.shared.repaint.lock() {
            *r = Some(Box::new(repaint));
        }
    }

    /// Ask for a frame. Nothing happens when it is cached, queued (its priority rises to the
    /// lower number) or running. A request at priority 0 (on screen) replaces this view's other
    /// queued priority-0 requests and cancels its running ones for other frames (same series and
    /// scale): scrubbing asks for a frame per refresh, and the newest position must not wait
    /// behind older ones. A running job for the same frame at an older revision is left to finish.
    // filmcraft: crates/ui-egui/src/frames.rs:585 `request`, `:590` `request_job`.
    pub fn request(&self, key: R::Key, job: R::Job, priority: u32) {
        self.request_job(key, job, priority, false);
    }

    fn request_job(&self, key: R::Key, job: R::Job, priority: u32, prefetch: bool) {
        let shared = &self.shared;
        if priority == 0 {
            lock(&shared.cache).set_on_screen(Some(key.clone()));
        }
        let hit = || lock(&shared.stats).request_hits += 1;
        if lock(&shared.cache).contains(&key) || lock(&shared.running).iter().any(|(k, cancel, _)| *k == key && !cancel.is_cancelled()) {
            hit();
            return;
        }
        let mut queue = lock(&shared.queue);
        if let Some(queued) = queue.iter_mut().find(|j| j.key == key) {
            queued.priority = queued.priority.min(priority);
            queued.prefetch &= prefetch;
            drop(queue);
            hit();
            return;
        }
        lock(&shared.stats).request_misses += 1;
        if priority == 0 {
            let same_view = |k: &R::Key| key.same_series(k) && key.same_scale(k);
            queue.retain(|j| !(j.priority == 0 && !j.prefetch && same_view(&j.key)));
            for (k, cancel, prefetch) in lock(&shared.running).iter() {
                if !prefetch && same_view(k) && k.frame() != key.frame() {
                    cancel.cancel();
                }
            }
        }
        queue.push_back(Job { key, job, priority, prefetch, cancel: CancelToken::default() });
        drop(queue);
        shared.wake.notify_one();
    }

    /// Playback scheduling for one monitor: the frame due now first, then the next frames in
    /// display order (`job_for(frame)` makes each job) at priorities 1, 2, …, following
    /// [`playback_plan`] and [`prefetch_depth`]; queued jobs of this series that are stale
    /// (behind the playhead, or further ahead than the plan's span plus 26 frames) are dropped.
    /// While `preroll`, only frames 0…[`PREROLL_FRAMES`] − 1 from the playhead are asked for.
    // filmcraft: crates/ui-egui/src/frames.rs:646 `schedule_playback`.
    pub fn schedule_playback(&self, request: &PlaybackRequest<R::Key>, job_for: impl Fn(i64) -> R::Job) {
        let key = &request.key;
        let step = playback_step(request.speed);
        let dir = step.signum();
        let step = step.abs();
        let fps = request.fps * request.speed.abs().max(1.0) / step as f64;
        let (lead, stride) = if request.preroll { (1, 1) } else { playback_plan(lock(&self.shared.stats).render_cost, fps, self.shared.workers) };
        let current = key.frame();
        if lead <= 1 {
            self.request_job(key.clone(), job_for(current), 0, false);
        }
        let ahead = if request.preroll { PREROLL_FRAMES - 1 } else { prefetch_depth(request.speed) };
        for i in 0..ahead {
            let frame = current.saturating_add(lead.saturating_add(i.saturating_mul(stride)).saturating_mul(dir).saturating_mul(step));
            if frame >= 0 && frame <= request.last {
                self.request_job(key.at_frame(frame), job_for(frame), i as u32 + 1, true);
            }
        }
        let span = lead.saturating_add(ahead.saturating_mul(stride)).saturating_mul(step).saturating_add(26);
        self.retain_queue(|k| !k.same_series(key) || ((k.frame() - current) * dir >= 0 && (k.frame() - current).abs() < span));
    }

    /// Whether playback can start at `key`: the next [`PREROLL_FRAMES`] frames in the playing
    /// direction (up to `last`) are cached.
    // filmcraft: crates/ui-egui/src/frames.rs:689 `preroll_ready`.
    pub fn preroll_ready(&self, key: &R::Key, speed: f64, last: i64) -> bool {
        let step = playback_step(speed);
        let cache = lock(&self.shared.cache);
        (0..PREROLL_FRAMES).map(|i| key.frame() + i * step).filter(|f| (0..=last).contains(f)).all(|f| cache.contains(&key.at_frame(f)))
    }

    /// The cached frame, marked as shown.
    pub fn get(&self, key: &R::Key) -> Option<Arc<R::Frame>> {
        lock(&self.shared.cache).get(key)
    }

    /// See [`FrameCache::nearest`].
    pub fn nearest(&self, key: &R::Key, max_back: i64) -> Option<(R::Key, Arc<R::Frame>)> {
        lock(&self.shared.cache).nearest(key, max_back)
    }

    /// See [`FrameCache::cached_frames`].
    pub fn cached_frames(&self, series: &R::Key) -> Vec<i64> {
        lock(&self.shared.cache).cached_frames(series)
    }

    /// The playing preview for the cache's eviction order ([`FrameCache::set_playhead`]).
    pub fn set_playhead(&self, playhead: Option<FramePlayhead<R::Key>>) {
        lock(&self.shared.cache).set_playhead(playhead);
    }

    /// Drop queued jobs that fail `keep` and cancel running ones (a worker blocked behind a
    /// decoder sees the token).
    // filmcraft: crates/ui-egui/src/frames.rs:702 `retain_queue`.
    pub fn retain_queue(&self, keep: impl Fn(&R::Key) -> bool) {
        lock(&self.shared.queue).retain(|j| keep(&j.key));
        for (key, cancel, _) in lock(&self.shared.running).iter() {
            if !keep(key) {
                cancel.cancel();
            }
        }
    }

    /// Playback stopped: drop and cancel every prefetch job.
    // filmcraft: crates/ui-egui/src/frames.rs:713 `stop_prefetch`.
    pub fn stop_prefetch(&self) {
        lock(&self.shared.queue).retain(|j| !j.prefetch);
        for (_, cancel, prefetch) in lock(&self.shared.running).iter() {
            if *prefetch {
                cancel.cancel();
            }
        }
    }

    /// A new cache budget, evicting at once.
    pub fn set_budget(&self, bytes: usize) {
        lock(&self.shared.cache).set_budget(bytes);
    }

    /// `(bytes held, budget)`.
    pub fn cache_usage(&self) -> (usize, usize) {
        let cache = lock(&self.shared.cache);
        (cache.bytes(), cache.budget())
    }

    /// The number of jobs waiting for a worker.
    pub fn queue_len(&self) -> usize {
        self.shared.queue.lock().map(|q| q.len()).unwrap_or(0)
    }

    /// The number of worker threads (zero on wasm).
    pub fn workers(&self) -> usize {
        self.shared.workers
    }
    /// A snapshot of the counters.
    pub fn stats(&self) -> FrameStats {
        *lock(&self.shared.stats)
    }

    /// Without worker threads (wasm): run queued jobs on this thread, lowest priority number
    /// first, until `budget` has passed (at least one job). Jobs whose renderer returned `None`
    /// without a cancel go back to the queue. The number of jobs finished; a no-op with workers.
    // filmcraft: crates/ui-egui/src/frames.rs:739 `pump`.
    pub fn pump(&self, budget: Duration) -> usize {
        if self.shared.workers > 0 {
            return 0;
        }
        let started = Instant::now();
        let (mut done, mut retry) = (0, Vec::new());
        while let Some(outcome) = self.shared.run_next() {
            match outcome {
                Some(job) => retry.push(job),
                None => done += 1,
            }
            if started.elapsed() >= budget {
                break;
            }
        }
        let mut queue = lock(&self.shared.queue);
        for job in retry.into_iter().rev() {
            if !queue.iter().any(|q| q.key == job.key) {
                queue.push_front(job);
            }
        }
        done
    }
}

impl<R: FrameRenderer> Shared<R> {
    fn new(renderer: Arc<R>, workers: usize, budget: usize) -> Self {
        Self {
            renderer,
            queue: Mutex::new(VecDeque::new()),
            wake: Condvar::new(),
            cache: Mutex::new(FrameCache::new(budget)),
            running: Mutex::new(Vec::new()),
            stats: Mutex::new(FrameStats::default()),
            repaint: Mutex::new(None),
            workers,
            stop: AtomicBool::new(false),
        }
    }

    /// A worker thread: run jobs until the server is dropped.
    #[cfg(not(target_arch = "wasm32"))]
    fn work(&self) {
        loop {
            {
                let mut queue = lock(&self.queue);
                loop {
                    if self.stop.load(Ordering::Relaxed) {
                        return;
                    }
                    if !queue.is_empty() {
                        break;
                    }
                    queue = self.wake.wait(queue).unwrap_or_else(PoisonError::into_inner);
                }
            }
            if self.run_one()
                && let Some(repaint) = lock(&self.repaint).as_ref()
            {
                repaint();
            }
        }
    }

    /// Take the next job (lowest priority number, oldest first), render it, cache the frame and
    /// update the render-cost estimate (prefetch jobs only, an exponential average with weight
    /// 0.2). `false` when the queue was empty.
    // filmcraft: crates/ui-egui/src/frames.rs (`run_job`, the worker loop).
    fn run_one(&self) -> bool {
        match self.run_next() {
            None => false,
            Some(dropped) => {
                if let Some(job) = dropped {
                    self.finish(job.key, None, false);
                }
                true
            }
        }
    }

    /// Run the next job. The outer `None`: the queue was empty. The inner job: the renderer had
    /// no frame and no cancel was set (the source is not ready); it is neither cached nor counted.
    fn run_next(&self) -> Option<Option<Job<R>>> {
        let job = {
            let mut queue = lock(&self.queue);
            let next = queue.iter().enumerate().min_by_key(|(i, j)| (j.priority, *i)).map(|(i, _)| i)?;
            let job = queue.remove(next)?;
            lock(&self.running).push((job.key.clone(), job.cancel.clone(), job.prefetch));
            job
        };
        if self.stop.load(Ordering::Relaxed) {
            job.cancel_token().cancel();
        }
        let started = Instant::now();
        let frame = if job.cancel_token().is_cancelled() {
            None
        } else {
            // A renderer panic (bad media) must not leave the frame running forever.
            catch_unwind(AssertUnwindSafe(|| self.renderer.render(&job.key, &job.job, job.cancel_token()))).unwrap_or(None)
        };
        let cancelled = job.cancel_token().is_cancelled();
        let retry = frame.is_none() && !cancelled;
        if job.prefetch && !retry {
            let cost = started.elapsed().as_secs_f64();
            let mut stats = lock(&self.stats);
            if !cancelled || cost > stats.render_cost {
                stats.render_cost = if stats.render_cost == 0.0 { cost } else { stats.render_cost * 0.8 + cost * 0.2 };
            }
        }
        let Job { key, cancel, .. } = &job;
        if retry {
            self.forget_running(key, cancel);
            return Some(Some(job));
        }
        self.finish(key.clone(), frame, cancelled);
        self.forget_running(key, cancel);
        Some(None)
    }

    fn forget_running(&self, key: &R::Key, cancel: &CancelToken) {
        lock(&self.running).retain(|(k, c, _)| !(k == key && Arc::ptr_eq(&c.0, &cancel.0)));
    }

    /// Cache the frame (a cancelled job's result may be incomplete: not cached) and count the job.
    fn finish(&self, key: R::Key, frame: Option<R::Frame>, cancelled: bool) {
        if let Some(frame) = frame
            && !cancelled
        {
            lock(&self.cache).insert(key, Arc::new(frame));
        }
        let mut stats = lock(&self.stats);
        stats.jobs_done += 1;
        stats.jobs_cancelled += u64::from(cancelled);
    }
}

impl<R: FrameRenderer> Job<R> {
    fn cancel_token(&self) -> &CancelToken {
        &self.cancel
    }
}

impl<R: FrameRenderer> Drop for FrameServer<R> {
    fn drop(&mut self) {
        let mut queue = lock(&self.shared.queue);
        self.shared.stop.store(true, Ordering::Relaxed);
        queue.clear();
        drop(queue);
        for (_, cancel, _) in lock(&self.shared.running).iter() {
            cancel.cancel();
        }
        self.shared.wake.notify_all();
    }
}
