//! Stories of `Slider`, in the old gallery's groups: Values, Step, Disabled and Right to left as form rows, then the
//! groups the old gallery did not have.

use super::{Slider, SliderScale, SliderThumb};
use crate::contract::{Bind, Props as _, Show as _, ShowContainer as _, Validity};
use crate::display::Progress;
use crate::form::Field;
use crate::runtime::env::Message;
use crate::testing::{Drive, Grip, StoryData, StoryOutput, story};
use crate::value_drag::TrackFace;
use craft_behavior::Mixed;
use craft_theme::ControlSize;
use egui::Ui;

const RANGE: std::ops::RangeInclusive<f64> = 0.0..=100.0;
const CAP: f64 = 80.0;

/// A slider of the "Values" group: `label` at `start` over 0 to 100.
fn value_row(ui: &mut Ui, data: &mut StoryData, label: &str, start: f64) -> StoryOutput {
    Slider::new(Bind::bound(data.ranged("value", start, RANGE)), RANGE).label(label.to_owned()).show(ui).into()
}

#[story(Slider<'static>, covers = [], drive = Drive::Never, group = "Values", title = "Minimum")]
fn slider_minimum(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    value_row(ui, data, "Minimum", 0.0)
}

#[story(Slider<'static>, covers = [], drive = Drive::Drag(Grip::new(Slider::THUMB)), group = "Values", title = "Middle")]
fn slider_middle(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    value_row(ui, data, "Middle", 50.0)
}

#[story(Slider<'static>, covers = [], drive = Drive::Never, group = "Values", title = "Maximum")]
fn slider_maximum(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    value_row(ui, data, "Maximum", 100.0)
}

#[story(Slider<'static>, covers = [], drive = Drive::Never, group = "Values", title = "Power 2")]
fn slider_power(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    Slider::new(Bind::bound(data.ranged("value", 25.0, RANGE)), RANGE).scale(SliderScale::Power(2.0)).label("Power 2").show(ui).into()
}

#[story(Slider<'static>, covers = [], drive = Drive::Step, group = "Values", title = "Log")]
fn slider_log(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    const LOG: std::ops::RangeInclusive<f64> = 1.0..=1000.0;
    Slider::new(Bind::bound(data.ranged("value", 100.0, LOG)), LOG).scale(SliderScale::Log).label("Log").show(ui).into()
}

#[story(Slider<'static>, covers = [], drive = Drive::Never, group = "Values", title = "Marks")]
fn slider_marks(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    Slider::new(Bind::bound(data.ranged("value", 50.0, RANGE)), RANGE).marks(&[0.0, 25.0, 50.0, 100.0]).label("Marks").show(ui).into()
}

#[story(Slider<'static>, covers = [], drive = Drive::Never, group = "Values", title = "Triangle, ticks")]
fn slider_triangle_ticks(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    Slider::new(Bind::bound(data.ranged("value", 30.0, RANGE)), RANGE).thumb(SliderThumb::Triangle).ticks(10).label("Triangle").show(ui).into()
}

/// A slider of the "Step" group: steps of `step` from `start`.
fn step_row(ui: &mut Ui, data: &mut StoryData, step: f64, start: f64) -> StoryOutput {
    Slider::new(Bind::bound(data.ranged("value", start, RANGE)), RANGE).step(step).label(format!("Step {step}")).show(ui).into()
}

#[story(Slider<'static>, covers = [], drive = Drive::Step, group = "Step", title = "Step 10")]
fn slider_step_10(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    step_row(ui, data, 10.0, 40.0)
}

#[story(Slider<'static>, covers = [], drive = Drive::Step, group = "Step", title = "Step 25")]
fn slider_step_25(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    step_row(ui, data, 25.0, 50.0)
}

/// A disabled slider of the "Disabled" group.
fn disabled_row(ui: &mut Ui, data: &mut StoryData, label: &str, start: f64) -> StoryOutput {
    Slider::new(Bind::bound(data.ranged("value", start, RANGE)), RANGE).label(label.to_owned()).enabled(false).show(ui).into()
}

