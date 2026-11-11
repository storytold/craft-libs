//! Plain value types the component styles in `craft-ui` build their looks from: sizes by control
//! size, colours by state, and the labelled colour sets of buttons, badges, checks and fields.
//! They hold no theme data of their own.

use crate::color::Rgba;
use crate::contrast::{contrast_ratio, ensure_contrast};
use crate::derive::over;
use crate::tokens::ColorTokens;
use crate::values;

/// The two control sizes a family offers.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum ControlSize {
    /// Small: toolbars, dense rows.
    Sm,
    /// Standard.
    Md,
}

/// One value per [`ControlSize`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct BySize<T> {
    /// Small.
    pub sm: T,
    /// Standard.
    pub md: T,
}

impl<T: Copy> BySize<T> {
    /// The value for `size`.
    pub fn get(&self, size: ControlSize) -> T {
        match size {
            ControlSize::Sm => self.sm,
            ControlSize::Md => self.md,
        }
    }
}

/// An interaction state. Keyboard focus is not one: the `FocusRing` primitive draws it from
/// `line.focus`. Selected and checked belong to the families that have them.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum State {
    /// Idle.
    Rest,
    /// Under the pointer.
    Hover,
    /// While pressed.
    Pressed,
    /// Not interactive.
    Disabled,
}

impl State {
    /// Every state, in path order.
    pub const ALL: [State; 4] = [State::Rest, State::Hover, State::Pressed, State::Disabled];

    /// The path segment, e.g. `"hover"`.
    pub const fn name(self) -> &'static str {
        match self {
            State::Rest => "rest",
            State::Hover => "hover",
            State::Pressed => "pressed",
            State::Disabled => "disabled",
        }
    }

    /// The state whose [`name`](State::name) is `name`.
    pub fn from_name(name: &str) -> Option<State> {
        State::ALL.into_iter().find(|s| s.name() == name)
    }
}

/// One value per [`State`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct States<T> {
    /// Idle.
    pub rest: T,
    /// Under the pointer.
    pub hover: T,
    /// While pressed.
    pub pressed: T,
    /// Not interactive.
    pub disabled: T,
}

impl<T: Copy> States<T> {
    /// The four values, in [`State::ALL`] order.
    pub const fn new(rest: T, hover: T, pressed: T, disabled: T) -> States<T> {
        States { rest, hover, pressed, disabled }
    }

    /// The value for `state`.
    pub fn get(&self, state: State) -> T {
        match state {
            State::Rest => self.rest,
            State::Hover => self.hover,
            State::Pressed => self.pressed,
            State::Disabled => self.disabled,
        }
    }

    /// The value for `state`, to change it.
    pub fn get_mut(&mut self, state: State) -> &mut T {
        match state {
            State::Rest => &mut self.rest,
            State::Hover => &mut self.hover,
            State::Pressed => &mut self.pressed,
            State::Disabled => &mut self.disabled,
        }
    }
}

/// The colours of an item without an outline, per state.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct ItemColors {
    /// Background.
    pub fill: States<Rgba>,
    /// Label and icon.
    pub text: States<Rgba>,
}

/// The colours of a knob that rides on a track, per state.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct ThumbColors {
    /// Face.
    pub fill: States<Rgba>,
    /// Outline.
    pub border: States<Rgba>,
}

/// A button's emphasis.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum ButtonVariant {
    /// The main action: accent fill.
    Primary,
    /// Other actions: neutral fill and border.
    Secondary,
    /// Toolbar and inline actions: no fill until hovered.
    Ghost,
    /// Destructive actions: `status.bad` fill.
    Danger,
}

impl ButtonVariant {
    /// Every variant, in path order.
    pub const ALL: [ButtonVariant; 4] = [ButtonVariant::Primary, ButtonVariant::Secondary, ButtonVariant::Ghost, ButtonVariant::Danger];

    /// The path segment, e.g. `"primary"`.
    pub const fn name(self) -> &'static str {
        match self {
            ButtonVariant::Primary => "primary",
            ButtonVariant::Secondary => "secondary",
            ButtonVariant::Ghost => "ghost",
            ButtonVariant::Danger => "danger",
        }
    }
}

/// The colours of one variant.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct ButtonColors {
    /// Background.
    pub fill: States<Rgba>,
    /// Outline, `border_width` wide.
    pub border: States<Rgba>,
    /// Label, icon and spinner.
    pub text: States<Rgba>,
}

/// What a badge says about its subject.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Default)]
pub enum BadgeTone {
    /// No status: a count, a tag.
    #[default]
    Neutral,
    /// The accent: new, selected.
    Accent,
    /// Success.
    Good,
    /// Needs attention.
    Warn,
    /// Failure.
    Bad,
    /// Information.
    Info,
}

impl BadgeTone {
    /// Every tone, in path order.
    pub const ALL: [BadgeTone; 6] = [BadgeTone::Neutral, BadgeTone::Accent, BadgeTone::Good, BadgeTone::Warn, BadgeTone::Bad, BadgeTone::Info];

