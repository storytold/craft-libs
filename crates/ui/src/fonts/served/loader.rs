//! Fetching served faces: the UI language's at install, others when egui asks the context's
//! providers for a character no installed face has. The fetching itself is a [`Fetcher`]: the
//! browser's on the web, a fake one in tests.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError, Weak};

use egui::epaint::text::{Blob, MissingGlyphPolicy};
use egui::{FallbackRequest, FontInsert, FontProvider};

use crate::fonts::Stack;

/// Seconds before a failed file is fetched again; each further failure doubles it, up to
/// [`RETRY_MAX`].
const RETRY_FIRST: f64 = 2.0;
const RETRY_MAX: f64 = 600.0;

/// Called with a fetched file's bytes, or why it failed.
pub(in crate::fonts) type Done = Box<dyn FnOnce(Result<Vec<u8>, String>) + Send>;
/// Called when a timer fires.
pub(in crate::fonts) type Wake = Box<dyn FnOnce() + Send>;

/// Fetches files and keeps time for a [`Loader`].
pub(in crate::fonts) trait Fetcher: Send + Sync {
    /// Start fetching `url`, checked against `integrity` when not empty, and call `done` with its
    /// bytes or why it failed.
    fn fetch(&self, url: &'static str, integrity: &'static str, done: Done);
    /// Seconds since some fixed time.
    fn now(&self) -> f64;
    /// Call `wake` once, `seconds` from now.
    fn after(&self, seconds: f64, wake: Wake);
}

/// Install `stack`, add the files fetched so far, and fetch the UI language's faces.
pub(in crate::fonts) fn install(ctx: &egui::Context, mut stack: Stack, fetcher: Arc<dyn Fetcher>) {
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

#[cfg(target_arch = "wasm32")]
pub(in crate::fonts) fn loading(ctx: &egui::Context) -> bool {
    let loader: Option<Arc<Loader>> = ctx.data(|d| d.get_temp(id()));
    loader.is_some_and(|loader| !loader.lock().ui_pending.is_empty())
}

fn id() -> egui::Id {
    egui::Id::unique("craft_ui::fonts::served")
}

/// Make egui ask the providers again for the characters it found no face for. egui remembers those
/// misses until its fonts reload and has no call that only forgets them; changing the missing
/// glyph policy reloads the fonts, and setting it back keeps the app's policy.
fn forget_misses(ctx: &egui::Context) {
    let policy = ctx.missing_glyph_policy();
    let other = match policy {
        MissingGlyphPolicy::Tofu => MissingGlyphPolicy::Panic,
        MissingGlyphPolicy::Panic => MissingGlyphPolicy::Tofu,
    };
    ctx.set_missing_glyph_policy(other);
    ctx.set_missing_glyph_policy(policy);
}

/// The context's fetched files and the stack they go into. Also the provider asked for characters
/// no face has, which starts fetching the face that draws them.
struct Loader {
    ctx: egui::Context,
    me: Weak<Loader>,
    fetcher: Arc<dyn Fetcher>,
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
    fn of(ctx: &egui::Context, fetcher: Arc<dyn Fetcher>) -> Arc<Self> {
        let existing: Option<Arc<Self>> = ctx.data(|d| d.get_temp(id()));
        existing.unwrap_or_else(|| {
            let loader = Arc::new_cyclic(|me| Self { ctx: ctx.clone(), me: me.clone(), fetcher, state: Mutex::default() });
            ctx.data_mut(|d| d.insert_temp(id(), Arc::clone(&loader)));
            loader
        })
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
                    let delay = (RETRY_FIRST * 2f64.powi(attempts.saturating_sub(1).min(16) as i32)).min(RETRY_MAX);
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
        forget_misses(&self.ctx);
        if let Some(delay) = retry {
            let me = self.me.clone();
            self.fetcher.after(
                delay,
                Box::new(move || {
                    if let Some(me) = me.upgrade() {
                        forget_misses(&me.ctx);
                        me.ctx.request_repaint();
                    }
                }),
            );
        }
        self.ctx.request_repaint();
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
