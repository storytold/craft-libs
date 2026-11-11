//! What the user picked (theme per mode, mode, contrast, motion, size, text size), how the suite's
//! choice, an app's own choice and the app's defaults combine, and resolution of the result
//! against what the OS reports.

use super::app_sizes::AppSizes;
use super::builtins;
use super::ramp::Mode;
use super::resolve::{Resolved, resolve_sized};
use super::theme::{Appearance, Contrast, Size, TextSize, ThemeDef, Variants};
use crate::{AppId, ThemeId};

/// Light, dark, or whatever the OS reports.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ModeChoice {
    #[allow(missing_docs)]
    Light,
    #[allow(missing_docs)]
    Dark,
    /// The OS's mode, else the app's fallback mode.
    System,
}

impl From<Mode> for ModeChoice {
    fn from(mode: Mode) -> Self {
        match mode {
            Mode::Light => ModeChoice::Light,
            Mode::Dark => ModeChoice::Dark,
        }
    }
}

/// Standard, high, or whatever the OS reports.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ContrastChoice {
    #[allow(missing_docs)]
    Standard,
    #[allow(missing_docs)]
    High,
    /// The OS's contrast, else standard.
    #[default]
    System,
}

impl From<Contrast> for ContrastChoice {
    fn from(contrast: Contrast) -> Self {
        match contrast {
            Contrast::Standard => ContrastChoice::Standard,
            Contrast::High => ContrastChoice::High,
        }
    }
}

/// Motion as designed, reduced, or whatever the OS reports.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MotionChoice {
    /// Motion as the theme designs it.
    Full,
    /// Reduced motion: every duration is zero.
    Reduced,
    /// Nothing animates: for hosts and tests that must see end states. The reduced-motion appearance applies.
    Off,
    /// The OS's reduced-motion setting, else full motion.
    #[default]
    System,
}

impl MotionChoice {
    /// Whether motion is reduced: the chosen answer, or what the OS reports (`None`: not
    /// reduced) when the choice is [`MotionChoice::System`].
    pub fn reduced(self, system: Option<bool>) -> bool {
        match self {
            MotionChoice::Full => false,
            MotionChoice::Reduced | MotionChoice::Off => true,
            MotionChoice::System => system.unwrap_or(false),
        }
    }
}

/// A theme for each mode. Either slot may hold [`crate::APP_DEFAULT`].
#[derive(Clone, Debug, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub struct ThemePair {
    /// The theme used in light mode.
    pub light: ThemeId,
    /// The theme used in dark mode.
    pub dark: ThemeId,
}

impl ThemePair {
    /// `id` in both slots.
    pub fn same(id: ThemeId) -> Self {
        ThemePair { light: id.clone(), dark: id }
    }

    /// The slot for `mode`.
    pub fn slot(&self, mode: Mode) -> &ThemeId {
        match mode {
            Mode::Light => &self.light,
            Mode::Dark => &self.dark,
        }
    }

    /// Puts `theme` in both slots if it has both variants, else in its own mode's slot.
    pub fn pick(&mut self, theme: &ThemeDef) {
        match theme.variants {
            Variants::Both => *self = ThemePair::same(theme.id.clone()),
            Variants::LightOnly => self.light = theme.id.clone(),
            Variants::DarkOnly => self.dark = theme.id.clone(),
        }
    }
}

/// What the user picked, every field concrete (after defaults are filled in).
#[derive(Clone, Debug, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub struct Selection {
    /// The theme for each mode.
    pub theme: ThemePair,
    /// Light, dark or the OS's mode.
    pub mode: ModeChoice,
    /// Standard, high or the OS's contrast.
    pub contrast: ContrastChoice,
    /// Full, reduced or the OS's motion.
    pub motion: MotionChoice,
    /// The size set.
    pub size: Size,
    /// The text size.
    pub text_size: TextSize,
}

