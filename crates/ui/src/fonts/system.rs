//! A font index built on a background thread, shared by every locale's provider, and rebuilt when
//! the fonts it was built from change.
//!
//! Lookups that may not block (the UI thread's) find nothing until the index is built: egui
//! draws with the faces it has, and every context that was turned away reloads its fonts when the
//! index arrives, which forgets egui's cached misses and repaints. If the build panics, every
//! lookup finds nothing and egui keeps the faces it has, until the fonts change and the build is
//! tried again.
//!
//! The index remembers the generation of the fonts it was built from. When the fonts' source
//! reports a change, and on every lookup and every pass of a context that started the index, it is
//! compared with the current generation, which is cheap. On a change the index is rebuilt in the
//! background while lookups keep asking the old one. Once the new index is in, every context that
//! started the index repaints, idle or not, and reloads its fonts at the start of that pass, so the
//! characters it had no font for are asked again; the faces egui already has stay. A rebuild that
//! panics keeps the old index until the fonts change again.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex, MutexGuard, PoisonError};

use egui::{FallbackRequest, FontInsert, FontProvider};

/// A font index a [`Lookup`] asks.
pub(super) trait FontIndex: Send + 'static {
    /// The provider's preference between fonts for one script, e.g. a locale.
    type Locale: Send + Sync + 'static;

    /// The generation of the fonts the index was built from, as [`Source::generation`] counts
    /// them.
    fn generation(&self) -> u64;

    fn font_for(&mut self, request: &FallbackRequest<'_>, locale: &Self::Locale) -> Option<FontInsert>;
}

/// Where an index's fonts come from: how to read their generation and hear of a change.
pub(super) struct Source {
    /// The fonts' current generation. It must not block: every UI pass reads it.
    pub(super) generation: fn() -> u64,
    /// Registers a callback for every change of the generation, e.g. a rescan on another thread.
    pub(super) on_change: fn(Box<dyn Fn() + Send + Sync>),
}

enum State<T> {
    Idle,
    /// `turned_away`: a lookup found nothing because the index wasn't built, so the contexts
    /// reload their fonts when it is.
    Building {
        turned_away: bool,
    },
    Ready {
        index: Arc<Mutex<T>>,
        generation: u64,
    },
    /// The fonts changed since `index` was built. Lookups that don't wait ask it until the new
    /// one is in.
    Rebuilding {
        index: Arc<Mutex<T>>,
    },
    /// The build at `generation` panicked; a later generation tries again.
    Failed {
        generation: u64,
    },
}

/// An index built on first use, and again whenever the fonts' generation changes.
pub(super) struct Shared<T> {
    state: Mutex<State<T>>,
    built: Condvar,
    build: fn() -> T,
    source: Source,
    /// Counts the rebuilds that replaced the index. A context reloads its fonts when this moves
    /// past the count it last saw.
    replaced: AtomicU64,
    /// The contexts that started the index, to repaint after a rebuild. egui has no weak handle to
    /// a context, so each stays until its integration calls [`egui::Context::on_exit`].
    contexts: Mutex<Vec<egui::Context>>,
}

impl<T: FontIndex> Shared<T> {
    pub(super) const fn new(build: fn() -> T, source: Source) -> Self {
        Self { state: Mutex::new(State::Idle), built: Condvar::new(), build, source, replaced: AtomicU64::new(0), contexts: Mutex::new(Vec::new()) }
    }

    /// Start building the index on a background thread unless it is built or building. `ctx`
    /// reloads its fonts when the index arrives if a lookup was turned away meanwhile, and follows
    /// the fonts' generation from then on.
    pub(super) fn start(&'static self, ctx: Option<&egui::Context>) {
        if let Some(ctx) = ctx {
            let replaced = self.replaced.load(Ordering::Acquire);
            ctx.data_mut(|d| d.insert_temp(self.seen_id(), replaced));
            let mut contexts = self.contexts();
            if !contexts.contains(ctx) {
                contexts.push(ctx.clone());
            }
            drop(contexts);
            ctx.add_plugin(Follow(self));
        }
        let idle = {
            let mut state = self.lock();
            let idle = matches!(*state, State::Idle);
            if idle {
                *state = State::Building { turned_away: false };
            }
            idle
        };
        if idle {
            (self.source.on_change)(Box::new(|| self.follow()));
            self.spawn_build();
        }
    }

