//! The axes behind one button: a `Button` that opens the axis sliders in a popover.

use craft_ui::contract::{Bind, Pattern, Props};
use craft_ui::runtime::env::Label;
use egui::Ui;

use super::{AxisSlidersOutput, AxisValues, FontAxis, NamedInstance};

/// A `Button` with the label that opens an anchored `Popover` holding `AxisSliders` over the same props. The open
/// state is the button's own memory. The output is the sliders' (`edit`, `instance`), with the button's response.
#[must_use = "show the button with `show`"]
#[derive(Pattern, Props)]
#[allow(dead_code)] // P3(type-w3): ty.X21
pub struct AxesButton<'a> {
    label: Label,
    axes: &'a [FontAxis],
    values: Bind<'a, AxisValues>,
    /// The named instances of the face, listed above the sliders.
    #[prop]
    instances: &'a [NamedInstance],
    /// The style's weight, which an unset `wght` axis shows (an unset axis shows its default without it).
    #[prop(quantity = "font weight, the `wght` coordinate")]
    weight: Option<f32>,
    /// Whether the style is italic: an unset `ital` axis then shows its maximum, once a weight is given.
    #[prop]
    italic: bool,
    /// The axes (by tag) the selection mixes: each shows `Mixed` and is left out of the edit's held values.
    #[prop]
    #[allow(dead_code)] // P3(type-w3): ty.X21
    mixed: &'a [[u8; 4]],
}

impl<'a> AxesButton<'a> {
    /// A button named `label` over the `axes` of a face, editing the coordinates `values`.
    pub fn new(label: impl Into<Label>, axes: &'a [FontAxis], values: Bind<'a, AxisValues>) -> Self {
        AxesButton { label: label.into(), axes, values, instances: &[], weight: None, italic: false, mixed: &[] }
    }

    /// Draws the button and, while it is open, the sliders; reports the sliders' edit made this pass.
    pub fn show(self, ui: &mut Ui) -> AxisSlidersOutput {
        let _ = ui;
        todo!("ty.X21")
    }
}
