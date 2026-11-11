#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]
//! The Craft design system's theme model: primitives (sRGB and OKLCH colour, WCAG contrast,
//! 12-step colour ramps, fixed scales), the semantic layer built on them (themes as seeds and
//! pins, derivation with a contrast pass, Size presets, type roles and [`resolve`]), the few
//! component tokens (the slider thumb), the user's selection with its resolution
//! ([`resolve_selection`]), and
//! [`explain`], which reports how each colour got its value and every contrast pair measured.
//!
//! Pure data and functions: no egui, no I/O, no threads. Theme files on disk, the suite folder
//! and change watching belong to `craft-theme-store`; `craft_ui::theme` turns tokens into egui
//! ones.
//!
//! ```
//! use craft_theme::{Appearance, Contrast, Mode, Size, TextSize, format, resolve};
//!
//! let file = br##"{"schema": "craft-design-theme", "schema_version": 1, "id": "example",
//!     "seeds": {"neutral": "#8b8d98", "accent": "#3e63dd",
//!               "status": {"good": "#30a46c", "warn": "#ffc53d", "bad": "#e5484d", "info": "#0090ff"}}}"##;
//! let theme = format::parse(file).unwrap().theme;
//! let appearance = Appearance { mode: Mode::Dark, contrast: Contrast::Standard, size: Size::Compact, text_size: TextSize::Small, reduced_motion: false };
//! let resolved = resolve(&theme, appearance, None);
//! assert!(resolved.warnings.is_empty());
//! ```

mod app_sizes;
mod builtins;
pub mod color;
mod color_model;
mod components;
pub mod contrast;
mod derive;
pub mod dump;
mod explain;
pub mod format;
mod id;
mod identify;
mod parts;
pub mod ramp;
pub mod report;
mod resolve;
pub mod scale;
pub mod selection;
pub mod sizes;
mod stateful;
mod theme;
mod token_group;
mod tokens;
mod values;
pub mod variant;

#[cfg(test)]
mod app_sizes_tests;
#[cfg(test)]
mod color_model_tests;
#[cfg(test)]
mod explain_tests;
#[cfg(test)]
mod format_tests;
#[cfg(test)]
mod semantic_tests;
#[cfg(test)]
mod sizes_tests;
#[cfg(test)]
mod stateful_tests;
#[cfg(test)]
mod tests;

pub use app_sizes::{AppSizes, SizeMap, SizeValue};
pub use builtins::builtins;
pub use color::{Hsb, Lab, Oklab, Oklch, Rgba, Srgba};
pub use color_model::{
    Channel, ColorModel, DefaultOn, HsbModel, LabModel, MAX_CHANNELS, ModelValues, OklchModel, RgbModel, builtin_models, kept_or_converted,
};
pub use components::{ComponentField, ComponentTokens, SliderTokens};
pub use contrast::{contrast_ratio, ensure_contrast, relative_luminance, text_contrast_min};
pub use derive::{Anchoring, ContrastCheck, PassMove, Ramps, over};
pub use explain::{ColorOrigin, Explained, PairReport, SizeLayer, SizeOrigin, explain, explain_sized};
pub use id::{APP_DEFAULT, AppId, IdError, RESERVED_PREFIX, SUITE_DEFAULT, ThemeId};
pub use identify::{Identify, identifiable_outline, visible_hover};
pub use parts::{
    BadgeColors, BadgeStyle, BadgeTone, BorderColors, ButtonColors, ButtonVariant, BySize, CheckOffColors, CheckOnColors, CheckSwitchColors,
    ControlSize, Dimmed, FieldPart, ItemColors, LabelColor, MenuTint, State, States, ThumbColors,
};
pub use ramp::{Mode, Ramp, RampKind};
pub use resolve::{Resolved, resolve, resolve_sized};
pub use scale::{Duration, Easing, Elevation, Opacity, Radius, Space, Stroke};
pub use selection::{
    AppChoice, AppDefaults, ContrastChoice, Fallback, ModeChoice, MotionChoice, Selection, Shown, SuiteChoice, SystemAppearance, ThemePair,
    resolve_selection,
};
pub use stateful::{FlagMask, Flags, Mix, Resolve, StateMix, Stateful};
pub use theme::{
    AppOverrides, Appearance, Contrast, ContrastIntent, EdgeSeed, InterfaceFont, ModePins, ModeSeeds, PinLayer, Pins, RadiusOffset, SeedOverrides,
    Seeds, ShapeSeed, Size, TextSize, TextSizeSteps, ThemeDef, Variants,
};
pub use token_group::{GroupValues, TokenGroup};
pub use tokens::{
    AccentColors, BarSizes, CanvasSizes, CaptionColors, ColorToken, ColorTokens, ControlSizes, DataColors, DockColumnSizes, Edge, EdgeColors,
    ElevationTokens, FloatSizes, FontSlot, Gaps, HandleSizes, HitSizes, IconSizes, Insets, LengthSizes, LineColors, MarkerSizes, Motion,
    MotionTokens, OverlayColors, Pads, PaneSizes, PlotSizes, RadiusSet, RailSizes, RampSizes, RowStyle, RulerSizes, RulerTickSizes, ShapeTokens,
    SizeTokens, SpaceTokens, StateColors, Status, StatusColors, StatusKind, StrokeTokens, SurfaceColors, TextColors, ThumbSizes, TokenRef, Tokens,
    TypeRole, TypeStyle, TypeTokens, WindowSizes,
};
pub use values::ColorSource;
pub use variant::{CustomVariant, Emphasis, RadiusStep, Variant, VariantLook};

#[cfg(test)]
extern crate self as craft_theme;

#[cfg(test)]
#[path = "../tests/main.rs"]
mod integration;