    /// Rebuild the index in the background if the fonts changed since it was built, or retry a
    /// failed build at a new generation.
    fn follow(&'static self) {
        let current = (self.source.generation)();
        let rebuild = {
            let mut state = self.lock();
            match &mut *state {
                State::Ready { index, generation } if *generation != current => {
                    let index = Arc::clone(index);
                    *state = State::Rebuilding { index };
                    true
                }
                State::Failed { generation } if *generation != current => {
                    // Every lookup since the failure found nothing.
                    *state = State::Building { turned_away: true };
                    true
                }
                _ => false,
            }
        };
        if rebuild {
            self.spawn_build();
        }
    }

    /// The index, or `None` while it builds unless `wait`, which blocks until it is built, or
    /// `None` if its build failed. While it is rebuilt, `wait` blocks for the new index and a
    /// lookup without gets the old one.
    fn get(&'static self, wait: bool) -> Option<Arc<Mutex<T>>> {
        self.start(None);
        self.follow();
        let mut state = self.lock();
        loop {
            match &mut *state {
                State::Ready { index, .. } => return Some(Arc::clone(index)),
                State::Rebuilding { index } if !wait => return Some(Arc::clone(index)),
                State::Failed { .. } => return None,
                State::Building { turned_away } if !wait => {
                    *turned_away = true;
                    return None;
                }
                _ => state = self.built.wait(state).unwrap_or_else(PoisonError::into_inner),
            }
        }
    }

    fn spawn_build(&'static self) {
        if std::thread::Builder::new().name("craft-ui system fonts".into()).spawn(|| self.finish()).is_err() {
            self.finish();
        }
    }

    fn finish(&'static self) {
        let before = (self.source.generation)();
        let built = std::panic::catch_unwind(self.build).map_err(|payload| {
            let message = payload
                .downcast_ref::<String>()
                .map(String::as_str)
                .or_else(|| payload.downcast_ref::<&str>().copied())
                .unwrap_or("non-string panic payload");
            eprintln!("craft-ui: building the system font index failed: {message}");
        });
        let ready = |index: T| State::Ready { generation: index.generation(), index: Arc::new(Mutex::new(index)) };
        let mut state = self.lock();
        let (next, reload, repaint) = match (built, std::mem::replace(&mut *state, State::Idle)) {
            (Ok(index), State::Rebuilding { .. }) => (ready(index), false, true),
            (Err(()), State::Rebuilding { index }) => (State::Ready { index, generation: before }, false, false),
            (Ok(index), State::Building { turned_away }) => (ready(index), turned_away, false),
            (Ok(index), _) => (ready(index), false, false),
            (Err(()), _) => (State::Failed { generation: before }, false, false),
        };
        *state = next;
        if repaint {
            self.replaced.fetch_add(1, Ordering::Release);
        }
        drop(state);
        self.built.notify_all();
        // Outside the lock: the UI thread holds the context's fonts while it asks for the index.
        // A repainted context reloads its fonts in its next pass (see `Follow`).
        let contexts = if reload || repaint { self.contexts().clone() } else { Vec::new() };
        for ctx in contexts {
            if reload {
                super::providers::reload_font_providers(&ctx);
            } else {
                ctx.request_repaint();
            }
        }
        // The fonts may have changed again while this build ran.
        self.follow();
    }

    /// Where a context keeps the [`Self::replaced`] count its fonts were last loaded at.
    fn seen_id(&self) -> egui::Id {
        egui::Id::unique(("craft_ui::fonts::system::seen", std::ptr::from_ref(self) as usize))
    }

