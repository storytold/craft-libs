//! `Button`, `IconButton`, `ToggleButton`, `SplitButton`, `Link` and `CommandSearch`.

pub mod button;
pub mod command_search;
pub(crate) mod face;
pub mod icon_button;
pub mod link;
pub mod split_button;
#[cfg(feature = "suite")]
pub(crate) mod stories;
pub mod toggle_button;
mod words;

use crate::contract::MemoProps;
use crate::runtime::env::EnvValue;

pub use button::{Button, ButtonChord, ButtonOutput, ButtonStyle};
pub use command_search::{CommandSearch, CommandSearchStyle};
pub use face::Arrangement;
pub use icon_button::{IconButton, IconButtonStyle, PopupMark, PressFocus};
pub use link::{Link, LinkStyle};
pub use split_button::{SplitButton, SplitButtonStyle, SplitMenu, SplitOutput};
pub use toggle_button::{ToggleButton, ToggleButtonStyle};

/// How much of a ribbon group's content is shown. A group sets it for its content through `Overrides::values`, and
/// styles and measurements keyed on the env follow a change.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, EnvValue, MemoProps)]
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

/// How a `Link` that stands alone marks itself beside its colour. An app sets it once at its root (`RootEnv::values`);
/// a link in a sentence stays underlined, since the text around it shares its line.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, EnvValue, MemoProps)]
#[env(style)]
#[non_exhaustive]
pub enum LinkLook {
    /// Underlined, so colour is not its only cue.
    #[default]
    Underlined,
    /// The link colour alone, as a list of links under its heading on a start page (VectorCraft's).
    Plain,
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
///
/// A page's call to action (a start page's primary link) sets `CallToAction`: a `Button` there stands
/// `size.page.action` high, its label strong.
///
/// A start page's New and Open stand in `PageAction`: a `Button` there stands at least `size.page.button` high, with at
/// least `size.page.button_side` at each side of its label.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, EnvValue, MemoProps)]
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
    /// A page's call to action.
    CallToAction,
    /// A start page's New and Open.
    PageAction,
}

/// How an icon button or a segment that is on shows it. A tool rail sets it for its buttons (`ToolRail::active_look`),
/// so the current tool can show the look an app's tools had; an app sets it at its root (`RootEnv::values`) for its
/// segmented pickers as well.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, EnvValue, MemoProps)]
#[env(style)]
#[non_exhaustive]
pub enum ActiveLook {
    /// The accent's soft fill and edge, with the accent's text colour.
    #[default]
    Accent,
    /// The pressed state's fill with no edge, and the glyph in the normal text colour.
    Pressed,
    /// The `surface.control` fill with no edge, and the glyph in the normal text colour: the open panel's icon of a
    /// dock in VectorCraft 0.24's block look (`PanelTabLook::Block`).
    Control,
}

/// The look of a push or split button that asks for no variant. An app sets it once at its root (`RootEnv::values`);
/// the default is the outline. A button that asks for a variant, `Variant::SECONDARY` included, keeps it; a ribbon's
/// flat look (`ControlPlace::Ribbon`) wins over it, and toggles keep the outline.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, EnvValue, MemoProps)]
#[env(style)]
#[non_exhaustive]
pub enum PushButtonLook {
    /// The outline, filled only by state.
    #[default]
    Outline,
    /// The filled neutral tile with no edge at rest (`Emphasis::Filled`): VectorCraft's buttons.
    Filled,
}

/// The filled neutral variant a push button takes under [`PushButtonLook::Filled`]: `surface.control` with no edge at
/// rest.
pub const FILLED: craft_theme::Variant = craft_theme::Variant::look(craft_theme::Emphasis::Filled, craft_theme::variant::Tone::Neutral);