/// The suite's choice as `selection.json` holds it. Each field is set once a user picks it in
/// some app; until then every app shows its own default for that field.
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct SuiteChoice {
    /// The theme chosen for light mode.
    pub theme_light: Option<ThemeId>,
    /// The theme chosen for dark mode.
    pub theme_dark: Option<ThemeId>,
    /// The chosen mode.
    pub mode: Option<ModeChoice>,
    /// The chosen contrast.
    pub contrast: Option<ContrastChoice>,
    /// The chosen motion.
    pub motion: Option<MotionChoice>,
    /// The chosen size set.
    pub size: Option<Size>,
    /// The chosen text size.
    pub text_size: Option<TextSize>,
}

impl SuiteChoice {
    /// Nothing chosen yet.
    pub fn is_empty(&self) -> bool {
        *self == SuiteChoice::default()
    }

    /// What an app that follows the suite shows: the chosen fields, else the app's defaults,
    /// contrast [`ContrastChoice::System`], motion [`MotionChoice::System`] and text size [`TextSize::Small`].
    pub fn with_defaults(&self, defaults: &AppDefaults) -> Selection {
        let slot = |chosen: &Option<ThemeId>, default: &ThemeId| chosen.clone().unwrap_or_else(|| default.clone());
        Selection {
            theme: ThemePair { light: slot(&self.theme_light, &defaults.theme.light), dark: slot(&self.theme_dark, &defaults.theme.dark) },
            mode: self.mode.unwrap_or(defaults.mode),
            contrast: self.contrast.unwrap_or_default(),
            motion: self.motion.unwrap_or_default(),
            size: self.size.unwrap_or(defaults.size),
            text_size: self.text_size.unwrap_or_default(),
        }
    }

    /// Chooses `theme` for both modes if it has both variants, else for its own mode only.
    pub fn pick_theme(&mut self, theme: &ThemeDef) {
        let id = Some(theme.id.clone());
        match theme.variants {
            Variants::Both => (self.theme_light, self.theme_dark) = (id.clone(), id),
            Variants::LightOnly => self.theme_light = id,
            Variants::DarkOnly => self.theme_dark = id,
        }
    }
}

impl From<Selection> for SuiteChoice {
    fn from(s: Selection) -> Self {
        SuiteChoice {
            theme_light: Some(s.theme.light),
            theme_dark: Some(s.theme.dark),
            mode: Some(s.mode),
            contrast: Some(s.contrast),
            motion: Some(s.motion),
            size: Some(s.size),
            text_size: Some(s.text_size),
        }
    }
}

/// What an app shows for every field the suite hasn't chosen.
///
/// Each app states its own, normally [`crate::SUITE_DEFAULT`] in both slots with the mode and
/// size that suit it (the creative apps Dark and Compact; office apps may ship Light).
#[derive(Clone, Debug, PartialEq)]
pub struct AppDefaults {
    /// The theme for each mode.
    pub theme: ThemePair,
    /// The mode the app starts in when nothing is chosen.
    pub mode: ModeChoice,
    /// The size set the app starts in when nothing is chosen.
    pub size: Size,
    /// The app's own sizes: base overrides and component pins, applied to whichever size set is
    /// chosen.
    pub sizes: AppSizes,
}

/// An app's own choice, kept in the app's preferences: "Use a different theme / mode / size in
/// this app". Fields left `None` follow the suite. A stored choice without `mode` reads as `None`.
///
/// No `Default`: an app states what a fresh install and a legacy preference file each become, so
/// a `#[serde(default)]` can't silently move existing users to the suite.
#[derive(Clone, Debug, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[non_exhaustive]
pub struct AppChoice {
    /// The app's own theme for each mode.
    pub theme: Option<ThemePair>,
    /// The app's own mode.
    pub mode: Option<ModeChoice>,
    /// The app's own size set.
    pub size: Option<Size>,
}

impl AppChoice {
    /// A choice with all three fields given.
    pub const fn new(theme: Option<ThemePair>, mode: Option<ModeChoice>, size: Option<Size>) -> Self {
        Self { theme, mode, size }
    }

    /// This choice with the app's own mode.
    pub fn with_mode(mut self, mode: ModeChoice) -> Self {
        self.mode = Some(mode);
        self
    }

    /// This choice with the app's own size.
    pub fn with_size(mut self, size: Size) -> Self {
        self.size = Some(size);
        self
    }

