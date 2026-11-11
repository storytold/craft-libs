//! Stories of `SliderField`.

use super::SliderField;
use crate::contract::{Bind, Props as _, Show as _, Validity};
use crate::runtime::env::{Message, UnitKind};
use crate::testing::{Drive, StoryData, StoryOutput, story};
use crate::value_drag::SliderScale;
use craft_behavior::Mixed;
use egui::Ui;

const PERCENT: std::ops::RangeInclusive<f64> = 0.0..=100.0;
const CAP: f64 = 80.0;

/// The caption and number field over the slider.
#[story(SliderField<'static>, covers = [Default], drive = Drive::Type("25"))]
fn slider_field(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    SliderField::new("Opacity", Bind::bound(data.value("opacity", 40.0)), PERCENT).unit_kind(UnitKind::Percent).show(ui).into()
}

#[story(SliderField<'static>, covers = [], drive = Drive::Never)]
fn slider_field_log(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    SliderField::new("Brush size", Bind::bound(data.value("size", 8.0)), 1.0..=200.0).scale(SliderScale::Log).step(1.0).show(ui).into()
}

#[story(SliderField<'static>, covers = [Mixed], drive = Drive::Never)]
fn slider_field_mixed(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    SliderField::new("Opacity", Bind::bound_mixed(data.value("mixed", Mixed::Mixed)), PERCENT).show(ui).into()
}

#[story(SliderField<'static>, covers = [Invalid], drive = Drive::Never)]
fn slider_field_invalid(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let refuse_over = |value: &f64| if *value > CAP { Validity::Refuse(Message::new("Above the limit")) } else { Validity::Ok };
    SliderField::new("Opacity", Bind::bound(data.value("over", 95.0)).check(refuse_over), PERCENT).show(ui).into()
}

#[story(SliderField<'static>, covers = [], drive = Drive::Type("25"), disabled_text = "Pick a layer first")]
fn slider_field_with_reason(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let tooltip = data.tooltip("The layer's opacity");
    SliderField::new("Opacity", Bind::bound(data.value("value", 40.0)), PERCENT).tooltip(tooltip).show(ui).into()
}
