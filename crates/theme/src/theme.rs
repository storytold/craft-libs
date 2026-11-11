//! What a theme supplies (seeds, pins, optional shape) and what the user chooses (appearance).

use std::collections::BTreeMap;

use super::color::Rgba;
use super::components::ComponentField;
use super::ramp::Mode;
use super::tokens::{ColorTokens, Edge, RadiusSet, RowStyle, Status, TokenRef};
use crate::{AppId, ThemeId};

/// A theme: seeds to derive from, values that override the derivation, and an optional shape.
#[derive(Clone, Debug, PartialEq)]
pub struct ThemeDef {
    /// Stable identity.
    pub id: ThemeId,
    /// Display name.
    pub name: String,
    /// The author's revision of the theme; 1 for a new theme.
    pub version: u32,
    /// The colours every token derives from.
    pub seeds: Seeds,
    /// Per-mode seeds that replace the matching `seeds` fields in that mode.
    pub mode_seeds: ModeSeeds,
    /// Radius and edge of themes with their own shape (Classic, Breeze); `None` takes the Size
    /// preset's.
    pub shape: Option<ShapeSeed>,
    /// Which modes the theme has.
    pub variants: Variants,
    /// Fixed colours per mode, replacing derived ones.
    pub pins: ModePins,
    /// Fixed colours used only with [`Contrast::High`], applied over `pins`.
    pub high_contrast: ModePins,
    /// Component token overrides: the slider thumb lengths.
    pub components: BTreeMap<ComponentField, f32>,
    /// Whether the theme is meant to have low contrast.
    pub contrast_intent: ContrastIntent,
    /// Per-app overrides, applied over the theme's own values when resolving for that app.
    pub apps: BTreeMap<AppId, AppOverrides>,
}

/// What a theme changes for one app. The pins apply after the theme's pins and before its
/// high-contrast pins, the high-contrast pins after the theme's high-contrast pins, and the
/// components after the theme's components.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct AppOverrides {
    /// Fixed colours per mode.
    pub pins: ModePins,
    /// Fixed colours per mode used only with [`Contrast::High`].
    pub high_contrast: ModePins,
    /// Component token overrides.
    pub components: BTreeMap<ComponentField, f32>,
}

impl ThemeDef {
    /// The seeds `mode` derives from: the theme's `seeds` with that mode's overrides applied.
    pub fn seeds_for(&self, mode: Mode) -> Seeds {
        let o = self.mode_seeds.get(mode);
        Seeds {
            neutral: o.neutral.unwrap_or(self.seeds.neutral),
            accent: o.accent.unwrap_or(self.seeds.accent),
            status: Status {
                good: o.status.good.unwrap_or(self.seeds.status.good),
                warn: o.status.warn.unwrap_or(self.seeds.status.warn),
                bad: o.status.bad.unwrap_or(self.seeds.status.bad),
                info: o.status.info.unwrap_or(self.seeds.status.info),
            },
        }
    }
}

/// Seed colours. Each one becomes a 12-step ramp per mode.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Seeds {
    /// The grey family and its tint.
    pub neutral: Rgba,
    /// The accent hue.
    pub accent: Rgba,
    /// One hue per status.
    pub status: Status<Rgba>,
}

/// Seeds one mode replaces; a field left `None` keeps the theme's value.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct SeedOverrides {
    /// Replaces the neutral seed.
    pub neutral: Option<Rgba>,
    /// Replaces the accent seed.
    pub accent: Option<Rgba>,
    /// Replaces individual status seeds.
    pub status: Status<Option<Rgba>>,
}

impl SeedOverrides {
    /// Whether no field is overridden.
    pub fn is_empty(&self) -> bool {
        *self == SeedOverrides::default()
    }
}

/// Seed overrides for each mode.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct ModeSeeds {
    /// Used in light mode.
    pub light: SeedOverrides,
    /// Used in dark mode.
    pub dark: SeedOverrides,
}

impl ModeSeeds {
    /// The overrides of `mode`.
    pub fn get(&self, mode: Mode) -> &SeedOverrides {
        match mode {
            Mode::Light => &self.light,
            Mode::Dark => &self.dark,
        }
    }
}

