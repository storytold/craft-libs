//! One call for the OS's colour scheme, contrast and reduced-motion preferences on every target:
//! the settings portal's [`Watcher`] where one exists, the browser's media queries on the web.

use super::{Contrast, Theme, Watcher};

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
/// one; on the web it asks the browser on each read, since media queries have no change
/// notification, so read it when building a frame. A target with neither reports no answers.
pub struct SystemReader {
    watcher: Option<Watcher>,
}

impl SystemReader {
    /// Starts watching. `wake` runs on the watcher's worker when a preference changes and must
    /// not block.
    pub fn start(wake: impl Fn() + Send + Sync + 'static) -> Self {
        Self { watcher: Watcher::start(wake) }
    }

    /// Whether a desktop watcher is running.
    pub fn watching(&self) -> bool {
        self.watcher.is_some()
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
        #[cfg(target_arch = "wasm32")]
        {
            Reading { theme: None, contrast: super::browser_contrast(), reduced_motion: super::browser_reduced_motion() }
        }
        #[cfg(not(target_arch = "wasm32"))]
        {
            let watcher = self.watcher.as_ref();
            Reading {
                theme: watcher.and_then(Watcher::current),
                contrast: watcher.and_then(Watcher::contrast),
                reduced_motion: watcher.and_then(Watcher::reduced_motion),
            }
        }
    }
}
