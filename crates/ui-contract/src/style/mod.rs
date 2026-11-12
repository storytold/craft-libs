//! The style layer: the state set and its resolver, style parts and `ComponentStyle`, tokens (`UiTokens`,
//! `StyleCx`), the token-group cache and the style cache.

pub(crate) mod cache;
pub(crate) mod cx;
mod key;
mod look;
mod paint;
mod replacements;
mod state;
mod surface;
pub(crate) mod text_measure;
mod tokens;

pub use cache::{Accepted, accepted_variant};
pub use craft_behavior::{ControlMetrics, Insets, Metrics};
#[doc(hidden)]
pub use craft_theme::GroupValues;
pub use craft_theme::{FlagMask, Flags, Mix, Resolve, State, StateMix, Stateful, States, TokenGroup, Tokens, TypeRole, Variant};
pub use craft_ui_macros::{ComponentStyle, TokenGroup};
pub(crate) use cx::control_metrics;
pub use cx::{StyleCx, VariantParts};
pub use key::{EnvKey, Ink, StyleKey};
pub(crate) use look::StyleView;
pub use look::{ComponentStyle, Corners, DataColor, FocusLook, IconLook, StyleColor, SurfaceKind, SurfaceLook, TextLook, TrackLook};
pub use paint::{
    HandleLook, MarkerLook, ResetMarkLook, ResizeLineLook, RuleKind, RuleLook, RuleWidth, ScrollBarKind, ScrollBarLook, resize_line_look,
};
pub(crate) use paint::{min_thumb, scroll_bar_look};
pub use replacements::{DuplicateStyle, StyleReplacements};
pub use state::StateSet;
pub use surface::{SurfaceEdge, SurfaceStyle};
pub use tokens::UiTokens;

#[doc(hidden)]
pub use cache::resolve_style;
#[doc(hidden)]
pub use paint::color as rgba_color;
#[doc(hidden)]
pub use paint::ink_color;
pub(crate) use paint::{rule_look, scrim_color, state_layer_look};
pub(crate) use surface::surface_style;