    fn lock(&self) -> MutexGuard<'_, State<T>> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn contexts(&self) -> MutexGuard<'_, Vec<egui::Context>> {
        self.contexts.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

/// Follows the fonts for a context that started a [`Shared`] index: at the start of each pass it
/// has the index rebuilt if they changed, and reloads the context's fonts once a new index is in.
struct Follow<T: 'static>(&'static Shared<T>);

impl<T: FontIndex> egui::plugin::Plugin for Follow<T> {
    fn debug_name(&self) -> &'static str {
        "craft_ui::fonts::system::Follow"
    }

    fn on_begin_pass(&mut self, ui: &mut egui::Ui) {
        let ctx = ui.ctx().clone();
        self.0.follow();
        let replaced = self.0.replaced.load(Ordering::Acquire);
        let seen = ctx.data_mut(|d| std::mem::replace(d.get_temp_mut_or_insert_with(self.0.seen_id(), || replaced), replaced));
        if seen != replaced {
            super::providers::reload_font_providers(&ctx);
        }
    }

    fn on_exit(&mut self, ctx: &egui::Context) {
        self.0.contexts().retain(|c| c != ctx);
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
    use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
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

        fn generation(&self) -> u64 {
            0
        }

        fn font_for(&mut self, request: &FallbackRequest<'_>, _: &()) -> Option<FontInsert> {
            Some(FontInsert::new(
                "index:Arabic",
                FontData::from_static(ARABIC),
                vec![InsertFontFamily { family: request.family.clone(), priority: FontPriority::Lowest }],
            ))
        }
    }

    fn unchanging() -> u64 {
        0
    }

    fn never_changes(_: Box<dyn Fn() + Send + Sync>) {}

    const UNCHANGING: Source = Source { generation: unchanging, on_change: never_changes };

    static GATED: Shared<Gated> = Shared::new(build_when_open, UNCHANGING);

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

    static PANICS: Shared<Gated> = Shared::new(build_panics, UNCHANGING);

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

    /// An index of the fonts at a generation; Arabic is installed from generation 2.
    struct Installed(u64);

    static FONTS: AtomicU64 = AtomicU64::new(1);
    static INSTALLED_BUILDS: AtomicUsize = AtomicUsize::new(0);

    fn fonts_generation() -> u64 {
        FONTS.load(Ordering::Acquire)
    }

    fn build_installed() -> Installed {
        INSTALLED_BUILDS.fetch_add(1, Ordering::Relaxed);
        Installed(fonts_generation())
    }

    impl FontIndex for Installed {
        type Locale = ();

        fn generation(&self) -> u64 {
            self.0
        }

        fn font_for(&mut self, request: &FallbackRequest<'_>, _: &()) -> Option<FontInsert> {
            if self.0 < 2 {
                return None;
            }
            Gated.font_for(request, &())
        }
    }

    static INSTALLED: Shared<Installed> = Shared::new(build_installed, Source { generation: fonts_generation, on_change: never_changes });

    #[test]
    fn a_change_in_the_fonts_rebuilds_the_index_and_the_context_reloads() {
        let ctx = egui::Context::default();
        let font_id = FontId::new(14.0, FontFamily::Proportional);
        let ui: Arc<dyn FontProvider> = Arc::new(Lookup::new(&INSTALLED, (), false));
        super::super::providers::set_font_providers(&ctx, vec![ui]);
        INSTALLED.start(Some(&ctx));
        let draws_arabic = || {
            ctx.run_ui(egui::RawInput::default(), |_| {}).textures_delta.clear();
            ctx.fonts_mut(|f| f.has_glyph(&font_id, 'ع'))
        };
        let request = FallbackRequest { cluster: "ع", family: &FontFamily::Proportional };
        assert!(Lookup::new(&INSTALLED, (), true).font_for(&request).is_none(), "no Arabic font at generation 1");
        for _ in 0..3 {
            assert!(!draws_arabic());
        }
        assert_eq!(INSTALLED_BUILDS.load(Ordering::Relaxed), 1, "an unchanged generation keeps the index");

        FONTS.store(2, Ordering::Release);
        let deadline = Instant::now() + Duration::from_secs(10);
        while !draws_arabic() {
            assert!(Instant::now() < deadline, "the context reloads its fonts once the index is rebuilt");
            std::thread::sleep(Duration::from_millis(5));
        }
        assert_eq!(INSTALLED_BUILDS.load(Ordering::Relaxed), 2, "one rebuild per change");
    }

