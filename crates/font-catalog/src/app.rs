//! What every Craft app does with the catalog: start it at launch, follow the suite's font store,
//! refresh it when a window is activated, and learn which families changed so it lays out only
//! the documents they affect.
//!
//! ```no_run
//! use std::sync::Arc;
//! use craft_font_catalog::app::{self, AppFonts};
//!
//! // At launch, before the first window. `on_change` wakes the app's event loop.
//! let wake: app::OnChange = Arc::new(|_| { /* request a repaint */ });
//! let catalog = app::start(AppFonts { dirs: vec!["/path/to/prefs/Fonts".into()], on_change: Some(wake), ..AppFonts::default() });
//! let mut seen = catalog.generation();
//!
//! // When a window is activated.
//! app::on_activated();
//!
//! // Where the app checks for work (each frame, or after `on_change` woke it).
//! if catalog.generation() != seen {
//!     let changes = app::changes_since(seen);
//!     seen = changes.generation;
//!     // Re-resolve the families each document asked for that were missing or nearest-style
//!     // (all of them when `changes.full`), and lay out the documents where one now resolves
//!     // differently or uses a removed or changed family.
//! }
//! ```
//!
//! - Desktop: [`start`] adds the app's folders, scans on a background thread and polls the font
//!   store's folders ([`Catalog::store_changed`]), so a family the launcher installs shows at
//!   once. [`on_activated`] runs [`Catalog::rescan_if_changed`] on a worker thread for every other
//!   folder, and a full [`Catalog::rescan`] at most every [`AppFonts::full_rescan_every`], for
//!   fonts a font service (Adobe Fonts, FontBase) activates from folders no scan has seen.
//! - CLI: [`configure`] instead of [`start`]: the first lookup scans and waits for it, and nothing
//!   polls.
//! - wasm: there are no folders, no font service and no threads. Fonts come in through
//!   [`Catalog::add_bytes`]; [`start`] polls nothing and [`on_activated`] does nothing.

use std::path::PathBuf;
use std::sync::{Arc, Mutex, MutexGuard, OnceLock, PoisonError, Weak};
use std::time::{Duration, Instant};

use crate::{Catalog, Changes};

/// How often [`on_activated`] runs a full [`Catalog::rescan`] at most, by default. A full rescan
/// lists the font service's files again: about 60 ms with 150 font files and 0.5 s with 4,000 on
/// Linux, on the worker thread.
pub const FULL_RESCAN_EVERY: Duration = Duration::from_secs(5 * 60);

/// How often [`start`] checks the font store's folders, by default. A check reads the
/// modification time of each folder in the store, one per installed family.
pub const STORE_POLL: Duration = Duration::from_secs(1);

/// Called on a worker thread after a family was added, removed or changed.
pub type OnChange = Arc<dyn Fn(&Catalog) + Send + Sync>;

/// The app's fonts, for [`start`].
#[derive(Clone)]
pub struct AppFonts {
    /// The app's own font folders (`<prefs>/Fonts`, folders the user added).
    pub dirs: Vec<PathBuf>,
    /// How often [`on_activated`] runs a full [`Catalog::rescan`] at most ([`Duration::MAX`]:
    /// never).
    pub full_rescan_every: Duration,
    /// How often [`start`] checks the font store's folders; `None`: only on activation.
    pub store_poll: Option<Duration>,
    /// Called after a store change or [`on_activated`] added, removed or changed a family, to wake
    /// an idle window so it checks [`changes_since`]. Keep it short: activations do nothing while
    /// it runs.
    pub on_change: Option<OnChange>,
}

impl Default for AppFonts {
    fn default() -> Self {
        Self { dirs: Vec::new(), full_rescan_every: FULL_RESCAN_EVERY, store_poll: Some(STORE_POLL), on_change: None }
    }
}

/// Start the global catalog for a desktop or web app: [`configure`], the first scan on a
/// background thread, and the font store poll. It doesn't wait for the scan. Call it once, before
/// the first window and before anything else uses [`Catalog::global`] or [`on_activated`].
pub fn start(fonts: AppFonts) -> &'static Arc<Catalog> {
    let poll = fonts.store_poll;
    let catalog = configure(fonts);
    catalog.scan_in_background();
    if let Some(interval) = poll
        && !cfg!(target_arch = "wasm32")
    {
        refresh().watch_store(interval);
    }
    catalog
}

/// [`start`] without the background scan or the store poll, for a CLI: the first lookup scans and
/// waits for it.
pub fn configure(fonts: AppFonts) -> &'static Arc<Catalog> {
    let catalog = Catalog::global();
    catalog.set_app_dirs(fonts.dirs);
    let _ = REFRESH.set(Arc::new(Refresh::new(Arc::clone(catalog), fonts.full_rescan_every, fonts.on_change)));
    catalog
}

