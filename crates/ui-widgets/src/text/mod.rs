//! Text and number fields: `TextField<C>` and `NumberField` on the text part (18.3).

mod adorn;
pub(crate) mod alt_text;
pub(crate) mod chips;
mod completions;
pub(crate) mod hex;
pub(crate) mod linked;
pub(crate) mod number;
pub(crate) mod selectable_text;
pub(crate) mod text_field;
pub(crate) mod word_run;
mod words;
pub(crate) mod zoom_field;

pub use adorn::{Adornment, Placed, place, reserve};
pub use alt_text::{AltText, AltTextField, AltTextFieldStyle, AltTextWords};
pub use chips::{ChipCodec, ChipField, ChipFieldOutput, ChipFieldStyle, ChipWords, PartialChips};
pub use hex::{HexCodec, HexField};
pub use linked::{LinkMode, LinkedFields, LinkedFieldsStyle, LinkedLayout, LinkedOutput, LinkedWords, follow_equal, follow_ratio, link_toggle};
pub use number::{
    NamedOr, NumberField, NumberLook, NumberName, NumberOutput, NumberPick, NumberPrecision, NumberStepper, NumberStyle, NumberValue, RangeAlert,
    StepArrows, UnitPlacement,
};
pub use selectable_text::{SelectableStyle, SelectableText};
pub use text_field::TextField;
pub use word_run::{RunWord, WordRun, WordRunOutput, WordRunStyle, WordState};
pub use words::TextWords;
pub use zoom_field::{InvalidZoom, ZoomField, ZoomFieldOutput, ZoomScale};

use crate::contract::Bind;
use crate::primitives::TextKey;
use crate::style::{ComponentStyle, IconLook, Ink, StyleCx, SurfaceLook};
use craft_behavior::{Insets, Metrics, PartKey};
use craft_theme::{ControlSize, Stateful, States};
use egui::Vec2;

/// The style of a text field: padding and minimum size from the control metrics, and the sizes of the adornments
/// a field draws beside its text (`adorn::place`).
#[derive(Clone, Debug, PartialEq, ComponentStyle)]
#[non_exhaustive]
pub struct FieldStyle {
    #[style(surface)]
    surface: SurfaceLook,
    /// Padding, the gap between adornments and the text, a control high, and `size.length.field` at the control's size
    /// wide where the room allows; a row that lacks room narrows it to `size.length.short`.
    #[style(metrics)]
    metrics: Metrics,
    /// A glyph adornment: its side and colour.
    #[style(icon)]
    glyph: IconLook,
    /// The side of a button adornment: the small control height, whatever the field's size.
    button: f32,
}

/// The space above the first and below the last of the lines of a multi-line text in a field of `metrics`, its lines
/// `line` high: that of a single line in its control, so a box of `n` lines shows them whole.
pub(crate) fn lines_inset(metrics: &Metrics, line: f32) -> f32 {
    ((metrics.min_size.y - line) / 2.0).max(0.0)
}

impl FieldStyle {
    /// The style a text-like field reads from the theme.
    pub fn at(cx: &StyleCx<'_>) -> FieldStyle {
        let control = cx.control();
        let metrics = Metrics::new()
            .padding(Insets::horizontal(control.pad_x))
            .gap(control.gap)
            .min_size(Vec2::new(cx.tokens.size.length.short, control.height))
            .ideal_width(control.field)
            // Where a row shrinks it, a field narrows to its padding and one glyph; what it shows there (an
            // ellipsis, its adornments) its content measure adds.
            .least_width(2.0 * control.pad_x + control.icon);
        let ink = cx.ink(Ink::OnPanel);
        let off = cx.color(cx.tokens.color.text.disabled);
        let glyph = IconLook::new(control.icon, Stateful::new(ink).off(States::new(ink, ink, ink, off)));
        FieldStyle { surface: cx.field(), metrics, glyph, button: cx.control_of(ControlSize::Sm).height }
    }

    /// The style of a field whose `measure` gives its whole width (its widest value or time): the look, padding and
    /// height of a field, with no field length to widen it and no shortest width.
    #[must_use]
    pub fn sized_by_content(mut self) -> FieldStyle {
        self.metrics = self.metrics.min_size(Vec2::new(0.0, self.metrics.min_size.y)).ideal_width(0.0);
        self
    }

    /// The style of a field with no field length to widen it past its shortest width (`size.length.short`): a field
    /// that keeps that width whatever it shows.
    #[must_use]
    pub fn shortest(mut self) -> FieldStyle {
        self.metrics = self.metrics.ideal_width(self.metrics.min_size.x);
        self
    }

    /// The style of a field held at the length `width` (a `size.length` token), whatever value it shows: a ribbon's
    /// font box at the small field length, as wide as a word processor's whichever family it shows. A row that lacks
    /// room still narrows it to `size.length.short`.
    #[must_use]
    pub fn held_at(mut self, width: f32) -> FieldStyle {
        let width = if width.is_finite() { width.max(self.metrics.min_size.x) } else { self.metrics.min_size.x };
        self.metrics = self.metrics.ideal_width(width);
        self
    }

    /// The width of a field that holds `chars` characters: `chars` times the width of a `0` in the body face (CSS's
    /// `ch`), plus the field's padding on both sides. [`held_at`](Self::held_at) takes it as the field's length.
    pub fn chars_width(cx: &StyleCx<'_>, chars: u16) -> f32 {
        let digit = cx.text_width(crate::style::TypeRole::Body, "0");
        let digit = if digit.is_finite() && digit > 0.0 { digit } else { 0.0 };
        f32::from(chars) * digit + 2.0 * cx.control().pad_x
    }

    /// The field's padding, gap and minimum size.
    pub fn metrics(&self) -> Metrics {
        self.metrics
    }

    /// The look of a glyph adornment.
    pub fn glyph(&self) -> &IconLook {
        &self.glyph
    }
}

/// The part key of the one text part.
const TEXT: PartKey = PartKey::new("text");

fn text_part<V>() -> TextKey<V> {
    TextKey::new(TEXT)
}

/// The text field every collection's `rename_editor` and the canvas `Pasteboard`'s page-name rename show over a label:
/// it selects the name on focus, so typing replaces it. It caps nothing: the item's name is the app's, and a rename shows
/// and edits all of it.
pub fn rename_field<'a>(draft: Bind<'a, String>, name: &str) -> TextField<'a> {
    TextField::new(name.to_owned(), draft).select_on_focus().limit(None::<usize>)
}
