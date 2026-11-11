//! The brush Size row ([`BrushSizeField`]): a value field over a log slider.

use std::sync::Arc;

use craft_behavior::{EditEvent, EditEvents, IconId, Mixed};
use craft_ui::buttons::IconButton;
use craft_ui::contract::{Bind, EditResponse, Outcome, Props as _, Show as _, ShowContainer as _, clicked};
use craft_ui::form::{Field, SliderField, SliderFieldLayout};
use craft_ui::runtime::env::{Env, Label, UnitKind};
use craft_ui::value_drag::SliderScale;
use egui::{Response, Ui};

use super::words::BrushSettingsWords;

/// What [`BrushSizeField::show`] reports.
#[derive(Debug)]
#[non_exhaustive]
#[must_use = "the host applies the size edit"]
pub struct BrushSizeOutput {
    /// The row.
    pub response: Response,
    /// The slider field's edit, or the restore button's `Set`; `None` when the body did not run.
    pub edit: Option<EditResponse<f64>>,
}

impl Outcome for BrushSizeOutput {
    type Value = f64;

    fn response(&self) -> &Response {
        &self.response
    }

    #[cfg(feature = "screen-testing")]
    fn story_facts(&self) -> craft_ui::testing::StoryOutput {
        match &self.edit {
            Some(edit) => edit.story_facts(),
            None => craft_ui::testing::StoryOutput::default(),
        }
    }
}

/// The brush Size row of Brush Tip Shape and of the Brush Preset picker's head: a log slider with a number
/// field (px, whole pixels, 1 to the maximum) under the label, and with [`restore`](Self::restore) Photoshop's
/// "Restore original size" icon button before them (sampled tips).
///
/// The field and the slider show one value: a slider drag rounds to whole pixels and stays one gesture while the
/// binding takes the rounded value; a typed value is a `Set`; restore is a `Set` of the original size. Escape
/// during a drag cancels.
#[must_use = "show the field with `show`"]
#[derive(craft_ui::contract::Pattern, craft_ui::contract::Props)]
pub struct BrushSizeField<'a> {
    label: Label,
    diameter: Bind<'a, f64>,
    max: f64,
    /// The size the restore button sets, clamped to 1 to the maximum; without one there is no button.
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
        let BrushSizeField { label, mut diameter, max, restore } = self;
        let words = Env::of(ui).words::<BrushSettingsWords>();
        let max = if max.is_finite() { max.max(1.0) } else { 1.0 };
        let name: Arc<str> = Arc::from(label.as_str());
        let shown = Field::new(label.clone()).show(ui, |ui, cx| {
            let mut restored = None;
            if let Some(original) = restore.filter(|original| original.is_finite()) {
                let button = IconButton::new(IconId::new("undo")).label(words.restore_size.clone()).show(ui);
                if clicked(&button) {
                    let value = original.clamp(1.0, max);
                    let before = match diameter.reborrow().read(cx) {
                        Some(Mixed::Same(held)) => held,
                        _ => value,
                    };
                    let event = EditEvent::set(Mixed::Same(before), value, name.clone(), None);
                    diameter.reborrow().write(cx, &event);
                    restored = Some(event);
                }
            }
            let mut edit = SliderField::new(label.clone(), diameter.reborrowed(), 1.0..=max)
                .scale(SliderScale::Log)
                .step(1.0)
                .unit_kind(UnitKind::Pixels)
                .enabled(max > 1.0)
                .layout(SliderFieldLayout::Inline)
                .show(ui);
            if restored.is_some() {
                edit.events = EditEvents::from(restored);
            }
            edit
        });
        BrushSizeOutput { response: shown.output, edit: shown.inner }
    }
}