/// For a window's activation: on a worker thread, [`Catalog::rescan_if_changed`], or a full
/// [`Catalog::rescan`] when the last was [`AppFonts::full_rescan_every`] ago or more. It never
/// blocks: while a worker runs, another activation does nothing. Nothing happens before the
/// first scan has finished, or on wasm. Returns whether a worker started.
pub fn on_activated() -> bool {
    if cfg!(target_arch = "wasm32") {
        return false;
    }
    refresh().activated_at(Instant::now())
}

/// A full [`Catalog::rescan`] now, on this thread, for the app's "Reload fonts" command. It counts
/// as [`on_activated`]'s full rescan, so the next activation doesn't run one again at once, and
/// calls [`AppFonts::on_change`] when a family changed. Returns what changed.
pub fn rescan_now() -> Changes {
    refresh().rescan_now(if cfg!(target_arch = "wasm32") { None } else { Some(Instant::now()) })
}

/// [`Catalog::changes_since`] of the global catalog.
pub fn changes_since(generation: u64) -> Changes {
    Catalog::global().changes_since(generation)
}

static REFRESH: OnceLock<Arc<Refresh>> = OnceLock::new();

fn refresh() -> &'static Arc<Refresh> {
    REFRESH.get_or_init(|| Arc::new(Refresh::new(Arc::clone(Catalog::global()), FULL_RESCAN_EVERY, None)))
}

/// The rescans one catalog runs for activations and font store changes, one at a time.
pub(crate) struct Refresh {
    catalog: Arc<Catalog>,
    full_every: Duration,
    on_change: Option<OnChange>,
    state: Mutex<RefreshState>,
}

#[derive(Default)]
struct RefreshState {
    running: bool,
    /// When the last full rescan started; the first activation counts the first scan as one.
    /// Not taken at construction: `Instant::now` panics on wasm.
    last_full: Option<Instant>,
}

fn lock(m: &Mutex<RefreshState>) -> MutexGuard<'_, RefreshState> {
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

/// Clears [`RefreshState::running`] when the rescan ends, panicking included.
struct Running<'a>(&'a Mutex<RefreshState>);

impl Drop for Running<'_> {
    fn drop(&mut self) {
        lock(self.0).running = false;
    }
}

impl Refresh {
    pub(crate) fn new(catalog: Arc<Catalog>, full_every: Duration, on_change: Option<OnChange>) -> Self {
        Self { catalog, full_every, on_change, state: Mutex::default() }
    }

    pub(crate) fn activated_at(self: &Arc<Self>, now: Instant) -> bool {
        if !self.catalog.is_ready() {
            return false;
        }
        let (full, last_full) = {
            let mut state = lock(&self.state);
            if state.running {
                return false;
            }
            let last_full = *state.last_full.get_or_insert(now);
            let full = now.saturating_duration_since(last_full) >= self.full_every;
            if full {
                state.last_full = Some(now);
            }
            state.running = true;
            (full, last_full)
        };
        let me = Arc::clone(self);
        let spawned = std::thread::Builder::new().name("font-catalog-refresh".into()).spawn(move || {
            let _running = Running(&me.state);
            let before = me.catalog.generation();
            if full {
                me.catalog.rescan();
            } else {
                me.catalog.rescan_if_changed();
            }
            me.changed_since(before);
        });
        if spawned.is_err() {
            let mut state = lock(&self.state);
            state.running = false;
            state.last_full = Some(last_full);
            return false;
        }
        true
    }

    /// Check the font store's folders every `interval` on a background thread, and rescan when
    /// they changed. A check that finds an activation's rescan running skips its turn. Stops when
    /// the `Refresh` is dropped.
    pub(crate) fn watch_store(self: &Arc<Self>, interval: Duration) {
        let me: Weak<Self> = Arc::downgrade(self);
        let _ = std::thread::Builder::new().name("font-catalog-store".into()).spawn(move || {
            loop {
                std::thread::sleep(interval);
                let Some(refresh) = me.upgrade() else { return };
                if !refresh.catalog.store_changed() {
                    continue;
                }
                {
                    let mut state = lock(&refresh.state);
                    if state.running {
                        continue;
                    }
                    state.running = true;
                    // A rescan lists the font service's files again: it counts as the full one.
                    state.last_full = Some(Instant::now());
                }
                let _running = Running(&refresh.state);
                let before = refresh.catalog.generation();
                refresh.catalog.rescan();
                refresh.changed_since(before);
            }
        });
    }

    /// [`rescan_now`] at `now` (`None` on wasm, which has no clock).
    pub(crate) fn rescan_now(&self, now: Option<Instant>) -> Changes {
        if let Some(now) = now {
            lock(&self.state).last_full = Some(now);
        }
        let before = self.catalog.generation();
        self.catalog.rescan();
        self.changed_since(before)
    }

    /// The changes since `before`, after calling `on_change` when there are any.
    fn changed_since(&self, before: u64) -> Changes {
        let changes = self.catalog.changes_since(before);
        if let Some(on_change) = &self.on_change
            && !changes.is_empty()
        {
            on_change(&self.catalog);
        }
        changes
    }
}
