//! RootEnv, the app's root values each frame, and their resolved form.

use super::app_values::AppValues;
use super::values::Scale;
use crate::runtime::egui::memory::Service;
use craft_behavior::MotionMode;
use craft_i18n::CalendarDate;
use craft_i18n::Direction;
use craft_theme::{AppSizes, Appearance, Contrast, ContrastChoice, Mode, ModeChoice, MotionChoice, Size, SystemAppearance, TextSize, ThemeId};
use egui::Context;
use std::sync::Arc;

/// The app's root values each frame.
#[derive(Clone)]
#[non_exhaustive]
pub struct RootEnv {
    /// The app's `sizes.json`.
    pub sizes: Arc<AppSizes>,
    /// The text-size step.
    pub text_size: TextSize,
    /// The layout direction.
    pub direction: Direction,
    /// The motion choice; `System` follows the OS where it reports a preference, else full motion.
    pub motion: MotionChoice,
    /// Light, dark, or the OS's mode.
    pub mode: ModeChoice,
    /// Standard, high, or the OS's contrast.
    pub contrast: ContrastChoice,
    /// The density preset.
    pub density: Size,
    /// The interface scale over the native pixels per point.
    pub scale: Scale,
    /// Whether Tab reaches every control.
    pub tab_all_controls: bool,
    /// Whether a bare Tab toggles the window's chrome in place of moving focus where the key stage's gate allows it.
    pub tab_toggles_chrome: bool,
    /// The theme the tokens resolve from; an id no built-in theme has resolves to the suite default.
    pub theme: ThemeId,
    /// Root app values.
    pub values: AppValues,
    /// Today's date in the host's time zone; the runtime reads no wall clock.
    pub today: Option<CalendarDate>,
}

impl Default for RootEnv {
    fn default() -> Self {
        RootEnv {
            sizes: Arc::new(AppSizes::default()),
            text_size: TextSize::default(),
            direction: Direction::Ltr,
            motion: MotionChoice::Full,
            mode: ModeChoice::Light,
            contrast: ContrastChoice::Standard,
            density: Size::default(),
            scale: Scale::default(),
            tab_all_controls: true,
            tab_toggles_chrome: false,
            theme: ThemeId::suite_default(),
            values: AppValues::default(),
            today: None,
        }
    }
}

impl RootEnv {
    /// The default root of `ctx`, built once so its sizes keep one identity for the token cache.
    pub(crate) fn fallback(ctx: &Context) -> RootEnv {
        #[derive(Default)]
        struct Fallback(Option<RootEnv>);
        Service::<Fallback>::run(ctx, |fallback| fallback.0.get_or_insert_with(RootEnv::default).clone())
    }

    /// A root with every value at its default.
    pub fn new() -> Self {
        Self::default()
    }

    /// Sets the interface scale.
    pub fn scale(mut self, scale: Scale) -> Self {
        self.scale = scale;
        self
    }

    /// Sets the layout direction.
    pub fn direction(mut self, direction: Direction) -> Self {
        self.direction = direction;
        self
    }

    /// Sets the motion choice from a motion mode.
    pub fn motion(mut self, motion: MotionMode) -> Self {
        self.motion = match motion {
            MotionMode::Off => MotionChoice::Off,
            MotionMode::Reduced => MotionChoice::Reduced,
            _ => MotionChoice::Full,
        };
        self
    }

    /// Sets the mode choice.
    pub fn mode(mut self, mode: impl Into<ModeChoice>) -> Self {
        self.mode = mode.into();
        self
    }

    /// Sets the theme.
    pub fn theme(mut self, theme: ThemeId) -> Self {
        self.theme = theme;
        self
    }

    /// What the choices resolve to on `ctx`'s pass: the one place `System` is replaced by the OS's answer.
    ///
    /// egui reports only the OS theme (`Context::system_theme`); an OS that reports none gives `Mode::Light`,
    /// and contrast and reduced motion, which neither egui nor `craft-window` expose, resolve to standard
    /// contrast and full motion.
    pub fn resolved(&self, ctx: &Context) -> ResolvedRoot {
        let system = SystemAppearance {
            mode: ctx.system_theme().map(|theme| if theme == egui::Theme::Dark { Mode::Dark } else { Mode::Light }),
            contrast: None,
            reduced_motion: None,
        };
        self.resolved_for(system)
    }

    /// What the choices resolve to when the OS reports `system`.
    pub fn resolved_for(&self, system: SystemAppearance) -> ResolvedRoot {
        let mode = match self.mode {
            ModeChoice::Light => Mode::Light,
            ModeChoice::Dark => Mode::Dark,
            ModeChoice::System => system.mode.unwrap_or(Mode::Light),
        };
        let contrast = match self.contrast {
            ContrastChoice::Standard => Contrast::Standard,
            ContrastChoice::High => Contrast::High,
            ContrastChoice::System => system.contrast.unwrap_or_default(),
        };
        let motion = match self.motion {
            MotionChoice::Off => MotionMode::Off,
            choice if choice.reduced(system.reduced_motion) => MotionMode::Reduced,
            _ => MotionMode::Full,
        };
        ResolvedRoot { mode, contrast, motion }
    }

    /// The appearance the root's tokens resolve under on `ctx`: the mode, contrast, density and text size.
    pub fn appearance(&self, ctx: &Context) -> Appearance {
        let resolved = self.resolved(ctx);
        Appearance {
            mode: resolved.mode,
            contrast: resolved.contrast,
            size: self.density,
            text_size: self.text_size,
            reduced_motion: resolved.motion != MotionMode::Full,
        }
    }

    /// Sets the contrast choice.
    pub fn contrast(mut self, contrast: impl Into<ContrastChoice>) -> Self {
        self.contrast = contrast.into();
        self
    }

    /// Sets the density preset.
    pub fn density(mut self, density: Size) -> Self {
        self.density = density;
        self
    }

    /// Sets today's date, as the host's clock and time zone give it.
    pub fn today(mut self, date: CalendarDate) -> Self {
        self.today = Some(date);
        self
    }
}

/// The mode, contrast and motion of a pass, with `System` resolved.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct ResolvedRoot {
    /// Light or dark.
    pub mode: Mode,
    /// Standard or high contrast.
    pub contrast: Contrast,
    /// The motion mode, before egui's animation time lowers it.
    pub motion: MotionMode,
}
