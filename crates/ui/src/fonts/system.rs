//! A font index built once on a background thread and shared by every locale's provider.
//!
//! Lookups that may not block (the UI thread's) find nothing until the index is built: egui
//! draws with the faces it has, and every context that was turned away reloads its fonts when the
//! index arrives, which forgets egui's cached misses and repaints. If the worker cannot start or
//! the build panics, every lookup finds nothing from then on and egui keeps the faces it has.

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
        self.start_with(ctx, || std::thread::Builder::new().name("craft-ui system fonts".into()).spawn(|| self.finish()).map(|_| ()));
    }

    fn start_with(&'static self, ctx: Option<&egui::Context>, spawn: impl FnOnce() -> std::io::Result<()>) {
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
        if idle && let Err(error) = spawn() {
            // Never fall back to indexing on the caller: this may be the UI thread.
            // Wake blocking lookups too, and retain the existing faces and fallbacks.
            *self.lock() = State::Failed;
            self.built.notify_all();
            eprintln!("craft-ui: starting the system font index failed, so it adds no fonts: {error}");
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
    fn thread_creation_failure_wakes_waiters_without_building_and_preserves_fallbacks() {
        static BUILDS: AtomicUsize = AtomicUsize::new(0);
        fn build() -> Gated {
            BUILDS.fetch_add(1, Ordering::Relaxed);
            Gated
        }
        static INDEX: Shared<Gated> = Shared::new(build);
        let ctx = egui::Context::default();
        ctx.set_fonts(egui::FontDefinitions::empty());
        let system: Arc<dyn FontProvider> = Arc::new(Lookup::new(&INDEX, (), false));
        let fallback: Arc<dyn FontProvider> =
            Arc::new(|_: &FallbackRequest<'_>| Some(FontInsert::new("fallback", FontData::from_static(ARABIC), vec![])));
        super::super::providers::set_font_providers(&ctx, vec![system, fallback]);
        let (waiting, ready) = std::sync::mpsc::channel();
        let (finished, done) = std::sync::mpsc::channel();
        let mut worker = None;
        INDEX.start_with(Some(&ctx), || {
            worker = Some(std::thread::spawn(move || {
                let mut state = INDEX.lock();
                waiting.send(()).unwrap();
                // Holding the mutex until wait releases it ensures failure notification
                // cannot race ahead of this waiter actually waiting.
                while matches!(*state, State::Building { .. }) {
                    state = INDEX.built.wait(state).unwrap();
                }
                finished.send(matches!(*state, State::Failed)).unwrap();
            }));
            ready.recv_timeout(Duration::from_secs(10)).unwrap();
            Err(std::io::Error::other("injected thread creation failure"))
        });
        assert_eq!(done.recv_timeout(Duration::from_secs(10)), Ok(true), "failure must wake existing waiters");
        worker.unwrap().join().unwrap();
        assert_eq!(BUILDS.load(Ordering::Relaxed), 0, "never build the index on the caller's thread");
        assert!(INDEX.get(false).is_none());
        assert!(INDEX.get(true).is_none());
        INDEX.start_with(None, || panic!("failed indexing must not retry on every lookup"));
        ctx.run_ui(Default::default(), |_| {}).textures_delta.clear();
        ctx.fonts_mut(|fonts| {
            assert!(fonts.has_glyph(&FontId::proportional(14.0), 'ع'));
            assert_eq!(fonts.discovered_fonts().last().unwrap().name, "fallback");
        });
    }

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

    #[test]
    fn index_completion_before_registration_does_not_leave_cached_fallback() {
        static INDEX: Shared<Gated> = Shared::new(|| Gated);
        fn selected(ctx: &egui::Context) -> Option<String> {
            ctx.run_ui(Default::default(), |_| {}).textures_delta.clear();
            ctx.fonts_mut(|fonts| {
                fonts.has_glyph(&FontId::proportional(14.0), 'ع');
                fonts.discovered_fonts().last().map(|font| font.name.clone())
            })
        }
        fn start(ctx: &egui::Context) {
            // Model another thread using any already-visible providers, then the worker
            // finishing just before this context can register for completion reloads.
            selected(ctx);
            INDEX.finish();
            INDEX.start(Some(ctx));
        }
        *INDEX.lock() = State::Building { turned_away: false, contexts: Vec::new() };
        let ctx = egui::Context::default();
        ctx.set_fonts(egui::FontDefinitions::empty());
        let system: Arc<dyn FontProvider> = Arc::new(Lookup::new(&INDEX, (), false));
        let fallback: Arc<dyn FontProvider> =
            Arc::new(|_: &FallbackRequest<'_>| Some(FontInsert::new("temporary", FontData::from_static(ARABIC), vec![])));
        super::super::install_context_providers(&ctx, Some((system, start)), vec![fallback]);
        assert_eq!(selected(&ctx).as_deref(), Some("index:Arabic"), "completion before registration must not leave a cached fallback");
    }

    #[test]
    fn custom_provider_install_reloads_fallbacks_and_preserves_opt_out() {
        static OPEN_CUSTOM: AtomicBool = AtomicBool::new(false);
        fn build() -> Gated {
            while !OPEN_CUSTOM.load(Ordering::Acquire) {
                std::thread::sleep(Duration::from_millis(1));
            }
            Gated
        }
        static INDEX: Shared<Gated> = Shared::new(build);
        // Own the worker in this test so its completion means every context has reloaded,
        // not just that the index was published or the first context started drawing it.
        *INDEX.lock() = State::Building { turned_away: false, contexts: Vec::new() };
        let (finished, all_reloaded) = std::sync::mpsc::channel();
        let worker = std::thread::spawn(move || {
            INDEX.finish();
            finished.send(()).unwrap();
        });
        fn start(ctx: &egui::Context) {
            INDEX.start(Some(ctx));
        }
        let ctx = egui::Context::default();
        ctx.set_fonts(egui::FontDefinitions::empty());
        let disabled = egui::Context::default();
        disabled.set_fonts(egui::FontDefinitions::empty());
        let fallback: Arc<dyn FontProvider> =
            Arc::new(|_: &FallbackRequest<'_>| Some(FontInsert::new("temporary", FontData::from_static(ARABIC), vec![])));
        let ui = || Arc::new(Lookup::new(&INDEX, (), false)) as Arc<dyn FontProvider>;
        super::super::install_context_providers(&ctx, Some((ui(), start)), vec![fallback]);
        super::super::install_context_providers(&disabled, Some((ui(), start)), vec![]);
        let selected = |ctx: &egui::Context| {
            ctx.run_ui(Default::default(), |_| {}).textures_delta.clear();
            ctx.fonts_mut(|f| {
                f.has_glyph(&FontId::proportional(14.0), 'ع');
                f.discovered_fonts().last().map(|f| f.name.clone())
            })
        };
        assert_eq!(selected(&ctx).as_deref(), Some("temporary"));
        assert!(selected(&disabled).is_none());
        super::super::install_font_providers(&disabled, None, vec![]);
        OPEN_CUSTOM.store(true, Ordering::Release);
        all_reloaded.recv_timeout(Duration::from_secs(10)).expect("worker must finish reloading both contexts");
        worker.join().unwrap();
        assert_eq!(selected(&ctx).as_deref(), Some("index:Arabic"), "temporary fallback must be forgotten");
        assert!(selected(&disabled).is_none(), "index completion must not restore disabled providers");
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
