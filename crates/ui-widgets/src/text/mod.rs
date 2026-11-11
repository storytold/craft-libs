//! Text and number fields: `TextField<C>` and `NumberField` on the text part (18.3).

mod adorn;
mod alt_text;
mod chips;
mod completions;
mod hex;
mod linked;
mod number;
mod selectable_text;
mod text_field;
mod words;

pub use adorn::{Adornment, Placed, place, reserve};
pub use alt_text::{AltText, AltTextField, AltTextFieldStyle, AltTextWords};
pub use chips::{ChipCodec, ChipField, ChipFieldStyle, ChipWords};
pub use hex::{HexCodec, HexField};
pub use linked::{LinkMode, LinkedFields, LinkedFieldsStyle, LinkedLayout, LinkedOutput, LinkedWords, follow_equal, follow_ratio, link_toggle};
pub use number::{NamedOr, NumberField, NumberName, NumberOutput, NumberPick, NumberStepper, NumberStyle, NumberValue};
pub use selectable_text::{SelectableStyle, SelectableText};
pub use text_field::TextField;

use crate::contract::{Bind, EditResponse};
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
    /// Padding, the gap between adornments and the text, a control high, and `size.length.field` wide where the room
    /// allows; a row that lacks room narrows it to `size.length.short`.
    #[style(metrics)]
    metrics: Metrics,
    /// A glyph adornment: its side and colour.
    #[style(icon)]
    glyph: IconLook,
    /// The side of a button adornment: the small control height, whatever the field's size.
    button: f32,
}

impl FieldStyle {
    /// The style a text-like field reads from the theme.
    pub fn at(cx: &StyleCx<'_>) -> FieldStyle {
        let control = cx.control();
        let metrics = Metrics::new()
            .padding(Insets::horizontal(control.pad_x))
            .gap(control.gap)
            .min_size(Vec2::new(cx.tokens.size.length.short, control.height))
            .ideal_width(cx.tokens.size.length.field);
        let ink = cx.ink(Ink::OnPanel);
        let off = cx.color(cx.tokens.color.text.disabled);
        let glyph = IconLook::new(control.icon, Stateful::new(ink).off(States::new(ink, ink, ink, off)));
        FieldStyle { surface: cx.field(), metrics, glyph, button: cx.control_of(ControlSize::Sm).height }
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

/// The text field every collection's `rename_editor` shows over an item's label: it selects the name on focus, so
/// typing replaces it.
pub(crate) fn rename_field(ui: &mut egui::Ui, draft: Bind<'_, String>, label: egui::Rect, name: &str) -> EditResponse<String> {
    use crate::contract::Show as _;
    TextField::new(name.to_owned(), draft).select_on_focus().show_at(ui, label)
}
