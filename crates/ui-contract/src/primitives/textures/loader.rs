//! `TextureLoader<K>`: a `TextureStore` fed by background jobs on a `craft_host::queue::JobQueue` per context.

use super::store::{Revision, TextureKey, TextureState, TextureStore, TextureStoreOptions};
use super::words::TextureWords;
use crate::runtime::egui::{memory, pass, repaint};
use crate::runtime::env::Env;
use craft_host::queue::{Cancel, JobQueue, QueueOptions};
use craft_host::{Jobs, Wake, default_jobs};
use egui::{Color32, ColorImage, Context};
use std::collections::HashMap;
use std::fmt::Debug;
use std::hash::Hash;
use std::sync::Arc;

pub use craft_host::queue::Priority;

/// A loader's settings, read when it is created (`TextureLoader::in_context`).
#[derive(Clone)]
#[non_exhaustive]
pub struct LoaderOptions {
    /// Settings of the loader's store.
    pub store: TextureStoreOptions,
    /// Jobs running at once; 0 runs one job per frame on the UI thread (wasm32).
    pub lanes: usize,
    /// Where the lanes run. `None`: the installed host's `Jobs`, or `craft_host::default_jobs()` when no host is
    /// installed (threads on native targets, one job per frame on wasm32). Stories and tests pass a `ManualJobs` to
    /// finish one job at a time.
    pub jobs: Option<Arc<dyn Jobs>>,
}

impl LoaderOptions {
    /// Thumbnail budgets, the machine's lanes, the host's jobs.
    pub fn new() -> Self {
        Self { store: TextureStoreOptions::thumbnails(), lanes: QueueOptions::default_lanes(), jobs: None }
    }

    /// Settings of the loader's store.
    #[must_use]
    pub fn store(mut self, store: TextureStoreOptions) -> Self {
        self.store = store;
        self
    }

    /// Jobs running at once.
    #[must_use]
    pub fn lanes(mut self, lanes: usize) -> Self {
        self.lanes = lanes;
        self
    }

    /// Where the lanes run.
    #[must_use]
    pub fn jobs(mut self, jobs: Arc<dyn Jobs>) -> Self {
        self.jobs = Some(jobs);
        self
    }
}

impl Default for LoaderOptions {
    fn default() -> Self {
        Self::new()
    }
}

impl Debug for LoaderOptions {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LoaderOptions").field("store", &self.store).field("lanes", &self.lanes).field("jobs", &self.jobs.is_some()).finish()
    }
}

/// What a background job makes when it succeeds: a picture, or the answer that the key has none.
#[derive(Debug)]
#[non_exhaustive]
pub enum LoadedPicture {
    /// The picture to upload.
    Image(ColorImage),
    /// The key has no picture (a file the decoder doesn't read): stored with `TextureStore::put_empty`, so its state
    /// is `Empty` and the row paints its own tile.
    Blank,
}

impl From<ColorImage> for LoadedPicture {
    fn from(image: ColorImage) -> Self {
        Self::Image(image)
    }
}

/// Why a job made no picture. A job carries no text: the state carries this value and the painter words it at run
/// time with `LoadFailure::text`.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum LoadFailure {
    /// The file or page the picture comes from is gone.
    Missing,
    /// It exists but could not be read or decoded.
    Unreadable,
    /// The job panicked. The loader makes this itself.
    Panicked,
    /// The source's own message, already in the user's language (a decoder's error text).
    Message(String),
}

impl LoadFailure {
    /// The failure tooltip in the words of the scope (`cx.words::<TextureWords>()`).
    pub fn text<'a>(&'a self, words: &'a TextureWords) -> &'a str {
        match self {
            Self::Missing => words.missing.as_str(),
            Self::Unreadable => words.unreadable.as_str(),
            Self::Panicked => words.panicked.as_str(),
            Self::Message(message) => message,
        }
    }
}

/// What a loader job returns: a `LoadedPicture`, or why there is none.
pub type LoadResult = Result<LoadedPicture, LoadFailure>;

/// A key's job in flight: the revision it makes and when it was last asked for.
struct Asked {
    revision: Revision,
    priority: Priority,
    /// The last frame a painter asked for the key.
    frame: u64,
}

type Finished<K> = (K, Revision, LoadResult);

struct Pool<K> {
    lanes: usize,
    queue: Arc<JobQueue<K, (Revision, LoadResult)>>,
    asked: HashMap<K, Asked>,
    /// The frame the pass work last ran in, so it runs once per frame.
    polled: Option<u64>,
    /// Finished results waiting for upload allowance, oldest first.
    waiting: Vec<Finished<K>>,
}

/// The loaders of one context and key type, by salt.
struct Pools<K> {
    by_salt: HashMap<u64, Pool<K>>,
}

impl<K> Default for Pools<K> {
    fn default() -> Self {
        Self { by_salt: HashMap::new() }
    }
}

