//! RootEnv, the app's root values each frame, and their resolved form.

use super::app_values::AppValues;
use super::values::{AltCodePages, Scale, WheelSpeed};
use crate::runtime::egui::memory::Service;
use crate::runtime::focus::TabReach;
use craft_behavior::MotionMode;
use craft_i18n::CalendarDate;
use craft_i18n::Direction;
use craft_theme::{
    AppSizes, Appearance, ComponentSet, Contrast, ContrastChoice, Mode, ModeChoice, MotionChoice, Size, SystemAppearance, TextSize, ThemeDef, ThemeId,
};
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
    /// Whether Escape in a focused text field also takes the focus from it, with or without typing to revert, so the
    /// next keys go to the window. A clearing field (a search box) still clears its text first.
    pub escape_leaves_fields: bool,
    /// Whether a held key's repeats run the command its chord binds again (`FirePhase::Repeat`), as a held Cmd+= keeps
    /// zooming in. A key with a hold binding, and a key held down while a modal layer opened or closed, repeat nothing.
    pub shortcuts_repeat: bool,
    /// Which Tab stops a dialog's Tab order visits.
    pub dialog_tab: TabReach,
    /// Whether the canvas surround shows the theme's pattern.
    pub canvas_pattern: bool,
    /// Whether painted cursor images scale with the interface zoom.
    pub cursor_follows_zoom: bool,
    /// Whether tooltips show.
    pub tooltips: bool,
    /// The theme the tokens resolve from; an id no built-in theme has resolves to the suite default.
    /// Private with `theme_def`, so the two change together.
    pub(super) theme: ThemeId,
    /// The definition of `theme` when it is a library theme: no built-in has its id, so the tokens resolve from this.
    /// Set only with `theme`, by `library_theme`; `theme(id)` clears it.
    pub(super) theme_def: Option<Arc<ThemeDef>>,
    /// The user's accent over the theme (`ThemeDef::with_accent`); `None` keeps the theme's own.
    pub(super) accent: Option<craft_theme::Rgba>,
    /// Root app values.
    pub values: AppValues,
    /// Today's date in the host's time zone; the runtime reads no wall clock.
    pub today: Option<CalendarDate>,
    /// This frame's wheel speed.
    pub wheel: WheelSpeed,
    /// The code pages that decode Alt+numpad codes.
    pub alt_codes: AltCodePages,
    /// On Windows and Linux, a Ctrl+wheel in fractions of a notch (a point-unit one on Linux) is a touchpad pinch
    /// and arrives as `egui::Event::Zoom`: Windows hands a precision touchpad's pinch to apps only that way, and some
    /// Wayland stacks do too. On by default. Off, it stays a wheel event with Ctrl held, for an app whose Ctrl+wheel
    /// does something else or zooms at its own rate (Zoom with Trackpad Pinch off). Whole notches stay wheel events
    /// either way; macOS sends a pinch as its own event.
    pub ctrl_wheel_pinch: bool,
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
            escape_leaves_fields: false,
            shortcuts_repeat: false,
            dialog_tab: TabReach::Root,
            canvas_pattern: true,
            cursor_follows_zoom: true,
            tooltips: true,
            theme: ThemeId::suite_default(),
            theme_def: None,
            accent: None,
            values: AppValues::default(),
            today: None,
            wheel: WheelSpeed::default(),
            alt_codes: AltCodePages::default(),
            ctrl_wheel_pinch: true,
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
        self.set_theme(theme);
        self
    }

    /// Sets the theme in place.
    pub fn set_theme(&mut self, theme: ThemeId) {
        self.theme = theme;
        self.theme_def = None;
    }

    /// The theme the tokens resolve from.
    pub fn theme_id(&self) -> &ThemeId {
        &self.theme
    }

    /// Sets a library theme: `def` is the theme and its id the root's theme.
    pub fn library_theme(mut self, def: Arc<ThemeDef>) -> Self {
        self.theme = def.id.clone();
        self.theme_def = Some(def);
        self
    }

    /// Overrides `seeds.accent` of whichever theme resolves, built-in or library, in both modes; the accent ramp derives
    /// from the seed, and the theme's fixed accent colours give way to it ([`ThemeDef::with_accent`]). `None` keeps the
    /// theme's own.
    pub fn accent(mut self, seed: Option<craft_theme::Rgba>) -> Self {
        self.accent = seed;
        self
    }

    /// What the choices resolve to on `ctx`'s pass: the one place `System` is replaced by the OS's answer, as
    /// [`system_appearance`](super::system_appearance) reads it. A value the OS reports none for resolves to light,
    /// standard contrast and full motion.
    pub fn resolved(&self, ctx: &Context) -> ResolvedRoot {
        self.resolved_for(super::system_appearance(ctx).appearance)
    }

    /// The mode the root's theme actually shows on `ctx`: the resolved mode clamped to the theme's supported
    /// variants (for example, `Mode::Light` for a light-only theme even when dark mode is requested).
    pub(crate) fn effective_mode(&self, ctx: &Context) -> Mode {
        let wanted = self.resolved(ctx).mode;
        if let Some(def) = &self.theme_def {
            return def.variants.mode_for(wanted);
        }
        let themes = craft_theme::builtins();
        let theme = [self.theme.as_str(), craft_theme::SUITE_DEFAULT]
            .into_iter()
            .find_map(|id| themes.iter().find(|theme| theme.id.as_str() == id))
            .unwrap_or(&themes[0]);
        theme.variants.mode_for(wanted)
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

    /// Sets whether painted cursor images scale with the interface zoom (the Scale Cursor Proportional to UI
    /// preference). On by default.
    pub fn cursor_follows_zoom(mut self, on: bool) -> Self {
        self.cursor_follows_zoom = on;
        self
    }

    /// Sets whether tooltips show (the Show Tool Tips preference). On by default.
    pub fn tooltips(mut self, on: bool) -> Self {
        self.tooltips = on;
        self
    }

    /// Sets the density preset.
    pub fn density(mut self, density: Size) -> Self {
        self.density = density;
        self
    }

    /// Sets the component set (`Selection::set`): a root app value, so a container can set another for its subtree.
    pub fn set(mut self, set: ComponentSet) -> Self {
        self.values.set(set);
        self
    }

    /// The component set the root's values carry.
    pub fn component_set(&self) -> ComponentSet {
        self.values.get::<ComponentSet>()
    }

    /// Sets today's date, as the host's clock and time zone give it.
    pub fn today(mut self, date: CalendarDate) -> Self {
        self.today = Some(date);
        self
    }

    /// Sets this frame's wheel speed.
    pub fn wheel_speed(mut self, speed: WheelSpeed) -> Self {
        self.wheel = speed;
        self
    }

    /// Sets the code pages that decode Alt+numpad codes.
    pub fn alt_code_pages(mut self, pages: AltCodePages) -> Self {
        self.alt_codes = pages;
        self
    }

    /// Sets whether a fractional Ctrl+wheel arrives as a pinch (`ctrl_wheel_pinch`).
    pub fn ctrl_wheel_pinch(mut self, pinch: bool) -> Self {
        self.ctrl_wheel_pinch = pinch;
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
