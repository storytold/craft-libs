//! What every Craft app does with the catalog: start it at launch, refresh it when a window is
//! activated, and learn which families changed so it lays out only the documents they affect.
//!
//! ```no_run
//! use craft_font_catalog::app::{self, AppFonts, Office};
//!
//! // At launch, before the first window.
//! let started = app::start(AppFonts { dirs: vec!["/path/to/prefs/Fonts".into()], office: Office::All, ..AppFonts::default() });
//! let mut seen = started.catalog.generation();
//!
//! // When a window is activated.
//! app::on_activated();
//!
//! // The app's "Reload fonts" command.
//! app::rescan_now();
//!
//! // Where the app checks for work (each frame, or from `AppFonts::on_change`).
//! if started.catalog.generation() != seen {
//!     let changes = app::changes_since(seen);
//!     seen = changes.generation;
//!     // Re-resolve the families each document asked for that were missing or nearest-style
//!     // (all of them when `changes.full`), and lay out the documents where one now resolves
//!     // differently or uses a removed or changed family.
//! }
//! ```
//!
//! - Desktop: [`start`] adds the app's folders and bundled faces and scans on a background
//!   thread. [`on_activated`] runs [`Catalog::rescan_if_changed`] on a worker thread, and a full
//!   [`Catalog::rescan`] at most every [`AppFonts::full_rescan_every`], for fonts a font service
//!   (Adobe Fonts, FontBase) activates from folders no scan has seen. craft-ui's UI fallback
//!   follows rescans by itself.
//! - CLI: [`configure`] instead of [`start`], so the first lookup scans and waits for it, and no
//!   activation hooks or [`Catalog::watch`]. A "no system fonts" switch makes
//!   [`Catalog::without_installed_fonts`] the global catalog first ([`Catalog::init_global`]).
//!   On macOS, use [`Office::Bundled`] at most: reading Office's CloudFonts folder can raise a
//!   privacy prompt, which a CLI can't answer.
//! - wasm: there are no folders, no font service and no threads. [`start`] adds the bundled faces;
//!   other fonts come in through [`Catalog::add_bytes`]. [`on_activated`] does nothing.

use std::path::PathBuf;
use std::sync::{Arc, Mutex, MutexGuard, OnceLock, PoisonError};
use std::time::{Duration, Instant};

use crate::env::{office_bundled_font_dirs, office_cloud_font_dirs};
use crate::{Catalog, Changes, FontFileError, Limits, Source};

/// How often [`on_activated`] runs a full [`Catalog::rescan`] at most, by default. A full rescan
/// lists the font service's files again: about 60 ms with 150 font files and 0.5 s with 4,000 on
/// Linux, on the worker thread.
pub const FULL_RESCAN_EVERY: Duration = Duration::from_secs(5 * 60);

/// Called on the worker thread after [`on_activated`] changed a family.
pub type OnChange = Arc<dyn Fn(&Catalog) + Send + Sync>;

/// Which of Office's font folders an app reads ([`crate::office_font_dirs`]), for the apps that
/// open Office documents.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Office {
    /// None.
    #[default]
    None,
    /// The fonts in the Office apps' bundles (macOS `DFonts`; Windows has none), whose reading
    /// raises no privacy prompt.
    Bundled,
    /// Those and the cloud fonts Office downloads (macOS: in Office's group container, which can
    /// raise a privacy prompt; Windows: under `%LOCALAPPDATA%`).
    All,
}

/// The app's fonts, for [`start`].
#[derive(Clone)]
pub struct AppFonts {
    /// The app's own font folders (`<prefs>/Fonts`, folders the user added).
    pub dirs: Vec<PathBuf>,
    /// Which of Office's font folders to read too.
    pub office: Office,
    /// Fonts that ship with the app (craft-fonts, the app's own), added as [`Source::Bundled`]
    /// with one rebuild of the lookups ([`Catalog::add_bytes_many`]). Shared, not copied:
    /// `Arc::from(bytes)` of a `Vec<u8>` or a `&'static [u8]` copies once.
    pub bundled: Vec<Arc<[u8]>>,
    /// How often [`on_activated`] runs a full [`Catalog::rescan`] at most ([`Duration::MAX`]:
    /// never).
    pub full_rescan_every: Duration,
    /// Called after [`on_activated`] found a family added, removed or changed, to wake an idle
    /// window so it checks [`changes_since`]. Keep it short: activations do nothing while it runs.
    pub on_change: Option<OnChange>,
}