/// A `TextureStore` whose pictures are made by background jobs: FilmCraft's frames, PdfCraft's pages and the
/// thumbnails of a list. The queue is a `JobQueue` of the context, built the way `Remember::pending` builds its own.
///
/// - **One frame.** The first call in a frame (`request`, `ask`, `submit`) takes finished results off the queue and
///   uploads them in the order they finished, while `TextureStore::allow_upload` allows (16 and 16 MiB per frame by
///   default). The rest wait for later frames and the loader asks for the next pass at once. A result for a
///   revision other than the one last asked for is discarded; a `LoadedPicture::Blank` answer is stored with
///   `TextureStore::put_empty` and a failure with `TextureStore::put_failure`.
/// - **Dropping.** In that first call, keys nobody asked for in this frame or the previous one are dropped from the
///   queue: rows scrolled away, a panel closed. A frame that does not draw the list asks for nothing, so its jobs
///   are dropped at the next frame that draws. Asked keys move to the priority they were last asked with.
/// - **Failures.** A job returns a `LoadFailure`; the state carries it. A job that panics is stored as
///   `LoadFailure::Panicked`.
/// - **Waking.** The queue asks for a pass when a job finishes; no timed repaint.
/// - **Without lanes** (`lanes == 0`) the first call in a frame runs one job inline.
pub struct TextureLoader<K: TextureKey> {
    store: TextureStore<K>,
    queue: Arc<JobQueue<K, (Revision, LoadResult)>>,
    lanes: usize,
}

impl<K: TextureKey> Clone for TextureLoader<K> {
    fn clone(&self) -> Self {
        Self { store: self.store.clone(), queue: Arc::clone(&self.queue), lanes: self.lanes }
    }
}

impl<K: TextureKey> TextureLoader<K> {
    /// The loader of `ctx` under `salt`, made with `options` the first time; its store is
    /// `TextureStore::in_context` under the same salt. Later calls with the same salt get the same loader and
    /// ignore `options`.
    pub fn in_context(ctx: &Context, salt: impl Hash + Debug, options: LoaderOptions) -> Self {
        let store = TextureStore::in_context(ctx, &salt, options.store);
        let id = store.salt();
        let known = memory::with(ctx, |pools: &mut Pools<K>| pools.by_salt.get(&id).map(|pool| (Arc::clone(&pool.queue), pool.lanes)));
        let (queue, lanes) = match known {
            Some(known) => known,
            None => {
                let jobs = options.jobs.unwrap_or_else(|| Env::host_of(ctx).map_or_else(default_jobs, |host| Arc::clone(host.jobs_shared())));
                let wake = {
                    let ctx = ctx.clone();
                    Wake::new(move || repaint::request(&ctx))
                };
                let queue = Arc::new(JobQueue::new(jobs, wake, QueueOptions { lanes: options.lanes, name: "craft-textures" }));
                memory::with(ctx, |pools: &mut Pools<K>| {
                    let pool = pools.by_salt.entry(id).or_insert_with(|| Pool::new(queue, options.lanes));
                    (Arc::clone(&pool.queue), pool.lanes)
                })
            }
        };
        Self { store, queue, lanes }
    }

    /// The loader's store: `state`, `forget`, `retain` and `statistics` work on it as on any store.
    pub fn cache(&self) -> &TextureStore<K> {
        &self.store
    }

    /// What `key` shows for `revision`, queueing the job `job()` makes when the store has nothing for this revision
    /// and no job for it is queued or running: `ask`, then `submit`. `job` is called only when a job is queued.
    pub fn request<J>(&self, ctx: &Context, key: &K, revision: Revision, priority: Priority, job: impl FnOnce() -> J) -> TextureState
    where
        J: FnOnce(&Cancel) -> LoadResult + Send + 'static,
    {
        match self.ask(ctx, key, revision, priority) {
            Some(state) => state,
            None if !crate::runtime::measure::effect(ctx, "textures::request") => self.store.state(ctx, key, revision),
            None => self.submit(ctx, key.clone(), revision, priority, job()),
        }
    }

    /// What `key` shows for `revision` when that is known without a new job: `Some` of the store's `Ready`, `Empty`
    /// or `Failed`, or `Some(Loading { stand_in })` while a job for this revision is queued or running; `None` when
    /// a job is needed. Marks the key asked for in this frame at `priority`, so its job is kept.
    pub fn ask(&self, ctx: &Context, key: &K, revision: Revision, priority: Priority) -> Option<TextureState> {
        self.run_frame_work(ctx);
        let state = match self.store.state(ctx, key, revision) {
            settled @ (TextureState::Ready(_) | TextureState::Empty | TextureState::Failed { .. }) => return Some(settled),
            other => other,
        };
        let frame = pass::root_frame_nr(ctx);
        self.with(ctx, |pool| {
            let asked = pool.asked.get_mut(key).filter(|asked| asked.revision == revision)?;
            asked.priority = priority;
            asked.frame = frame;
            Some(())
        })?;
        Some(TextureState::Loading { stand_in: state.shown() })
    }

