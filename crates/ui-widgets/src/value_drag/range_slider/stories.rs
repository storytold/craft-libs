//! Stories of `RangeSlider`.

use super::{LOWER, RangeSlider};
use crate::contract::{Bind, Props as _, Show as _, Validity};
use crate::display::Progress;
use crate::runtime::env::Message;
use crate::testing::{Drive, Grip, StoryData, StoryOutput, story};
use crate::value_drag::TrackFace;
use craft_behavior::Mixed;
use craft_theme::ControlSize;
use egui::Ui;

const RANGE: std::ops::RangeInclusive<f64> = 0.0..=100.0;
const CAP: f64 = 80.0;
const LEVEL: (Mixed<f64>, Mixed<f64>) = (Mixed::Same(20.0), Mixed::Same(60.0));

#[story(RangeSlider<'static>, covers = [Default], drive = Drive::Drag(Grip::new(LOWER)))]
fn range_slider(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let size = *data.choice("size", ControlSize::Md, &[("Sm", ControlSize::Sm), ("Md", ControlSize::Md)]);
    let disabled = *data.value("disabled", false);
    RangeSlider::new(Bind::bound(data.value("range", LEVEL)), RANGE).label("Level").size(size).enabled(!disabled).show(ui).into()
}

#[story(RangeSlider<'static>, covers = [Mixed], drive = Drive::Never)]
fn range_slider_mixed(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    RangeSlider::new(Bind::bound_mixed(data.value("mixed", Mixed::Mixed)), RANGE).label("Level").show(ui).into()
}

#[story(RangeSlider<'static>, covers = [], drive = Drive::Never)]
fn range_slider_upper_mixed(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    RangeSlider::new(Bind::bound(data.value("upper-mixed", (Mixed::Same(20.0), Mixed::Mixed))), RANGE).label("Level").show(ui).into()
}

#[story(RangeSlider<'static>, covers = [Invalid], drive = Drive::Never)]
fn range_slider_invalid(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let refuse_over = |range: &(Mixed<f64>, Mixed<f64>)| {
        if range.1.same().is_some_and(|upper| *upper > CAP) { Validity::Refuse(Message::new("Above the limit")) } else { Validity::Ok }
    };
    RangeSlider::new(Bind::bound(data.value("over", (Mixed::Same(30.0), Mixed::Same(95.0)))).check(refuse_over), RANGE).label("Level").show(ui).into()
}

#[story(RangeSlider<'static>, covers = [], drive = Drive::Never)]
fn range_slider_labelled_marks(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let marks = [(0.0, "Dark"), (50.0, "Mid"), (100.0, "Light")];
    RangeSlider::new(Bind::bound(data.value("range", LEVEL)), RANGE).marks_labelled(marks).label("Level").show(ui).into()
}

#[story(RangeSlider<'static>, covers = [], drive = Drive::Never)]
fn range_slider_ticks(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    RangeSlider::new(Bind::bound(data.value("range", LEVEL)), RANGE).ticks(10).label("Level").show(ui).into()
}

#[story(RangeSlider<'static>, covers = [], drive = Drive::Never)]
fn range_slider_track_face(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    RangeSlider::new(Bind::bound(data.value("range", LEVEL)), RANGE)
        .track_face(TrackFace::new(Progress::new(1.0).label("Track")))
        .label("Hue")
        .show(ui)
        .into()
}