/// The variant a push or split button that asks for none takes in `env` (`Component::default_variant`): the filled one
/// under [`PushButtonLook::Filled`], outside a ribbon.
pub(crate) fn push_default(env: &crate::runtime::env::Env) -> Option<craft_theme::Variant> {
    (env.get::<PushButtonLook>() == PushButtonLook::Filled && env.get::<ControlPlace>() != ControlPlace::Ribbon).then_some(FILLED)
}

/// The style context a push, split or labelled toggle button draws in: in a ribbon (`ControlPlace::Ribbon`) the default
/// variant becomes the ghost one, so the button is flat at rest; any other variant, and any other place, stays as it is.
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
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, EnvValue, MemoProps)]
#[env(style)]
#[non_exhaustive]
pub struct RibbonLook {
    /// A stacked button's leading icon.
    pub large: u8,
    /// Every other button's icon and glyph.
    pub small: u8,
    /// The room a menu arrow after a label takes. Set, the ribbon's buttons draw that arrow as the small filled
    /// triangle right after their label, as the old ribbon drew it, in place of the stroked chevron.
    pub arrow: u8,
    /// The height of a non-stacked button, so three of them, a tight gap apart, fill the ribbon's body (the old
    /// ribbon's 22-point pitch); 0 keeps the control's height.
    pub row: u8,
    /// Whether a ribbon strip's tabs rest in the normal text colour, as the old ribbon's did, not the muted one.
    pub tab_ink: bool,
    /// The height of a ribbon strip's underline under the shown tab; 0 keeps the thick stroke.
    pub underline: u8,
    /// Whether the ribbon's body stands on its own lighter fill, ruled at its top and bottom, as the old ribbon's did.
    pub body: bool,
}

impl RibbonLook {
    /// Stacked icons `large` points square and the others `small`; 0 keeps the framework's side.
    pub const fn new(large: u8, small: u8) -> Self {
        RibbonLook { large, small, arrow: 0, row: 0, tab_ink: false, underline: 0, body: false }
    }

    /// The strip's tabs in the normal text colour at rest, and its underline `underline` points high (0 keeps the
    /// thick stroke).
    pub const fn tabs(mut self, underline: u8) -> Self {
        self.tab_ink = true;
        self.underline = underline;
        self
    }

    /// The body on its own lighter fill, ruled at its top and bottom.
    pub const fn body(mut self) -> Self {
        self.body = true;
        self
    }

    /// Non-stacked buttons `row` points high; 0 keeps the control's height.
    pub const fn row(mut self, row: u8) -> Self {
        self.row = row;
        self
    }

    /// Menu arrows as filled triangles in `arrow` points of room right after the label; 0 keeps the chevron.
    pub const fn arrow(mut self, arrow: u8) -> Self {
        self.arrow = arrow;
        self
    }
}

/// The height of a one-line control in `cx` (a non-stacked button, a check box): a ribbon look's row height
/// (`RibbonLook::row`) in a ribbon, else `height`.
pub(crate) fn row_height(cx: &crate::style::StyleCx<'_>, height: f32) -> f32 {
    let look = cx.env::<RibbonLook>();
    if look.row > 0 && cx.env::<ControlPlace>() == ControlPlace::Ribbon { f32::from(look.row) } else { height }
}

/// The side of a non-stacked button's icon or glyph in `cx`: the ribbon's small side where one is set, else `side`.
pub(crate) fn small_icon(cx: &crate::style::StyleCx<'_>, side: f32) -> f32 {
    let small = cx.env::<RibbonLook>().small;
    if small > 0 && cx.env::<ControlPlace>() == ControlPlace::Ribbon { f32::from(small) } else { side }
}

/// Whether a button styled in `cx` is a ribbon's flat button: the ghost variant in a ribbon (`ControlPlace::Ribbon`).
pub(crate) fn flat_in_ribbon(cx: &crate::style::StyleCx<'_>) -> bool {
    cx.variant == craft_theme::Variant::GHOST && cx.env::<ControlPlace>() == ControlPlace::Ribbon
}