impl Default for AppFonts {
    fn default() -> Self {
        Self { dirs: Vec::new(), office: Office::None, bundled: Vec::new(), full_rescan_every: FULL_RESCAN_EVERY, on_change: None }
    }
}

/// What [`start`] and [`configure`] did.
pub struct Started {
    /// The global catalog.
    pub catalog: &'static Arc<Catalog>,
    /// The bundled faces that couldn't be added, by their place in [`AppFonts::bundled`].
    pub bundled_errors: Vec<(usize, FontFileError)>,
}

/// Start the global catalog for a desktop or web app: [`configure`], then the first scan on a
/// background thread. It doesn't wait for the scan. Call it once, before the first window and
/// before anything else uses [`Catalog::global`] or [`on_activated`].
pub fn start(fonts: AppFonts) -> Started {
    let started = configure(fonts);
    started.catalog.scan_in_background();
    started
}

/// [`start`] without the background scan, for a CLI: the first lookup scans and waits for it.
pub fn configure(fonts: AppFonts) -> Started {
    let catalog = Catalog::global();
    let (full_every, on_change) = (fonts.full_rescan_every, fonts.on_change.clone());
    let bundled_errors = add_app_fonts(catalog, fonts);
    let _ = REFRESH.set(Arc::new(Refresh::new(Arc::clone(catalog), full_every, on_change)));
    Started { catalog, bundled_errors }
}

/// The folders and bundled faces of `fonts` into `catalog`; returns the bundled faces that
/// couldn't be added.
pub(crate) fn add_app_fonts(catalog: &Catalog, fonts: AppFonts) -> Vec<(usize, FontFileError)> {
    let mut dirs = fonts.dirs;
    if fonts.office != Office::None {
        dirs.extend(office_bundled_font_dirs(catalog.env()));
    }
    if fonts.office == Office::All {
        dirs.extend(office_cloud_font_dirs(catalog.env()));
    }
    catalog.set_app_dirs(dirs);
    let added = catalog.add_bytes_many(fonts.bundled, Source::Bundled, &Limits::default());
    added.into_iter().enumerate().filter_map(|(i, r)| Some((i, r.err()?))).collect()
}

/// For a window's activation: on a worker thread, [`Catalog::rescan_if_changed`], or a full
/// [`Catalog::rescan`] when the last was [`AppFonts::full_rescan_every`] ago or more. It never
/// blocks: while a worker runs, another activation does nothing. Nothing happens before the
/// first scan has finished, or on wasm. Returns whether a worker started.
pub fn on_activated() -> bool {
    refresh().activated()
}

fn refresh() -> &'static Arc<Refresh> {
    REFRESH.get_or_init(|| Arc::new(Refresh::new(Arc::clone(Catalog::global()), FULL_RESCAN_EVERY, None)))
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

/// The activation rescans of one catalog.
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

/// Clears [`RefreshState::running`] when the worker ends, panicking included.
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

    fn activated(self: &Arc<Self>) -> bool {
        if cfg!(target_arch = "wasm32") {
            return false;
        }
        self.activated_at(Instant::now())
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
        let spawned = std::thread::Builder::new().name("font-catalog-refresh".into()).spawn(move || me.run(full));
        if spawned.is_err() {
            let mut state = lock(&self.state);
            state.running = false;
            state.last_full = Some(last_full);
            return false;
        }
        true
    }

    fn run(&self, full: bool) {
        let _running = Running(&self.state);
        let before = self.catalog.generation();
        if full {
            self.catalog.rescan();
        } else {
            self.catalog.rescan_if_changed();
        }
        self.changed_since(before);
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

    /// Whether a worker is running.
    #[cfg(test)]
    pub(crate) fn is_running(&self) -> bool {
        lock(&self.state).running
    }
}