/// Fixed colours for some semantic tokens and component colour fields, and references: tokens
/// that take another token's final value (`"{accent.solid}"` in a theme file).
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
pub struct Pins {
    /// Tokens pinned to a colour.
    pub colors: BTreeMap<TokenRef, Rgba>,
    /// Tokens pinned to the final value of another colour token or colour field, after
    /// derivation and the contrast pass, in the same theme, mode, contrast and size. Resolution
    /// replaces each with a colour pin. A reference to a token pinned in the theme reads the
    /// pinned value. A token in `colors` and `refs` of the same layer takes the reference.
    pub refs: BTreeMap<TokenRef, TokenRef>,
}

impl Pins {
    /// The pinned colour of `token`, if any. A reference is not a colour until resolution.
    pub fn get(&self, token: impl Into<TokenRef>) -> Option<Rgba> {
        self.colors.get(&token.into()).copied()
    }

    /// Whether nothing is pinned or referenced.
    pub fn is_empty(&self) -> bool {
        self.colors.is_empty() && self.refs.is_empty()
    }
}

impl<K: Into<TokenRef>> FromIterator<(K, Rgba)> for Pins {
    fn from_iter<I: IntoIterator<Item = (K, Rgba)>>(iter: I) -> Pins {
        Pins { colors: iter.into_iter().map(|(k, v)| (k.into(), v)).collect(), refs: BTreeMap::new() }
    }
}

/// Pins for each mode.
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
pub struct ModePins {
    /// Used in light mode.
    pub light: Pins,
    /// Used in dark mode.
    pub dark: Pins,
}

impl ModePins {
    /// The pins of `mode`.
    pub fn get(&self, mode: Mode) -> &Pins {
        match mode {
            Mode::Light => &self.light,
            Mode::Dark => &self.dark,
        }
    }
}

/// One set of pins in the order resolution applies them; a later layer wins.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
#[non_exhaustive]
pub enum PinLayer {
    /// [`ThemeDef::pins`].
    Theme,
    /// The app's [`AppOverrides::pins`].
    App,
    /// [`ThemeDef::high_contrast`], with [`Contrast::High`] only.
    ThemeHighContrast,
    /// The app's [`AppOverrides::high_contrast`], with [`Contrast::High`] only.
    AppHighContrast,
}

/// A theme's own shape. Fields left `None` come from the Size preset.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ShapeSeed {
    /// Replaces every radius of the preset.
    pub radius: Option<RadiusSet>,
    /// Added to the control, surface and window radii in force (the preset's, or `radius`), so a
    /// theme keeps its own shape at every Size. A result below 0 is 0; a "full" radius stays full.
    pub radius_offset: Option<RadiusOffset>,
    /// Replaces the button radius in force (the preset's, or `radius`'s); 999 gives pills.
    pub button_radius: Option<f32>,
    /// Replaces the preset's edge style.
    pub edge: Option<EdgeSeed>,
    /// Replaces the preset's row style.
    pub rows: Option<RowStyle>,
}

/// Per-tier additions to the corner radii, in logical pixels.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct RadiusOffset {
    /// Added to `control`.
    pub control: f32,
    /// Added to `surface`.
    pub surface: f32,
    /// Added to `window`.
    pub window: f32,
}

impl RadiusOffset {
    /// `radius` with the offsets added; `panel`, `button` and `pill` are not offset.
    pub fn apply(self, radius: RadiusSet) -> RadiusSet {
        let add = |r: f32, d: f32| if r >= crate::scale::Radius::Full.px() { r } else { (r + d).max(0.0) };
        RadiusSet {
            control: add(radius.control, self.control),
            surface: add(radius.surface, self.surface),
            window: add(radius.window, self.window),
            ..radius
        }
    }
}

/// A theme's edge style. A bevel's colours are the `edge.light` and `edge.dark` colour tokens:
/// `surface.panel` lit and shaded unless pinned.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum EdgeSeed {
    /// No edge effect.
    Flat,
    /// A two-tone bevel.
    Bevel {
        /// Line width in logical pixels.
        width: f32,
    },
}

impl EdgeSeed {
    /// The edge this seed gives, a bevel taking `edge.light` and `edge.dark` from `color`.
    pub fn resolve(self, color: &ColorTokens) -> Edge {
        match self {
            EdgeSeed::Flat => Edge::Flat,
            EdgeSeed::Bevel { width } => Edge::Bevel { light: color.edge.light, dark: color.edge.dark, width },
        }
    }
}