    /// The path segment, e.g. `"good"`.
    pub const fn name(self) -> &'static str {
        match self {
            BadgeTone::Neutral => "neutral",
            BadgeTone::Accent => "accent",
            BadgeTone::Good => "good",
            BadgeTone::Warn => "warn",
            BadgeTone::Bad => "bad",
            BadgeTone::Info => "info",
        }
    }

    /// The tone whose [`name`](BadgeTone::name) is `name`.
    pub fn from_name(name: &str) -> Option<BadgeTone> {
        BadgeTone::ALL.into_iter().find(|t| t.name() == name)
    }
}

/// How a badge fills its tone.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Default)]
pub enum BadgeStyle {
    /// The tone's tinted background, text and border.
    #[default]
    Soft,
    /// The tone's solid fill, no border.
    Solid,
}

impl BadgeStyle {
    /// Both styles, in path order.
    pub const ALL: [BadgeStyle; 2] = [BadgeStyle::Soft, BadgeStyle::Solid];

    /// The path segment, e.g. `"soft"`.
    pub const fn name(self) -> &'static str {
        match self {
            BadgeStyle::Soft => "soft",
            BadgeStyle::Solid => "solid",
        }
    }

    /// The style whose [`name`](BadgeStyle::name) is `name`.
    pub fn from_name(name: &str) -> Option<BadgeStyle> {
        BadgeStyle::ALL.into_iter().find(|s| s.name() == name)
    }
}

/// The colours of one tone in one style.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct BadgeColors {
    /// Background.
    pub fill: Rgba,
    /// Label.
    pub text: Rgba,
    /// Outline.
    pub border: Rgba,
}

/// The colours of the indicator while off, per state. Off has no mark.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct CheckOffColors {
    /// Inside of the box, circle or track.
    pub fill: States<Rgba>,
    /// Outline of the box, circle or track.
    pub border: States<Rgba>,
}

/// The colours of the indicator and its mark while on, per state.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct CheckOnColors {
    /// Inside of the box, circle or track.
    pub fill: States<Rgba>,
    /// Outline of the box, circle or track.
    pub border: States<Rgba>,
    /// Check, dash or dot.
    pub mark: States<Rgba>,
}

/// The colours of a switch track while on, per state. A switch draws no mark.
pub type CheckSwitchColors = CheckOffColors;

/// A part of a field that has one colour while enabled and one while disabled.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum FieldPart {
    /// The inside of the box.
    Fill,
    /// The typed text.
    Text,
    /// The text shown while the field is empty.
    Placeholder,
    /// A number's unit, drawn at the box's end.
    Unit,
    /// The label or icon at a number field's start that a drag scrubs the value with.
    Scrub,
}

impl FieldPart {
    /// Every part, in path order.
    pub const ALL: [FieldPart; 5] = [FieldPart::Fill, FieldPart::Text, FieldPart::Placeholder, FieldPart::Unit, FieldPart::Scrub];

    /// The path segment, e.g. `"placeholder"`.
    pub const fn name(self) -> &'static str {
        match self {
            FieldPart::Fill => "fill",
            FieldPart::Text => "text",
            FieldPart::Placeholder => "placeholder",
            FieldPart::Unit => "unit",
            FieldPart::Scrub => "scrub",
        }
    }
}

/// A colour while enabled and while disabled.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Dimmed {
    /// Enabled, in every state.
    pub rest: Rgba,
    /// Disabled.
    pub disabled: Rgba,
}

impl Dimmed {
    /// The colour while enabled or not.
    pub fn get(&self, enabled: bool) -> Rgba {
        if enabled { self.rest } else { self.disabled }
    }
}

/// The colours of a field's edge, per [`BorderState`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct BorderColors {
    /// Idle.
    pub rest: Rgba,
    /// Under the pointer.
    pub hover: Rgba,
    /// With keyboard focus.
    pub focused: Rgba,
    /// Not interactive.
    pub disabled: Rgba,
}

/// PhotoCraft's layer label colours (`LabelColor`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum LabelColor {
    #[allow(missing_docs)]
    Red,
    #[allow(missing_docs)]
    Orange,
    #[allow(missing_docs)]
    Yellow,
    #[allow(missing_docs)]
    Green,
    #[allow(missing_docs)]
    Seafoam,
    #[allow(missing_docs)]
    Blue,
    #[allow(missing_docs)]
    Indigo,
    #[allow(missing_docs)]
    Magenta,
    #[allow(missing_docs)]
    Fuchsia,
    #[allow(missing_docs)]
    Violet,
    #[allow(missing_docs)]
    Gray,
}

impl LabelColor {
    /// Every label colour, in path order.
    pub const ALL: [LabelColor; 11] = [
        LabelColor::Red,
        LabelColor::Orange,
        LabelColor::Yellow,
        LabelColor::Green,
        LabelColor::Seafoam,
        LabelColor::Blue,
        LabelColor::Indigo,
        LabelColor::Magenta,
        LabelColor::Fuchsia,
        LabelColor::Violet,
        LabelColor::Gray,
    ];

