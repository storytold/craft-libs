//! Fetching served faces: the UI language's at install, others when egui asks the context's providers for a character
//! no installed face has. The fetching itself is a [`FontFetcher`]: the browser's on the web, a fake one in tests.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError, Weak};

use egui::epaint::text::Blob;
use egui::{Context, FallbackRequest, FontInsert, FontProvider};

use crate::fonts::Stack;
use crate::runtime::egui::{fonts as egui_fonts, memory};

/// Seconds before a failed file is fetched again; each further failure doubles it, up to
/// [`RETRY_MAX`].
const RETRY_FIRST: f64 = 2.0;
const RETRY_MAX: f64 = 600.0;
/// The most doublings of the retry delay; the delay is capped long before.
const MAX_DOUBLINGS: u32 = 16;

/// Called with a fetched file's bytes, or why it failed.
pub type FetchDone = Box<dyn FnOnce(Result<Vec<u8>, String>) + Send>;
/// Called when a timer fires.
pub type FetchWake = Box<dyn FnOnce() + Send>;

/// Fetches files and keeps time for the loader of served faces.
pub trait FontFetcher: Send + Sync {
    /// Start fetching `url`, checked against `integrity` when not empty, and call `done` with its
    /// bytes or why it failed.
    fn fetch(&self, url: &'static str, integrity: &'static str, done: FetchDone);
    /// Seconds since some fixed time.
    fn now(&self) -> f64;
    /// Call `wake` once, `seconds` from now.
    fn after(&self, seconds: f64, wake: FetchWake);
}

/// The context's loader.
#[derive(Default)]
struct Slot(Option<Arc<Loader>>);

fn loader_of(ctx: &Context) -> Option<Arc<Loader>> {
    memory::with::<Slot, _>(ctx, |slot| slot.0.clone())
}

/// Install `stack`, add the files fetched so far, and fetch the UI language's faces.
pub(in crate::fonts) fn install(ctx: &Context, mut stack: Stack, fetcher: Arc<dyn FontFetcher>) {
    let loader = Loader::of(ctx, fetcher);
    let fetch = {
        let mut state = loader.lock();
        let now = loader.fetcher.now();
        for (&(url, integrity), file) in &state.files {
            stack.provide_fetched(url, integrity, file);
        }
        state.ui_pending.clear();
        let mut fetch: Vec<Key> = Vec::new();
        for face in stack.ui_downloads() {
            let key = (face.url, face.integrity);
            if state.files.contains_key(&key) || !state.due(key, now) {
                continue;
            }
            state.ui_pending.insert(key);
            if !fetch.contains(&key) {
                fetch.push(key);
            }
        }
        state.stack = Some(stack.clone());
        fetch
    };
    stack.install_faces(ctx, Some(Arc::clone(&loader) as Arc<dyn FontProvider>));
    for key in fetch {
        loader.fetch(key);
    }
}

pub(in crate::fonts) fn loading(ctx: &Context) -> bool {
    loader_of(ctx).is_some_and(|loader| !loader.lock().ui_pending.is_empty())
}

/// The context's fetched files and the stack they go into. Also the provider asked for characters
/// no face has, which starts fetching the face that draws them.
struct Loader {
    ctx: Context,
    me: Weak<Loader>,
    fetcher: Arc<dyn FontFetcher>,
    state: Mutex<State>,
}

/// A file as one face asks for it: its URL and the integrity the bytes were checked against.
/// Bytes fetched for one integrity (or none) are never handed to a face that requires another.
type Key = (&'static str, &'static str);

#[derive(Default)]
struct State {
    /// The installed stack, without the files' data.
    stack: Option<Stack>,
    /// Fetched files; the browser's HTTP cache keeps them across page loads.
    files: BTreeMap<Key, Blob>,
    in_flight: BTreeSet<Key>,
    failed: BTreeMap<Key, Failure>,
    /// The UI language's files not fetched yet.
    ui_pending: BTreeSet<Key>,
}