    static IDLE_FONTS: AtomicU64 = AtomicU64::new(1);
    static IDLE_BUILDS: AtomicUsize = AtomicUsize::new(0);
    static IDLE_CALLBACKS: Mutex<Vec<Box<dyn Fn() + Send + Sync>>> = Mutex::new(Vec::new());

    fn idle_generation() -> u64 {
        IDLE_FONTS.load(Ordering::Acquire)
    }

    fn build_idle() -> Installed {
        IDLE_BUILDS.fetch_add(1, Ordering::Relaxed);
        Installed(idle_generation())
    }

    fn on_idle_change(callback: Box<dyn Fn() + Send + Sync>) {
        IDLE_CALLBACKS.lock().unwrap().push(callback);
    }

    static IDLE: Shared<Installed> = Shared::new(build_idle, Source { generation: idle_generation, on_change: on_idle_change });

    #[test]
    fn an_idle_context_repaints_when_the_fonts_change() {
        let ctx = egui::Context::default();
        let ui: Arc<dyn FontProvider> = Arc::new(Lookup::new(&IDLE, (), false));
        super::super::providers::set_font_providers(&ctx, vec![ui]);
        IDLE.start(Some(&ctx));
        let request = FallbackRequest { cluster: "ع", family: &FontFamily::Proportional };
        assert!(Lookup::new(&IDLE, (), true).font_for(&request).is_none());
        let draws_arabic = || {
            ctx.run_ui(egui::RawInput::default(), |_| {}).textures_delta.clear();
            ctx.fonts_mut(|f| f.has_glyph(&FontId::new(14.0, FontFamily::Proportional), 'ع'))
        };
        assert!(!draws_arabic());
        // Idle: no repaint pending, as an app between inputs. egui calls the callback only for a
        // repaint requested after that.
        for _ in 0..10 {
            if !ctx.has_requested_repaint() {
                break;
            }
            assert!(!draws_arabic());
        }
        assert!(!ctx.has_requested_repaint(), "the context settles");
        let woken = Arc::new(AtomicBool::new(false));
        let wake = Arc::clone(&woken);
        ctx.set_request_repaint_callback(move |_| wake.store(true, Ordering::Release));

        // A rescan on another thread, with no pass running.
        IDLE_FONTS.store(2, Ordering::Release);
        for callback in IDLE_CALLBACKS.lock().unwrap().iter() {
            callback();
        }
        let deadline = Instant::now() + Duration::from_secs(10);
        while !woken.load(Ordering::Acquire) {
            assert!(Instant::now() < deadline, "the context is asked to repaint once the index is rebuilt");
            std::thread::sleep(Duration::from_millis(5));
        }
        assert_eq!(IDLE_BUILDS.load(Ordering::Relaxed), 2);
        draws_arabic();
        assert!(draws_arabic(), "the repainted context reloads its fonts");
    }

    fn build_unless_first() -> Installed {
        let generation = FLAKY_FONTS.load(Ordering::Acquire);
        assert!(generation > 1, "stand-in index build failure");
        Installed(generation)
    }

    static FLAKY_FONTS: AtomicU64 = AtomicU64::new(1);

    fn flaky_generation() -> u64 {
        FLAKY_FONTS.load(Ordering::Acquire)
    }

    static FLAKY: Shared<Installed> = Shared::new(build_unless_first, Source { generation: flaky_generation, on_change: never_changes });

    #[test]
    fn a_failed_build_is_tried_again_when_the_fonts_change() {
        let request = FallbackRequest { cluster: "ع", family: &FontFamily::Proportional };
        assert!(Lookup::new(&FLAKY, (), true).font_for(&request).is_none());
        assert!(Lookup::new(&FLAKY, (), true).font_for(&request).is_none(), "an unchanged generation doesn't retry");
        FLAKY_FONTS.store(2, Ordering::Release);
        assert!(Lookup::new(&FLAKY, (), true).font_for(&request).is_some(), "a new generation builds again");
    }
}
