//! `Button`, `IconButton`, `ToggleButton`, `SplitButton` and `Link`.

pub mod button;
pub(crate) mod face;
pub mod icon_button;
pub mod link;
pub mod split_button;
#[cfg(feature = "testing")]
mod stories;
pub mod toggle_button;
mod words;

use crate::runtime::env::EnvValue;

pub use button::{Button, ButtonOutput, ButtonStyle};
pub use face::Arrangement;
pub use icon_button::{IconButton, IconButtonStyle, PopupMark};
pub use link::{Link, LinkStyle};
pub use split_button::{SplitButton, SplitButtonStyle, SplitMenu, SplitOutput};
pub use toggle_button::{ToggleButton, ToggleButtonStyle};

/// How much of a ribbon group's content is shown. A group sets it for its content through `Overrides::values`, and
/// styles and measurements keyed on the env follow a change.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, EnvValue)]
#[env(style)]
#[non_exhaustive]
pub enum GroupLevel {
    /// Every control at full size, with its label.
    #[default]
    Full,
    /// Buttons drop their labels.
    Compact,
    /// The group is collapsed into one button.
    Collapsed,
}

/// Where controls are shown. A ribbon group sets `Ribbon` for its content (and for its collapsed button and the
/// popover that holds its content), and styles keyed on the env follow it:
///
/// - a `Button` or `SplitButton` that asks for no variant (or for the default `Variant::SECONDARY`) draws flat at
///   rest, as the ghost variant: no edge and no fill until the pointer is over it or presses it; a flat button's
///   sides take the control's padding (`space.pad.control`), not its corner round;
/// - a `ComboBox` is as wide as its widest option and its chevron, not a field length, and a font picker is a short
///   field (`size.length.short`) whichever family it shows.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, EnvValue)]
#[env(style)]
#[non_exhaustive]
pub enum ControlPlace {
    /// Anywhere but a ribbon: each control's own look and length.
    #[default]
    Standard,
    /// In a ribbon group.
    Ribbon,
}

/// The style context a push or split button draws in: in a ribbon (`ControlPlace::Ribbon`) the default variant
/// becomes the ghost one, so the button is flat at rest; any other variant, and any other place, stays as it is.
pub(crate) fn placed<'a>(cx: &crate::style::StyleCx<'a>) -> crate::style::StyleCx<'a> {
    if cx.variant == craft_theme::Variant::SECONDARY && cx.env::<ControlPlace>() == ControlPlace::Ribbon {
        cx.with_variant(craft_theme::Variant::GHOST)
    } else {
        *cx
    }
}

/// Whether a button styled in `cx` is a ribbon's flat button: the ghost variant in a ribbon (`ControlPlace::Ribbon`).
pub(crate) fn flat_in_ribbon(cx: &crate::style::StyleCx<'_>) -> bool {
    cx.variant == craft_theme::Variant::GHOST && cx.env::<ControlPlace>() == ControlPlace::Ribbon
}
