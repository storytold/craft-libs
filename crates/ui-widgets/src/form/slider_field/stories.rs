//! Stories of `SliderField`.

use super::SliderField;
use crate::contract::{Props as _, Show as _, Validity};
use crate::runtime::env::{Message, UnitKind};
use crate::testing::{Drive, StoryData, StoryOutput, story};
use crate::value_drag::{SliderScale, TrackPress};
use craft_behavior::Mixed;
use egui::Ui;

const PERCENT: std::ops::RangeInclusive<f64> = 0.0..=100.0;
const CAP: f64 = 80.0;

/// The caption and number field over the slider.
#[story(SliderField<'static>, covers = [Default], drive = Drive::Type("25"))]
fn slider_field(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    SliderField::new("Opacity", data.bind("opacity", 40.0), PERCENT).unit_kind(UnitKind::Percent).show(ui).into()
}

/// Words at the two ends of the track, as a softness or a contrast control names its extremes.
#[story(SliderField<'static>, covers = [], drive = Drive::Never)]
fn slider_field_end_words(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    SliderField::new("Softness", data.bind("softness", 50.0), PERCENT).marks_labelled([(0.0, "Soft"), (100.0, "Hard")]).show(ui).into()
}

#[story(SliderField<'static>, covers = [], drive = Drive::Never)]
fn slider_field_log(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    SliderField::new("Brush size", data.bind("size", 8.0), 1.0..=200.0).scale(SliderScale::Log).step(1.0).show(ui).into()
}

/// A whole-step slider whose field keeps finer typed values.
#[story(SliderField<'static>, covers = [], drive = Drive::Never)]
fn slider_field_typed_precision(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    SliderField::new("Size", data.bind("precise_size", 36.1231), 1.0..=100.0).step(1.0).typed_decimals(4).show(ui).into()
}

/// A press on the track sets the value as the button goes down (`TrackPress::OnPress`): a brush setting whose
/// preview should answer the first touch.
#[story(SliderField<'static>, covers = [], drive = Drive::Never)]
fn slider_field_track_on_press(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    SliderField::new("Flow", data.bind("flow", 60.0), PERCENT).unit_kind(UnitKind::Percent).track_press(TrackPress::OnPress).show(ui).into()
}

#[story(SliderField<'static>, covers = [Mixed], drive = Drive::Never)]
fn slider_field_mixed(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    SliderField::new("Opacity", data.bind_mixed("mixed", Mixed::Mixed), PERCENT).show(ui).into()
}

#[story(SliderField<'static>, covers = [Invalid], drive = Drive::Never)]
fn slider_field_invalid(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let refuse_over = |value: &f64| if *value > CAP { Validity::Refuse(Message::new("Above the limit")) } else { Validity::Ok };
    SliderField::new("Opacity", data.bind("over", 95.0).check(refuse_over), PERCENT).show(ui).into()
}

#[story(SliderField<'static>, covers = [], drive = Drive::Type("25"), disabled_text = "Pick a layer first")]
fn slider_field_with_reason(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let tooltip = data.tooltip("The layer's opacity");
    SliderField::new("Opacity", data.bind("value", 40.0), PERCENT).tooltip(tooltip).show(ui).into()
}
