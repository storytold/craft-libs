//! Stories of `PointPicker`.

use super::PointPicker;
use crate::contract::{Bind, Props as _, Show as _, Validity};
use crate::runtime::env::Message;
use crate::testing::{Drive, Grip, StoryData, StoryOutput, story};
use craft_behavior::Mixed;
use egui::Ui;

const RANGE: std::ops::RangeInclusive<f64> = 0.0..=100.0;
const CENTER: [f64; 2] = [50.0, 50.0];
const RIGHT: [f64; 2] = [90.0, 50.0];
const CAP: f64 = 80.0;

#[story(PointPicker<'static>, covers = [Default], drive = Drive::Step)]
fn point_picker(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    PointPicker::new(Bind::bound(data.value("point", Mixed::each(CENTER))), RANGE, RANGE).label("Blur center").show(ui).into()
}

#[story(PointPicker<'static>, covers = [], drive = Drive::Drag(Grip::new(PointPicker::WELL)))]
fn point_picker_drag(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    PointPicker::new(Bind::bound(data.value("point", Mixed::each(CENTER))), RANGE, RANGE).label("Blur center").show(ui).into()
}

#[story(PointPicker<'static>, covers = [Mixed], drive = Drive::Never)]
fn point_picker_mixed(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    PointPicker::new(Bind::bound_mixed(data.value("mixed", Mixed::Mixed)), RANGE, RANGE).label("Blur center").show(ui).into()
}

#[story(PointPicker<'static>, covers = [Invalid], drive = Drive::Never)]
fn point_picker_invalid(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let refuse_right = |point: &[Mixed<f64>; 2]| match point[0].same() {
        Some(x) if *x > CAP => Validity::Refuse(Message::new("Too far right")),
        _ => Validity::Ok,
    };
    PointPicker::new(Bind::bound(data.value("over", Mixed::each(RIGHT))).check(refuse_right), RANGE, RANGE).label("Blur center").show(ui).into()
}

#[story(PointPicker<'static>, covers = [], drive = Drive::Never)]
fn point_picker_mixed_axis(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let value = [Mixed::Same(CENTER[0]), Mixed::Mixed];
    PointPicker::new(Bind::bound(data.value("axis", value)), RANGE, RANGE).label("Blur center").show(ui).into()
}

#[story(PointPicker<'static>, covers = [], drive = Drive::Never)]
fn point_picker_y_up(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    PointPicker::new(Bind::bound(data.value("point", Mixed::each(CENTER))), RANGE, 100.0..=0.0).label("Blur center").show(ui).into()
}