#[story(Slider<'static>, covers = [], drive = Drive::Never, group = "Disabled", title = "Disabled 1")]
fn slider_disabled_1(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    disabled_row(ui, data, "Disabled 1", 0.0)
}

#[story(Slider<'static>, covers = [], drive = Drive::Never, group = "Disabled", title = "Disabled 2")]
fn slider_disabled_2(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    disabled_row(ui, data, "Disabled 2", 30.0)
}

#[story(Slider<'static>, covers = [], drive = Drive::Never, group = "Disabled", title = "Disabled 3")]
fn slider_disabled_3(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    disabled_row(ui, data, "Disabled 3", 100.0)
}

/// With the direction set to right to left the label moves to the right; the slider keeps its geometry.
#[story(Slider<'static>, covers = [], drive = Drive::Step, group = "Right to left", title = "Opacity")]
fn slider_right_to_left(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    Slider::new(Bind::bound(data.ranged("value", 25.0, RANGE)), RANGE).step(1.0).label("Opacity").show(ui).into()
}

#[story(Slider<'static>, covers = [Default], drive = Drive::Step, group = "Playground", title = "Opacity")]
fn slider(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let size = *data.choice("size", ControlSize::Md, &[("Sm", ControlSize::Sm), ("Md", ControlSize::Md)]);
    let disabled = *data.value("disabled", false);
    let unfilled = *data.value("unfilled_track", false);
    Slider::new(Bind::bound(data.ranged("value", 40.0, RANGE)), RANGE).label("Opacity").size(size).enabled(!disabled).fill(!unfilled).show(ui).into()
}

#[story(Slider<'static>, covers = [Mixed], drive = Drive::Never, group = "States", title = "Mixed")]
fn slider_mixed(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    Slider::new(Bind::bound_mixed(data.value("mixed", Mixed::Mixed)), RANGE).label("Opacity").show(ui).into()
}

#[story(Slider<'static>, covers = [Invalid], drive = Drive::Never, group = "States", title = "Above the limit")]
fn slider_invalid(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let refuse_over = |value: &f64| if *value > CAP { Validity::Refuse(Message::new("Above the limit")) } else { Validity::Ok };
    Slider::new(Bind::bound(data.value("over", 95.0)).check(refuse_over), RANGE).label("Opacity").show(ui).into()
}

#[story(Slider<'static>, covers = [], drive = Drive::Never, group = "Looks", title = "Labelled marks")]
fn slider_labelled_marks(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let marks = [(0.0, "Off"), (50.0, "Half"), (100.0, "Full")];
    Slider::new(Bind::bound(data.value("value", 50.0)), RANGE).marks_labelled(marks).label("Opacity").show(ui).into()
}

#[story(Slider<'static>, covers = [], drive = Drive::Never, group = "Looks", title = "Unfilled")]
fn slider_unfilled(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    Slider::new(Bind::bound(data.value("value", 40.0)), RANGE).fill(false).label("Opacity").show(ui).into()
}

#[story(Slider<'static>, covers = [], drive = Drive::Never, group = "Looks", title = "Value in words")]
fn slider_value_text(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    Slider::new(Bind::bound(data.value("value", 40.0)), RANGE).value_text("40 percent").label("Opacity").show(ui).into()
}

#[story(Slider<'static>, covers = [], drive = Drive::Never, group = "Looks", title = "Track face")]
fn slider_track_face(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    Slider::new(Bind::bound(data.value("value", 40.0)), RANGE)
        .track_face(TrackFace::new(Progress::new(1.0).label("Track")))
        .label("Hue")
        .show(ui)
        .into()
}

/// A slider shown in a field: the story's subject is the slider, and its value changes through its own thumb.
#[story(Slider<'static>, covers = [], drive = Drive::Drag(Grip::new(Slider::THUMB)), title = "In a field")]
fn slider_in_a_field(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let shown = Field::new("Opacity").show(ui, |ui, _| Slider::new(Bind::bound(data.value("value", 40.0)), 0.0..=100.0).label("Opacity").show(ui));
    shown.inner.map(StoryOutput::from).unwrap_or_default()
}