    /// `suite` with this app's theme, mode and size where set.
    pub fn apply(&self, suite: &Selection) -> Selection {
        Selection {
            theme: self.theme.clone().unwrap_or_else(|| suite.theme.clone()),
            mode: self.mode.unwrap_or(suite.mode),
            size: self.size.unwrap_or(suite.size),
            ..suite.clone()
        }
    }
}

/// What the OS reports; `None` where it gives no answer.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct SystemAppearance {
    /// The OS's mode.
    pub mode: Option<Mode>,
    /// The OS's contrast preference.
    pub contrast: Option<Contrast>,
    /// Whether the OS asks for reduced motion.
    pub reduced_motion: Option<bool>,
}

/// What an app paints for a selection.
#[derive(Clone, Debug, PartialEq)]
pub struct Shown {
    /// The theme resolved: the selection's, the app's default or [`crate::SUITE_DEFAULT`].
    pub theme: ThemeId,
    /// The appearance it was resolved for, "System" replaced by the OS's answer or the
    /// fallbacks. `mode` is the mode asked for; [`Resolved::variant_used`] is the one shown.
    pub appearance: Appearance,
    /// The resolved tokens and what was used to resolve them.
    pub resolved: Resolved,
    /// Why the result differs from what the selection asked for, in the order the steps found
    /// them: mode, theme, variant.
    pub notices: Vec<Fallback>,
}

/// One way the result differs from the selection.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Fallback {
    /// No theme has this id (deleted, or from another machine). Reported for each id tried.
    UnknownTheme(ThemeId),
    /// The theme has only this variant and the other mode was asked for.
    OnlyVariant(Mode),
    /// The mode follows the system and the OS gave no answer: the app's fallback mode.
    SystemSilent,
}

/// Resolves `sel` for this app. Pure.
///
/// `System` mode takes `system.mode`, else `fallback_mode`; `System` contrast takes
/// `system.contrast`, else standard; `System` motion takes `system.reduced_motion`, else full motion. The
/// theme is the pair's slot for the mode; [`crate::APP_DEFAULT`] means the slot of
/// `defaults.theme`. An id `lookup` doesn't know falls back to that slot, then to the built-in
/// [`crate::SUITE_DEFAULT`]. `app` selects the theme's per-app overrides.
pub fn resolve_selection(
    sel: &Selection,
    system: SystemAppearance,
    fallback_mode: Mode,
    defaults: &AppDefaults,
    app: Option<&AppId>,
    lookup: &mut dyn FnMut(&ThemeId) -> Option<ThemeDef>,
) -> Shown {
    let mut notices = Vec::new();
    let mode = match sel.mode {
        ModeChoice::Light => Mode::Light,
        ModeChoice::Dark => Mode::Dark,
        ModeChoice::System => system.mode.unwrap_or_else(|| {
            notices.push(Fallback::SystemSilent);
            fallback_mode
        }),
    };
    let contrast = match sel.contrast {
        ContrastChoice::Standard => Contrast::Standard,
        ContrastChoice::High => Contrast::High,
        ContrastChoice::System => system.contrast.unwrap_or_default(),
    };
    let appearance =
        Appearance { mode, contrast, size: sel.size, text_size: sel.text_size, reduced_motion: sel.motion.reduced(system.reduced_motion) };

    let wanted = sel.theme.slot(mode);
    let default = defaults.theme.slot(mode);
    let mut candidates = vec![wanted];
    if default != wanted {
        candidates.push(default);
    }
    let mut found = None;
    for id in candidates.into_iter().filter(|id| !id.is_app_default()) {
        match lookup(id) {
            Some(def) => {
                found = Some(def);
                break;
            }
            None => notices.push(Fallback::UnknownTheme(id.clone())),
        }
    }
    let theme = found.unwrap_or_else(|| builtins::suite_default().clone());

    let resolved = resolve_sized(&theme, appearance, app, &defaults.sizes);
    if resolved.variant_used != mode {
        notices.push(Fallback::OnlyVariant(resolved.variant_used));
    }
    Shown { theme: theme.id, appearance, resolved, notices }
}
