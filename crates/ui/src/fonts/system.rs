//! A font index built once on a background thread and shared by every locale's provider.
//!
//! Lookups that may not block (the UI thread's) find nothing until the index is built: egui
//! draws with the faces it has, and every context that was turned away reloads its fonts when the
//! index arrives, which forgets egui's cached misses and repaints. If the build panics, every
//! lookup finds nothing from then on and egui keeps the faces it has.

use std::sync::{Arc, Condvar, Mutex, MutexGuard, PoisonError};

use egui::{FallbackRequest, FontInsert, FontProvider};

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
}

impl<T: FontIndex> Shared<T> {
    pub(super) const fn new(build: fn() -> T) -> Self {
        Self { state: Mutex::new(State::Idle), built: Condvar::new(), build }
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
                super::providers::reload_font_providers(&ctx);
            }
        }
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

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
    use std::time::{Duration, Instant};

    use egui::{FontData, FontFamily, FontId, FontPriority, InsertFontFamily};

    use super::*;

    const ARABIC: &[u8] = include_bytes!("../../tests/fonts/NotoSansArabic-test.ttf");

    struct Gated;

    static OPEN: AtomicBool = AtomicBool::new(false);
    static BUILDS: AtomicUsize = AtomicUsize::new(0);

    fn build_when_open() -> Gated {
        while !OPEN.load(Ordering::Acquire) {
            std::thread::sleep(Duration::from_millis(1));
        }
        BUILDS.fetch_add(1, Ordering::Relaxed);
        Gated
    }

    impl FontIndex for Gated {
        type Locale = ();

        fn font_for(&mut self, request: &FallbackRequest<'_>, _: &()) -> Option<FontInsert> {
            Some(FontInsert::new(
                "index:Arabic",
                FontData::from_static(ARABIC),
                vec![InsertFontFamily { family: request.family.clone(), priority: FontPriority::Lowest }],
            ))
        }
    }

    static GATED: Shared<Gated> = Shared::new(build_when_open);

    #[test]
    fn ui_lookups_skip_the_index_until_it_is_built_then_the_context_reloads() {
        let ctx = egui::Context::default();
        let font_id = FontId::new(14.0, FontFamily::Proportional);
        let ui: Arc<dyn FontProvider> = Arc::new(Lookup::new(&GATED, (), false));
        super::super::providers::set_font_providers(&ctx, vec![ui]);
        GATED.start(Some(&ctx));
        let draws_arabic = || {
            ctx.run_ui(egui::RawInput::default(), |_| {}).textures_delta.clear();
            ctx.fonts_mut(|f| f.has_glyph(&font_id, 'ع'))
        };

        assert!(!draws_arabic(), "a lookup before the index is built finds nothing, without blocking");
        let other_locale = Lookup::new(&GATED, (), false);
        let request = FallbackRequest { cluster: "ع", family: &FontFamily::Proportional };
        assert!(other_locale.font_for(&request).is_none());

        OPEN.store(true, Ordering::Release);
        let checks = Lookup::new(&GATED, (), true);
        assert!(checks.font_for(&request).is_some(), "a waiting lookup blocks until the index is built");
        let deadline = Instant::now() + Duration::from_secs(10);
        while !draws_arabic() {
            assert!(Instant::now() < deadline, "the context reloads its fonts once the index is built");
            std::thread::sleep(Duration::from_millis(5));
        }
        assert!(other_locale.font_for(&request).is_some());
        assert_eq!(BUILDS.load(Ordering::Relaxed), 1, "every locale shares one index");
    }

    fn build_panics() -> Gated {
        panic!("stand-in index build failure");
    }

    static PANICS: Shared<Gated> = Shared::new(build_panics);

    #[test]
    fn a_panicking_build_leaves_lookups_finding_nothing() {
        let (sent, received) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let request = FallbackRequest { cluster: "ع", family: &FontFamily::Proportional };
            let found = Lookup::new(&PANICS, (), true).font_for(&request).is_some();
            let _ = sent.send(found);
        });
        let found = received.recv_timeout(Duration::from_secs(10));
        assert_eq!(found, Ok(false), "a waiting lookup returns once the build has failed");
        let request = FallbackRequest { cluster: "ع", family: &FontFamily::Proportional };
        assert!(Lookup::new(&PANICS, (), false).font_for(&request).is_none());
        assert!(Lookup::new(&PANICS, (), true).font_for(&request).is_none());
    }
}
