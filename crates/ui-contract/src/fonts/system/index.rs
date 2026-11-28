//! A font index built once on a background thread and shared by every locale's provider.
//!
//! Lookups that may not block (the UI thread's) find nothing until the index is built: egui
//! draws with the faces it has, and every context that was turned away reloads its fonts when the
//! index arrives, which forgets egui's cached misses and repaints. If the build panics, every
//! lookup finds nothing from then on and egui keeps the faces it has. Either way the index's
//! `built` hook runs next, on the thread that built it.

use std::sync::{Arc, Condvar, Mutex, MutexGuard, PoisonError};

use egui::{FallbackRequest, FontInsert, FontProvider};

use crate::runtime::egui::fonts as egui_fonts;

/// A font index a [`Lookup`] asks.
pub(super) trait FontIndex: Send + 'static {
    /// The provider's preference between fonts for one script, e.g. a locale.
    type Locale: Send + Sync + 'static;

    fn font_for(&mut self, request: &FallbackRequest<'_>, locale: &Self::Locale) -> Option<FontInsert>;
}

enum State<T> {
    Idle,
    /// `turned_away`: a lookup found nothing because the index wasn't built, so `contexts` reload
    /// their fonts when it is.
    Building {
        turned_away: bool,
        contexts: Vec<egui::Context>,
    },
    Ready(Arc<Mutex<T>>),
    Failed,
}

/// An index built at most once per process, on first use.
pub(super) struct Shared<T> {
    state: Mutex<State<T>>,
    built: Condvar,
    build: fn() -> T,
    /// Runs once the index is built or its build failed, outside every lock.
    on_built: fn(),
}

impl<T: FontIndex> Shared<T> {
    pub(super) const fn new(build: fn() -> T, on_built: fn()) -> Self {
        Self { state: Mutex::new(State::Idle), built: Condvar::new(), build, on_built }
    }

    /// Start building the index on a background thread unless it is built or building. `ctx`
    /// reloads its fonts when the index arrives if a lookup was turned away meanwhile.
    pub(super) fn start(&'static self, ctx: Option<&egui::Context>) {
        let idle = {
            let mut state = self.lock();
            let idle = matches!(*state, State::Idle);
            if idle {
                *state = State::Building { turned_away: false, contexts: Vec::new() };
            }
            if let (State::Building { contexts, .. }, Some(ctx)) = (&mut *state, ctx) {
                contexts.push(ctx.clone());
            }
            idle
        };
        if idle && std::thread::Builder::new().name("craft-ui system fonts".into()).spawn(|| self.finish()).is_err() {
            self.finish();
        }
    }

    /// The index, or `None` while it builds unless `wait`, which blocks until it is built, or
    /// `None` if its build failed.
    fn get(&'static self, wait: bool) -> Option<Arc<Mutex<T>>> {
        self.start(None);
        let mut state = self.lock();
        loop {
            match &mut *state {
                State::Ready(index) => return Some(Arc::clone(index)),
                State::Failed => return None,
                State::Building { turned_away, .. } if !wait => {
                    *turned_away = true;
                    return None;
                }
                _ => state = self.built.wait(state).unwrap_or_else(PoisonError::into_inner),
            }
        }
    }

    /// Run `f` on the index, waiting until it is built; `None` if its build failed.
    pub(super) fn with<R>(&'static self, f: impl FnOnce(&mut T) -> R) -> Option<R> {
        let index = self.get(true)?;
        let mut index = index.lock().unwrap_or_else(PoisonError::into_inner);
        Some(f(&mut index))
    }

    fn finish(&self) {
        let built = std::panic::catch_unwind(self.build).map_err(|payload| {
            let message = payload
                .downcast_ref::<String>()
                .map(String::as_str)
                .or_else(|| payload.downcast_ref::<&str>().copied())
                .unwrap_or("non-string panic payload");
            eprintln!("craft-ui: building the system font index failed, so it adds no fonts: {message}");
        });
        let next = built.map_or(State::Failed, |index| State::Ready(Arc::new(Mutex::new(index))));
        let previous = std::mem::replace(&mut *self.lock(), next);
        self.built.notify_all();
        // Outside the lock: the UI thread holds the context's fonts while it asks for the index.
        if let State::Building { turned_away: true, contexts } = previous {
            for ctx in contexts {
                egui_fonts::reload_providers(&ctx);
            }
        }
        (self.on_built)();
    }

    fn lock(&self) -> MutexGuard<'_, State<T>> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

/// One locale's provider over a [`Shared`] index.
pub(super) struct Lookup<T: FontIndex> {
    index: &'static Shared<T>,
    locale: T::Locale,
    /// Block until the index is built, for checks; the UI thread doesn't.
    wait: bool,
}

impl<T: FontIndex> Lookup<T> {
    pub(super) fn new(index: &'static Shared<T>, locale: T::Locale, wait: bool) -> Self {
        Self { index, locale, wait }
    }
}

impl<T: FontIndex> FontProvider for Lookup<T> {
    fn font_for(&self, request: &FallbackRequest<'_>) -> Option<FontInsert> {
        let index = self.index.get(self.wait)?;
        let mut index = index.lock().unwrap_or_else(PoisonError::into_inner);
        index.font_for(request, &self.locale)
    }
}
