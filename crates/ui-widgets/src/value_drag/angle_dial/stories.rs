//! Stories of `AngleDial`.

use super::{AngleDial, AngleOrientation, FACE};
use crate::contract::{Bind, Props as _, Show as _, Validity};
use crate::runtime::env::Message;
use crate::testing::{Drive, Grip, StoryData, StoryOutput, story};
use craft_behavior::Mixed;
use craft_theme::ControlSize;
use egui::Ui;

const ANGLE: f64 = 120.0;
const CAP: f64 = 90.0;

#[story(AngleDial<'static>, covers = [Default], drive = Drive::Step)]
fn angle_dial(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let ranges = [("-180 to 180", -180.0..=180.0), ("-360 to 360", -360.0..=360.0), ("0 to 90", 0.0..=90.0)];
    let range = data.choice("range", -180.0..=180.0, &ranges).clone();
    let orientation = *data.choice(
        "orientation",
        AngleOrientation::CounterClockwise,
        &[("counter-clockwise", AngleOrientation::CounterClockwise), ("Clockwise", AngleOrientation::Clockwise)],
    );
    let size = *data.choice("size", ControlSize::Md, &[("Sm", ControlSize::Sm), ("Md", ControlSize::Md)]);
    let disabled = *data.value("disabled", false);
    AngleDial::new(Bind::bound(data.ranged("angle", ANGLE, 0.0..=360.0)))
        .label("Angle")
        .range(range)
        .orientation(orientation)
        .size(size)
        .enabled(!disabled)
        .show(ui)
        .into()
}

#[story(AngleDial<'static>, covers = [], drive = Drive::Drag(Grip::new(FACE)))]
fn angle_dial_drag(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    AngleDial::new(Bind::bound(data.value("angle", ANGLE))).label("Angle").show(ui).into()
}

#[story(AngleDial<'static>, covers = [Mixed], drive = Drive::Never)]
fn angle_dial_mixed(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    AngleDial::new(Bind::bound_mixed(data.value("mixed", Mixed::Mixed))).label("Angle").show(ui).into()
}

#[story(AngleDial<'static>, covers = [Invalid], drive = Drive::Never)]
fn angle_dial_invalid(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let refuse_over = |angle: &f64| if *angle > CAP { Validity::Refuse(Message::new("Above the limit")) } else { Validity::Ok };
    AngleDial::new(Bind::bound(data.value("over", ANGLE)).check(refuse_over)).label("Angle").show(ui).into()
}
