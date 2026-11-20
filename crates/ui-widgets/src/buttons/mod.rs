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
/// - a `Button`, `SplitButton` or `ToggleButton` that asks for no variant (or for the default `Variant::SECONDARY`)
///   draws flat at rest, as the ghost variant: no edge and no fill until the pointer is over it or presses it, and a
///   toggle shows the on plate while on; a flat button's sides take the control's padding (`space.pad.control`), not
///   its corner round;
/// - a `ComboBox` is as wide as its widest option and its chevron, not a field length, and a font picker is a short
///   field (`size.length.short`) whichever family it shows.
///
/// A page's side menu (a File page's list of pages) sets `SideMenu` for its rows: a `Button` there shows its label in
/// the large body role (`TypeRole::BodyLarge`), with `space.pad.segment` above and below it and `space.pad.roomy`
/// at its sides, so the menu reads as a list of places rather than a row of controls.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, EnvValue)]
#[env(style)]
#[non_exhaustive]
pub enum ControlPlace {
    /// Anywhere but a ribbon: each control's own look and length.
    #[default]
    Standard,
    /// In a ribbon group.
    Ribbon,
    /// In a page's side menu.
    SideMenu,
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

/// The icon sides of the buttons in a ribbon (`ControlPlace::Ribbon`), in points: `large` for a stacked button's
/// leading icon, `small` for every other button's icon and an icon-only button's glyph. A side of 0 keeps the
/// framework's: `size.icon.lg` (or the control's icon, when larger) for a stacked icon, the control's icon or glyph
/// for the rest. An app whose ribbon draws its icons larger than its other controls sets it on the ribbon
/// (`Ribbon::icons`); outside a ribbon it changes nothing.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, EnvValue)]
#[env(style)]
pub struct RibbonIcons {
    /// A stacked button's leading icon.
    pub large: u8,
    /// Every other button's icon and glyph.
    pub small: u8,
    /// The room a menu arrow after a label takes. Set, the ribbon's buttons draw that arrow as the small filled
    /// triangle right after their label, as the old ribbon drew it, in place of the stroked chevron.
    pub arrow: u8,
}

impl RibbonIcons {
    /// Stacked icons `large` points square and the others `small`; 0 keeps the framework's side.
    pub const fn new(large: u8, small: u8) -> Self {
        RibbonIcons { large, small, arrow: 0 }
    }

    /// Menu arrows as filled triangles in `arrow` points of room right after the label; 0 keeps the chevron.
    pub const fn arrow(mut self, arrow: u8) -> Self {
        self.arrow = arrow;
        self
    }
}

/// The side of a non-stacked button's icon or glyph in `cx`: the ribbon's small side where one is set, else `side`.
pub(crate) fn small_icon(cx: &crate::style::StyleCx<'_>, side: f32) -> f32 {
    let small = cx.env::<RibbonIcons>().small;
    if small > 0 && cx.env::<ControlPlace>() == ControlPlace::Ribbon { f32::from(small) } else { side }
}

/// Whether a button styled in `cx` is a ribbon's flat button: the ghost variant in a ribbon (`ControlPlace::Ribbon`).
pub(crate) fn flat_in_ribbon(cx: &crate::style::StyleCx<'_>) -> bool {
    cx.variant == craft_theme::Variant::GHOST && cx.env::<ControlPlace>() == ControlPlace::Ribbon
}
