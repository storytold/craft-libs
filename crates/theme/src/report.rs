//! The contrast sweep over every appearance of a theme, shared by the `craft-theme` CLI and the
//! Theme workspace's Problems panel.

use crate::{AppId, Appearance, Contrast, ContrastCheck, Mode, Size, TextSize, ThemeDef, Variants, resolve};

/// The resolve warnings of one appearance of a theme, for the theme itself or one of its apps.
#[derive(Clone, Debug)]
pub struct AppearanceReport {
    /// The app the sweep resolved for; `None` for the theme itself.
    pub app: Option<AppId>,
    /// Text size is the default and reduced motion is off.
    pub appearance: Appearance,
    /// The contrast checks that failed in this appearance.
    pub warnings: Vec<ContrastCheck>,
}

impl AppearanceReport {
    /// `"<app>: light/high/compact"`, without the app prefix for the theme itself.
    pub fn label(&self) -> String {
        let a = &self.appearance;
        format!(
            "{}{}/{}/{}",
            self.app.as_ref().map_or(String::new(), |app| format!("{app}: ")),
            if a.mode == Mode::Light { "light" } else { "dark" },
            if a.contrast == Contrast::High { "high" } else { "standard" },
            if a.size == Size::Compact { "compact" } else { "comfortable" },
        )
    }
}

/// [`AppearanceReport`]s in sweep order: the theme itself, then each app; within each, modes the
/// theme has, then contrast (standard, high), then size (compact, comfortable).
#[derive(Clone, Debug, Default)]
pub struct ContrastReport {
    /// One report per appearance, in sweep order.
    pub appearances: Vec<AppearanceReport>,
}

impl ContrastReport {
    /// Iterates over the appearances in sweep order.
    pub fn iter(&self) -> std::slice::Iter<'_, AppearanceReport> {
        self.appearances.iter()
    }

    /// The report of one appearance; `None` when the theme lacks that mode or app.
    pub fn for_appearance(&self, app: Option<&AppId>, mode: Mode, contrast: Contrast, size: Size) -> Option<&AppearanceReport> {
        self.appearances
            .iter()
            .find(|r| r.app.as_ref() == app && r.appearance.mode == mode && r.appearance.contrast == contrast && r.appearance.size == size)
    }

    /// The number of warnings over all appearances.
    pub fn warning_count(&self) -> usize {
        self.appearances.iter().map(|r| r.warnings.len()).sum()
    }
}

impl<'a> IntoIterator for &'a ContrastReport {
    type Item = &'a AppearanceReport;
    type IntoIter = std::slice::Iter<'a, AppearanceReport>;

    fn into_iter(self) -> Self::IntoIter {
        self.iter()
    }
}

/// Resolves `theme` for every app × mode it has × contrast × size and collects the warnings.
///
/// This calls [`resolve`] `(1 + apps) × modes × 4` times; the caller schedules that cost.
pub fn contrast_report(theme: &ThemeDef) -> ContrastReport {
    let modes: &[Mode] = match theme.variants {
        Variants::Both => &[Mode::Light, Mode::Dark],
        Variants::LightOnly => &[Mode::Light],
        Variants::DarkOnly => &[Mode::Dark],
    };
    let apps = std::iter::once(None).chain(theme.apps.keys().map(Some));
    let mut appearances = Vec::new();
    for app in apps {
        for &mode in modes {
            for contrast in [Contrast::Standard, Contrast::High] {
                for size in [Size::Compact, Size::Comfortable] {
                    let appearance = Appearance { mode, contrast, size, text_size: TextSize::Small, reduced_motion: false };
                    let warnings = resolve(theme, appearance, app).warnings;
                    appearances.push(AppearanceReport { app: app.cloned(), appearance, warnings });
                }
            }
        }
    }
    ContrastReport { appearances }
}