/// Which modes a theme has.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Variants {
    /// Light and dark.
    Both,
    /// Light only; dark mode shows the light variant.
    LightOnly,
    /// Dark only; light mode shows the dark variant.
    DarkOnly,
}

impl Variants {
    /// The mode that is shown when `wanted` is asked for.
    pub fn mode_for(self, wanted: Mode) -> Mode {
        match self {
            Variants::Both => wanted,
            Variants::LightOnly => Mode::Light,
            Variants::DarkOnly => Mode::Dark,
        }
    }
}

/// The contrast a theme is designed for.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum ContrastIntent {
    /// Meets the standard minimums except where pins say otherwise.
    #[default]
    Standard,
    /// Deliberately soft (Soft); the picker labels it.
    Low,
}

/// The user's contrast setting.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Contrast {
    /// WCAG AA minimums; pinned colours are kept and reported.
    #[default]
    Standard,
    /// Higher minimums, enforced on every colour including pinned ones.
    High,
}

/// The density preset: control heights, padding, gaps, type and default shape.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Size {
    /// Dense, sharper corners (Pro).
    #[default]
    Compact,
    /// Roomier, rounder corners (Studio).
    Comfortable,
}

/// The text-size setting (Photoshop's Interface › UI Font Size); scales type roles only, by the
/// factor [`TextSizeSteps`] gives each step.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TextSize {
    /// Photoshop's Tiny (10 pt on a 12 pt base).
    Tiny,
    /// Photoshop's Small (12 pt), the default: factor 1.
    #[default]
    #[serde(alias = "default")]
    Small,
    /// Photoshop's Medium (14 pt).
    Medium,
    /// Photoshop's Large (16 pt).
    #[serde(alias = "larger")]
    Large,
}

impl TextSize {
    /// Every step, smallest first.
    pub const ALL: [TextSize; 4] = [TextSize::Tiny, TextSize::Small, TextSize::Medium, TextSize::Large];
}

/// The multipliers of the four text-size steps on the type roles, a runtime value an app may
/// replace through [`AppSizes::text_steps`](crate::AppSizes::text_steps). The default is
/// [`TextSizeSteps::PHOTOSHOP`]. Only the roles scale: faces, icon sizes and control heights
/// don't, so Size and Text size stay independent. Scaled sizes round as every type size does
/// (`values::TYPE_ROUNDING`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TextSizeSteps {
    /// Multipliers in [`TextSize::ALL`] order; the second is 1.0.
    pub factors: [f32; 4],
}

impl TextSizeSteps {
    /// Photoshop's UI Font Size ladder relative to Small (12 pt): Tiny 10, Medium 14, Large 16.
    pub const PHOTOSHOP: TextSizeSteps = TextSizeSteps { factors: [10.0 / 12.0, 1.0, 14.0 / 12.0, 16.0 / 12.0] };

    /// The multiplier of one step.
    pub fn factor(self, size: TextSize) -> f32 {
        match size {
            TextSize::Tiny => self.factors[0],
            TextSize::Small => self.factors[1],
            TextSize::Medium => self.factors[2],
            TextSize::Large => self.factors[3],
        }
    }
}

impl Default for TextSizeSteps {
    fn default() -> Self {
        TextSizeSteps::PHOTOSHOP
    }
}

/// The interface-font setting: the face the interface text draws with. Mono text keeps the suite's
/// face either way, and type sizes stay the same.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InterfaceFont {
    /// The suite's interface face.
    #[default]
    Suite,
    /// The OS's own interface face (San Francisco, Segoe UI, the Linux desktop's), where the app
    /// can read it; elsewhere the suite's face.
    System,
}

impl InterfaceFont {
    /// Every choice, default first.
    pub const ALL: [InterfaceFont; 2] = [InterfaceFont::Suite, InterfaceFont::System];
}

/// The user's appearance choices, after "System" has been replaced by what the OS reports.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Appearance {
    /// Light or dark.
    pub mode: Mode,
    /// Standard or high contrast.
    pub contrast: Contrast,
    /// Density preset.
    pub size: Size,
    /// Type scaling.
    pub text_size: TextSize,
    /// Whether animations are turned off.
    pub reduced_motion: bool,
}
