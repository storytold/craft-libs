//! One call for the OS's colour scheme, contrast and reduced-motion preferences on every target:
//! the settings portal's [`Watcher`] where one exists, the browser's media queries on the web,
//! and the colour scheme the windowing integration reports.

use super::{Contrast, Theme, Watcher};
use std::sync::atomic::{AtomicU8, AtomicU64, Ordering};

/// What the OS reports now. `None` is no answer; the host decides the fallback.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Reading {
    /// The colour scheme.
    pub theme: Option<Theme>,
    /// The contrast preference.
    pub contrast: Option<Contrast>,
    /// True when the user asks for reduced motion.
    pub reduced_motion: Option<bool>,
}

impl From<Reading> for craft_theme::SystemAppearance {
    fn from(reading: Reading) -> Self {
        craft_theme::SystemAppearance {
            mode: reading.theme.map(|t| match t {
                Theme::Dark => craft_theme::Mode::Dark,
                Theme::Light => craft_theme::Mode::Light,
            }),
            contrast: reading.contrast.map(|c| match c {
                Contrast::Standard => craft_theme::Contrast::Standard,
                Contrast::High => craft_theme::Contrast::High,
            }),
            reduced_motion: reading.reduced_motion,
        }
    }
}

/// Reads the OS's appearance preferences. Owns the desktop [`Watcher`] where the target has
/// one; on the web it asks the browser's contrast and motion queries on each read. The host
/// reports the colour scheme its windowing integration gives ([`SystemReader::report_theme`]);
/// where the watcher knows the scheme, the watcher's answer wins.
pub struct SystemReader {
    watcher: Option<Watcher>,
    reported: Reported,
}

/// What the host reported. Codes: 0 no answer; theme 1 dark, 2 light; contrast 1 standard,
/// 2 high; motion 1 animations allowed, 2 reduced.
#[derive(Default)]
struct Reported {
    theme: AtomicU8,
    contrast: AtomicU8,
    motion: AtomicU8,
    /// Raised on each change of a reported value.
    generation: AtomicU64,
}

impl Reported {
    fn set(&self, slot: &AtomicU8, code: u8) {
        if slot.swap(code, Ordering::AcqRel) != code {
            self.generation.fetch_add(1, Ordering::AcqRel);
        }
    }
}

impl SystemReader {
    /// Starts watching. `wake` runs on the watcher's worker when a preference changes and must
    /// not block.
    pub fn start(wake: impl Fn() + Send + Sync + 'static) -> Self {
        Self { watcher: Watcher::start(wake), reported: Reported::default() }
    }

    /// A reader without a desktop watcher: it answers what the host reports (and the browser's
    /// queries on the web).
    pub fn unwatched() -> Self {
        Self { watcher: None, reported: Reported::default() }
    }

    /// Whether a desktop watcher is running.
    pub fn watching(&self) -> bool {
        self.watcher.is_some()
    }

    /// Takes the colour scheme the windowing integration reports (egui's `Event::SystemTheme`).
    pub fn report_theme(&self, theme: Theme) {
        let reported = &self.reported;
        reported.set(&reported.theme, if theme == Theme::Dark { 1 } else { 2 });
    }

    /// Takes a contrast preference in place of an OS source.
    #[cfg(feature = "testing")]
    pub fn report_contrast(&self, contrast: Contrast) {
        let reported = &self.reported;
        reported.set(&reported.contrast, if contrast == Contrast::High { 2 } else { 1 });
    }

    /// Takes a reduced-motion preference in place of an OS source.
    #[cfg(feature = "testing")]
    pub fn report_reduced_motion(&self, reduced: bool) {
        let reported = &self.reported;
        reported.set(&reported.motion, if reduced { 2 } else { 1 });
    }

    /// Rises each time a preference [`SystemReader::read`] answers from changes, as its source
    /// reports it: the watcher's theme, contrast or reduced motion, or a reported value. Read
    /// again when it moved. On the web the browser's queries raise it in no case. Read it before
    /// [`SystemReader::read`]: the watcher stores a value before it raises the generation.
    pub fn generation(&self) -> u64 {
        self.watcher.as_ref().map_or(0, Watcher::generation) + self.reported.generation.load(Ordering::Acquire)
    }

    /// GNOME's `button-layout` as the watcher last heard it; see [`Watcher::button_layout`].
    /// `None` on the web and where no watcher runs.
    pub fn button_layout(&self) -> Option<String> {
        self.watcher.as_ref().and_then(Watcher::button_layout)
    }

    /// GNOME's `icon-theme` name as the watcher last heard it; see [`Watcher::icon_theme`].
    /// `None` on the web and where no watcher runs.
    pub fn icon_theme(&self) -> Option<String> {
        self.watcher.as_ref().and_then(Watcher::icon_theme)
    }

    /// The preferences now, without blocking.
    pub fn read(&self) -> Reading {
        let reported = &self.reported;
        let theme = match reported.theme.load(Ordering::Acquire) {
            1 => Some(Theme::Dark),
            2 => Some(Theme::Light),
            _ => None,
        };
        let contrast = match reported.contrast.load(Ordering::Acquire) {
            1 => Some(Contrast::Standard),
            2 => Some(Contrast::High),
            _ => None,
        };
        let reduced_motion = match reported.motion.load(Ordering::Acquire) {
            1 => Some(false),
            2 => Some(true),
            _ => None,
        };
        #[cfg(target_arch = "wasm32")]
        {
            Reading { theme, contrast: super::browser_contrast().or(contrast), reduced_motion: super::browser_reduced_motion().or(reduced_motion) }
        }
        #[cfg(not(target_arch = "wasm32"))]
        {
            let watcher = self.watcher.as_ref();
            Reading {
                theme: watcher.and_then(Watcher::current).or(theme),
                contrast: watcher.and_then(Watcher::contrast).or(contrast),
                reduced_motion: watcher.and_then(Watcher::reduced_motion).or(reduced_motion),
            }
        }
    }
}