    /// The path segment.
    pub const fn name(self) -> &'static str {
        match self {
            LabelColor::Red => "red",
            LabelColor::Orange => "orange",
            LabelColor::Yellow => "yellow",
            LabelColor::Green => "green",
            LabelColor::Seafoam => "seafoam",
            LabelColor::Blue => "blue",
            LabelColor::Indigo => "indigo",
            LabelColor::Magenta => "magenta",
            LabelColor::Fuchsia => "fuchsia",
            LabelColor::Violet => "violet",
            LabelColor::Gray => "gray",
        }
    }

    const fn fill_and_swatch(self) -> (Rgba, Rgba) {
        match self {
            LabelColor::Red => (Rgba::rgb(0x9F, 0x2F, 0x30), Rgba::rgb(0xFC, 0x5D, 0x5B)),
            LabelColor::Orange => (Rgba::rgb(0x93, 0x4F, 0x0C), Rgba::rgb(0xF7, 0x97, 0x44)),
            LabelColor::Yellow => (Rgba::rgb(0x99, 0x78, 0x0C), Rgba::rgb(0xDC, 0xD6, 0x4B)),
            LabelColor::Green => (Rgba::rgb(0x4E, 0x71, 0x2E), Rgba::rgb(0x85, 0xDC, 0x6A)),
            LabelColor::Seafoam => (Rgba::rgb(0x0B, 0x54, 0x4F), Rgba::rgb(0x1C, 0x84, 0x88)),
            LabelColor::Blue => (Rgba::rgb(0x41, 0x5B, 0x87), Rgba::rgb(0x78, 0xAD, 0xF6)),
            LabelColor::Indigo => (Rgba::rgb(0x36, 0x34, 0x8E), Rgba::rgb(0x54, 0x4B, 0xE7)),
            LabelColor::Magenta => (Rgba::rgb(0x98, 0x1B, 0x51), Rgba::rgb(0xCC, 0x1A, 0x7E)),
            LabelColor::Fuchsia => (Rgba::rgb(0x71, 0x0F, 0x74), Rgba::rgb(0xB0, 0x14, 0xC0)),
            LabelColor::Violet => (Rgba::rgb(0x5D, 0x3F, 0x8E), Rgba::rgb(0x91, 0x76, 0xD5)),
            LabelColor::Gray => (Rgba::rgb(0x57, 0x57, 0x57), Rgba::rgb(0x9C, 0x9C, 0x9C)),
        }
    }

    /// The label in a colour menu or on a tile. A fixed data colour, not a theme colour.
    pub const fn swatch(self) -> Rgba {
        self.fill_and_swatch().1
    }

    /// Behind a toggle cell of a row with this label. A fixed data colour, not a theme colour.
    pub const fn fill(self) -> Rgba {
        self.fill_and_swatch().0
    }

    /// The toggle cell's icon on [`fill`](LabelColor::fill). A fixed data colour, not a theme colour.
    pub const fn icon(self) -> Rgba {
        Rgba::rgb(0xE2, 0xE2, 0xE2)
    }
}

/// The colours of a row with a label colour.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MenuTint {
    /// Behind the row: the label colour over `surface.raised`, opaque.
    pub fill: Rgba,
    /// The label, icon, check or radio mark and submenu arrow.
    pub text: Rgba,
    /// The shortcut.
    pub shortcut: Rgba,
}

impl MenuTint {
    /// The tint of `color` (its alpha scales the strength) for a menu on the popup card `base`
    /// with the text colours of `c`. It starts at `values::MENU_TINT_ALPHA` of the colour; `text.normal` and `text.muted`
    /// then move in OKLCH lightness until they reach the text minimum (4.5:1, 7:1 with `high`)
    /// on the fill. When they can't, the alpha drops by 0.01 until they do; at 0 the fill is the
    /// card itself. The check and radio marks and the arrow use the label colour, which has
    /// the text minimum, more than the 3:1 (4.5:1) that marks need.
    pub fn derive(color: Rgba, base: Rgba, c: &ColorTokens, high: bool) -> MenuTint {
        let min = if high { values::TEXT_MIN.1 } else { values::TEXT_MIN.0 };
        let start = f64::from(values::MENU_TINT_ALPHA) * f64::from(color.a()) / 255.0;
        let steps = (values::MENU_TINT_ALPHA * 100.0).round() as u32;
        let mut tint = MenuTint { fill: base, text: c.text.normal, shortcut: c.text.muted };
        for k in (0..=steps).rev() {
            let alpha = start * f64::from(k) / f64::from(steps);
            let fill = over(Rgba([color.0[0], color.0[1], color.0[2], (alpha * 255.0).round() as u8]), base);
            tint = MenuTint { fill, text: ensure_contrast(c.text.normal, fill, min), shortcut: ensure_contrast(c.text.muted, fill, min) };
            if contrast_ratio(tint.text, fill) >= min && contrast_ratio(tint.shortcut, fill) >= min {
                break;
            }
        }
        tint
    }
}
