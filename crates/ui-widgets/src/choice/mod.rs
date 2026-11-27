//! `Checkbox`, `Switch`, `RadioGroup`, `Segmented` and `Rating`.

pub mod check;
pub mod radio;
pub(crate) mod rating;
pub(crate) mod segmented;
pub(crate) mod switch;
mod words;

pub(crate) mod swatch;
pub(crate) mod swatch_table;

pub use check::{Checkbox, CheckboxStyle};
pub use radio::{GroupStyle, Radio, RadioGroup, RadioKind, RadioStyle};
pub use rating::{Rating, RatingStyle};
pub use segmented::{Segmented, SegmentedStyle};
pub use switch::{Switch, SwitchStyle};
pub use words::ChoiceWords;

pub use swatch::{
    CellMarks, RING_SHARE, Swatch, SwatchFace, SwatchFill, SwatchImage, SwatchMetrics, SwatchOutput, SwatchShape, SwatchStyle, paint_ring, ring_band,
};
pub use swatch_table::{RowMarks, SwatchArrangement, SwatchRows, SwatchSpacing, SwatchTable, SwatchTableOutput, SwatchTableStyle};

/// How a check box, radio button or switch track that is on fills. An app sets it once at its root
/// (`RootEnv::values`); the default is the accent.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, crate::runtime::env::EnvValue, crate::contract::MemoProps)]
#[env(style)]
#[non_exhaustive]
pub enum ChoiceLook {
    /// `accent.solid` with a `text.on_accent` mark.
    #[default]
    Accent,
    /// `accent.strong` with a white `text.on_strong` mark: VectorCraft's.
    Strong,
}

/// The well of a check box, radio button or switch track with `radius` corners: the sunken surface, so a bevelled theme
/// draws its bevel, filled with the off fill or the on accent. A flat theme draws the outline as its edge, in the failure
/// colour while the value is invalid.
fn indicator(cx: &crate::style::StyleCx<'_>, radius: egui::CornerRadius) -> crate::style::SurfaceLook {
    use crate::style::{Flags, SurfaceKind, SurfaceLook};
    let mut look = SurfaceLook::new(well(cx).on(on_fill(cx).off)).kind(SurfaceKind::Sunken).radius(radius).border_replaces_edge();
    if matches!(cx.surface(SurfaceKind::Sunken).edge, crate::style::SurfaceEdge::Border(_)) {
        let edge = outline(cx).on(on_fill(cx).off).when(Flags::INVALID, cx.tone(craft_theme::variant::Tone::Bad));
        look = look.border(edge, cx.tokens.stroke.medium);
    }
    look
}

/// The well of a quiet check box: dark in either state with a thin faint edge, so the box reads over a picture and
/// its checked state is told by the mark alone.
fn quiet_indicator(cx: &crate::style::StyleCx<'_>, radius: egui::CornerRadius) -> crate::style::SurfaceLook {
    use crate::style::{SurfaceKind, SurfaceLook};
    use craft_theme::{Stateful, States};
    let dark = cx.color(cx.tokens.color.surface.scrim);
    let edge = cx.color(cx.tokens.color.text.faint);
    let fill = Stateful::new(dark).off(States::new(dark, dark, dark, dark));
    SurfaceLook::new(fill).kind(SurfaceKind::Sunken).radius(radius).border(Stateful::new(edge), cx.tokens.stroke.thin)
}

/// The mark of a quiet check box: `text.on_strong`, white in every theme, in both states: the well under it is the
/// scrim, dark in every theme.
fn quiet_mark(cx: &crate::style::StyleCx<'_>) -> craft_theme::Stateful<crate::style::StyleColor> {
    craft_theme::Stateful::new(cx.color(cx.tokens.color.text.on_strong))
}

/// The fill of a check box, radio button or switch track while off: the sunken well, the state layer's press fill while
/// pressed, and the panel while disabled, so a disabled off control is an empty frame.
fn well(cx: &crate::style::StyleCx<'_>) -> craft_theme::Stateful<crate::style::StyleColor> {
    use craft_theme::{Stateful, States};
    let colors = &cx.tokens.color;
    let (well, pressed, disabled) = (cx.color(colors.surface.sunken), cx.color(colors.state.pressed), cx.color(colors.state.disabled_fill));
    Stateful::new(well).off(States::new(well, well, pressed, disabled))
}

/// The fill of a check box, radio button or switch track while on: the accent of the [`ChoiceLook`], its pressed shade
/// while pressed, and `text.disabled` while disabled.
fn on_fill(cx: &crate::style::StyleCx<'_>) -> craft_theme::Stateful<crate::style::StyleColor> {
    use craft_theme::{Stateful, States};
    let accent = &cx.tokens.color.accent;
    let (solid, pressed) = match cx.env::<ChoiceLook>() {
        ChoiceLook::Accent => (accent.solid, accent.pressed),
        ChoiceLook::Strong => (accent.strong, accent.strong_pressed),
    };
    let (solid, pressed, disabled) = (cx.color(solid), cx.color(pressed), cx.color(cx.tokens.color.text.disabled));
    Stateful::new(solid).off(States::new(solid, solid, pressed, disabled))
}

/// The mark on the on fill (check, dash, dot): the text colour paired with the [`ChoiceLook`]'s accent, and the panel
/// while disabled, cut into the disabled box.
fn mark(cx: &crate::style::StyleCx<'_>) -> craft_theme::Stateful<crate::style::StyleColor> {
    use craft_theme::{Stateful, States};
    let colors = &cx.tokens.color;
    let mark = match cx.env::<ChoiceLook>() {
        ChoiceLook::Accent => colors.text.on_accent,
        ChoiceLook::Strong => colors.text.on_strong,
    };
    let (mark, disabled) = (cx.color(mark), cx.color(colors.state.disabled_fill));
    Stateful::new(mark).off(States::new(mark, mark, mark, disabled))
}

/// The outline of a check box, radio button or switch track: `text.faint`, moved only as far as it needs to be
/// identifiable on the sunken fill and on the panel and raised surfaces; `text.muted` under the pointer.
fn outline(cx: &crate::style::StyleCx<'_>) -> craft_theme::Stateful<crate::style::StyleColor> {
    use craft_theme::{Contrast, Stateful, States};
    let colors = &cx.tokens.color;
    let high = cx.contrast() == Contrast::High;
    let rest =
        cx.color(craft_theme::identifiable_outline(colors.text.faint, colors.surface.sunken, &[colors.surface.panel, colors.surface.raised], high));
    let (muted, disabled) = (cx.color(colors.text.muted), cx.color(colors.text.disabled));
    Stateful::new(rest).off(States::new(rest, muted, muted, disabled))
}
