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

/// The fill of a check box, radio button or switch track while off: the sunken well, and the panel while disabled, so a
/// disabled off control is an empty frame.
fn well(cx: &crate::style::StyleCx<'_>) -> craft_theme::Stateful<crate::style::StyleColor> {
    use craft_theme::{Stateful, States};
    let colors = &cx.tokens.color;
    let (well, disabled) = (cx.color(colors.surface.sunken), cx.color(colors.state.disabled_fill));
    Stateful::new(well).off(States::new(well, well, well, disabled))
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