    /// Queues `job` making `key`'s picture at `revision`, replacing a queued job of the key, and returns `Loading`
    /// with the key's old picture as the stand-in. Marks the key asked for in this frame.
    pub fn submit(
        &self,
        ctx: &Context,
        key: K,
        revision: Revision,
        priority: Priority,
        job: impl FnOnce(&Cancel) -> LoadResult + Send + 'static,
    ) -> TextureState {
        if !crate::runtime::measure::effect(ctx, "textures::submit") {
            return self.store.state(ctx, &key, revision);
        }
        self.run_frame_work(ctx);
        let stand_in = self.store.state(ctx, &key, revision).shown();
        let frame = pass::root_frame_nr(ctx);
        self.with(ctx, |pool| pool.asked.insert(key.clone(), Asked { revision, priority, frame }));
        // A panicking job is a failure, not a key that stays Loading for good.
        self.queue.submit(key, priority, move |cancel| {
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| job(cancel))).unwrap_or(Err(LoadFailure::Panicked));
            (revision, result)
        });
        if self.lanes == 0 {
            // The job runs in the next frame's `run_frame_work`; the web repaints only on input.
            repaint::request(ctx);
        }
        TextureState::Loading { stand_in }
    }

    /// Stops the loader's queue: waiting jobs are dropped, running ones are cancelled, and the call waits up to
    /// `timeout` for them to end. Returns whether every job has. For an app about to exit the process.
    pub fn shutdown(&self, timeout: std::time::Duration) -> bool {
        self.queue.shutdown(timeout)
    }

    /// Runs `f` on the loader's state under the service lock. `f` runs no app code and makes no call that reaches
    /// the context.
    fn with<R>(&self, ctx: &Context, f: impl FnOnce(&mut Pool<K>) -> R) -> R {
        memory::with(ctx, |pools: &mut Pools<K>| {
            let pool = pools.by_salt.entry(self.store.salt()).or_insert_with(|| Pool::new(Arc::clone(&self.queue), self.lanes));
            f(pool)
        })
    }

    /// In the first call of a frame, runs the frame's work: an inline job when there are no lanes, dropping jobs
    /// nobody asked for, then storing finished results while the store's upload allowance lasts. A result for
    /// another revision than the one last asked for is discarded.
    fn run_frame_work(&self, ctx: &Context) {
        let frame = pass::root_frame_nr(ctx);
        let first = self.with(ctx, |pool| pool.polled.replace(frame) != Some(frame));
        if !first {
            return;
        }
        if self.lanes == 0 {
            self.queue.run_inline(1);
        }
        let batch = self.with(ctx, |pool| {
            let recent = frame.saturating_sub(1);
            let asked = &mut pool.asked;
            for key in pool.queue.reprioritize(|key, _| asked.get(key).filter(|asked| asked.frame >= recent).map(|asked| asked.priority)) {
                asked.remove(&key);
            }
            while let Some(finished) = pool.queue.try_recv() {
                let (revision, result) = finished.result;
                pool.waiting.push((finished.key, revision, result));
            }
            let waiting = std::mem::take(&mut pool.waiting);
            waiting.into_iter().filter(|(key, revision, _)| pool.asked.get(key).is_some_and(|asked| asked.revision == *revision)).collect::<Vec<_>>()
        });
        let mut batch = batch.into_iter();
        let mut stored = Vec::new();
        let mut deferred = Vec::new();
        while let Some((key, revision, result)) = batch.next() {
            match result {
                Ok(LoadedPicture::Image(image)) => {
                    if !self.store.allow_upload(ctx, image.pixels.len() * size_of::<Color32>()) {
                        deferred.push((key, revision, Ok(LoadedPicture::Image(image))));
                        deferred.extend(batch);
                        repaint::request(ctx);
                        break;
                    }
                    self.store.put(ctx, key.clone(), revision, image);
                }
                Ok(LoadedPicture::Blank) => self.store.put_empty(ctx, key.clone(), revision),
                Err(failure) => self.store.put_failure(ctx, key.clone(), revision, failure),
            }
            stored.push(key);
        }
        self.with(ctx, |pool| {
            for key in &stored {
                pool.asked.remove(key);
            }
            pool.waiting.splice(0..0, deferred);
        });
    }
}

impl<K: TextureKey> Pool<K> {
    fn new(queue: Arc<JobQueue<K, (Revision, LoadResult)>>, lanes: usize) -> Self {
        Self { lanes, queue, asked: HashMap::new(), polled: None, waiting: Vec::new() }
    }
}
