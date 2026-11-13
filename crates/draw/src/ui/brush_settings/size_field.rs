//! The brush Size row ([`BrushSizeField`]): a number field over a log slider.

use craft_ui::contract::{Bind, EditResponse, Outcome, Props as _, Show as _};
use craft_ui::form::SliderField;
use craft_ui::runtime::env::{Label, UnitKind};
use craft_ui::value_drag::SliderScale;
use egui::{Response, Ui};

/// What [`BrushSizeField::show`] reports.
#[derive(Debug)]
#[non_exhaustive]
#[must_use = "the host applies the size edit"]
pub struct BrushSizeOutput {
    /// The row.
    pub response: Response,
    /// The slider field's edit.
    pub edit: EditResponse<f64>,
}

impl Outcome for BrushSizeOutput {
    type Value = f64;

    fn response(&self) -> &Response {
        &self.response
    }

    #[cfg(feature = "screen-testing")]
    fn story_facts(&self) -> craft_ui::testing::StoryOutput {
        self.edit.story_facts()
    }
}

/// The brush Size row of Brush Tip Shape and of the Brush Preset picker's head: the label and a number field (px,
/// whole pixels, 1 to the maximum) on one line, a log slider below.
///
/// The field and the slider show one value: a slider drag rounds to whole pixels and stays one gesture while the
/// binding takes the rounded value; a typed value is a `Set`. Escape during a drag cancels. With
/// [`restore`](Self::restore) (a sampled tip's original size) the reset gesture of the label, the field and the
/// slider sets the original size.
#[must_use = "show the field with `show`"]
#[derive(craft_ui::contract::Pattern, craft_ui::contract::Props)]
pub struct BrushSizeField<'a> {
    label: Label,
    diameter: Bind<'a, f64>,
    max: f64,
    /// The size a reset sets, clamped to 1 to the maximum; read each pass, so it follows the tip the host shows.
    #[prop(quantity = "pixels")]
    restore: Option<f64>,
}

impl<'a> BrushSizeField<'a> {
    /// A Size row named `label` over `diameter` pixels, 1 to `max` (the app's engine limit).
    pub fn new(label: impl Into<Label>, diameter: Bind<'a, f64>, max: f64) -> Self {
        Self { label: label.into(), diameter, max, restore: None }
    }

    /// Shows the row and reports the new diameter.
    pub fn show(self, ui: &mut Ui) -> BrushSizeOutput {
        let BrushSizeField { label, diameter, max, restore } = self;
        let max = if max.is_finite() { max.max(1.0) } else { 1.0 };
        let diameter = match restore.filter(|original| original.is_finite()) {
            Some(original) => diameter.reset(original.clamp(1.0, max)),
            None => diameter,
        };
        let edit =
            SliderField::new(label, diameter, 1.0..=max).scale(SliderScale::Log).step(1.0).unit_kind(UnitKind::Pixels).enabled(max > 1.0).show(ui);
        BrushSizeOutput { response: edit.response.clone(), edit }
    }
}