struct Failure {
    attempts: u32,
    retry_at: f64,
}

impl State {
    /// Whether `key` may be fetched: it never failed, or its retry time has come.
    fn due(&self, key: Key, now: f64) -> bool {
        self.failed.get(&key).is_none_or(|failure| now >= failure.retry_at)
    }
}

impl Loader {
    fn of(ctx: &Context, fetcher: Arc<dyn FontFetcher>) -> Arc<Self> {
        if let Some(existing) = loader_of(ctx) {
            return existing;
        }
        let loader = Arc::new_cyclic(|me| Self { ctx: ctx.clone(), me: me.clone(), fetcher, state: Mutex::default() });
        memory::with::<Slot, _>(ctx, |slot| Arc::clone(slot.0.get_or_insert(loader)))
    }

    fn lock(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn fetch(&self, key: Key) {
        if !self.lock().in_flight.insert(key) {
            return;
        }
        let Some(me) = self.me.upgrade() else { return };
        let (url, integrity) = key;
        self.fetcher.fetch(url, integrity, Box::new(move |result| me.arrived(key, result)));
    }

    fn arrived(&self, key: Key, result: Result<Vec<u8>, String>) {
        let (stack, retry) = {
            let mut state = self.lock();
            state.in_flight.remove(&key);
            state.ui_pending.remove(&key);
            match result {
                Ok(bytes) => {
                    state.failed.remove(&key);
                    let file: Blob = Arc::new(bytes);
                    state.files.insert(key, Arc::clone(&file));
                    let stack = state.stack.as_mut().map(|stack| {
                        stack.provide_fetched(key.0, key.1, &file);
                        stack.clone()
                    });
                    (stack, None)
                }
                Err(_) => {
                    let attempts = state.failed.get(&key).map_or(1, |failure| failure.attempts + 1);
                    let delay = (RETRY_FIRST * 2f64.powi(i32::try_from(attempts.saturating_sub(1).min(MAX_DOUBLINGS)).unwrap_or(0))).min(RETRY_MAX);
                    state.failed.insert(key, Failure { attempts, retry_at: self.fetcher.now() + delay });
                    (None, Some(delay))
                }
            }
        };
        if let Some(stack) = stack
            && let Some(me) = self.me.upgrade()
        {
            stack.install_faces(&self.ctx, Some(me as Arc<dyn FontProvider>));
        }
        // The file that arrived draws characters egui found no face for; after a failure, they go
        // to the next face that draws them now and to this one again at its retry time.
        egui_fonts::reload_providers(&self.ctx);
        if let Some(delay) = retry {
            let me = self.me.clone();
            self.fetcher.after(
                delay,
                Box::new(move || {
                    if let Some(me) = me.upgrade() {
                        egui_fonts::reload_providers(&me.ctx);
                        egui_fonts::wake(&me.ctx);
                    }
                }),
            );
        }
        egui_fonts::wake(&self.ctx);
    }
}

impl FontProvider for Loader {
    /// Start fetching the face that draws the character and find nothing for now: the face is
    /// installed when it arrives, and egui asks again. Only faces whose files map the character
    /// are candidates, the first one the family asks; a failed one leaves the character to the
    /// next until its retry time. A character no served face maps fetches nothing.
    fn font_for(&self, request: &FallbackRequest<'_>) -> Option<FontInsert> {
        let now = self.fetcher.now();
        let state = self.lock();
        let mut next = None;
        for face in state.stack.as_ref()?.downloads(request.family, request.cluster) {
            let key = (face.url, face.integrity);
            if state.in_flight.contains(&key) || state.files.contains_key(&key) {
                // It draws the character once it arrives, or already does.
                return None;
            }
            if state.due(key, now) {
                next = Some(key);
                break;
            }
        }
        drop(state);
        self.fetch(next?);
        None
    }
}
