//! `Checkbox`, `Switch`, `RadioGroup`, `Segmented` and `Rating`.

pub mod check;
pub mod radio;
mod rating;
mod segmented;
mod switch;
mod words;

mod swatch;

pub use check::{Checkbox, CheckboxStyle};
pub use radio::{GroupStyle, Radio, RadioGroup, RadioKind, RadioStyle};
pub use rating::{Rating, RatingStyle};
pub use segmented::{Segmented, SegmentedStyle};
pub use switch::{Switch, SwitchStyle};
pub use words::ChoiceWords;

pub use swatch::{Swatch, SwatchFace, SwatchMetrics, SwatchOutput, SwatchStyle};

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

/// The fill of a check box, radio button or switch track while off: the sunken well, the state layer's press fill while
/// pressed, and the panel while disabled, so a disabled off control is an empty frame.
fn well(cx: &crate::style::StyleCx<'_>) -> craft_theme::Stateful<crate::style::StyleColor> {
    use craft_theme::{Stateful, States};
    let colors = &cx.tokens.color;
    let (well, pressed, disabled) = (cx.color(colors.surface.sunken), cx.color(colors.state.pressed), cx.color(colors.state.disabled_fill));
    Stateful::new(well).off(States::new(well, well, pressed, disabled))
}

/// The fill of a check box, radio button or switch track while on: the accent, its pressed shade while pressed, and
/// `text.disabled` while disabled.
fn on_fill(cx: &crate::style::StyleCx<'_>) -> craft_theme::Stateful<crate::style::StyleColor> {
    use craft_theme::{Stateful, States};
    let accent = &cx.tokens.color.accent;
    let (solid, pressed, disabled) = (cx.color(accent.solid), cx.color(accent.pressed), cx.color(cx.tokens.color.text.disabled));
    Stateful::new(solid).off(States::new(solid, solid, pressed, disabled))
}

/// The mark on an on-accent fill (check, dash, dot): `text.on_accent`, and the panel while disabled, cut into the
/// disabled box.
fn mark(cx: &crate::style::StyleCx<'_>) -> craft_theme::Stateful<crate::style::StyleColor> {
    use craft_theme::{Stateful, States};
    let colors = &cx.tokens.color;
    let (mark, disabled) = (cx.color(colors.text.on_accent), cx.color(colors.state.disabled_fill));
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
