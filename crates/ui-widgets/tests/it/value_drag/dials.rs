//! `AngleDial` points at the pointer in its orientation and steps round or against the ends of its range;
//! `PointPicker` maps a press to the point on the well and moves one axis per arrow key.

use craft_ui::contract::{Bind, Props as _, Show as _};
use craft_ui::testing::{Fixture, Story, StoryData, StoryOutput, trial};
use craft_ui::value_drag::AngleDial;
use egui::{Key, Modifiers, Ui};

fn at_turn_end(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    AngleDial::new(Bind::bound(data.value("angle", 180.0))).label("Angle").show(ui).into()
}

fn quarter_end(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    AngleDial::new(Bind::bound(data.value("angle", 90.0))).range(0.0..=90.0).label("Angle").show(ui).into()
}

fn quarter_end_wrapping(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    AngleDial::new(Bind::bound(data.value("angle", 90.0))).range(0.0..=90.0).wrap(true).label("Angle").show(ui).into()
}

fn shown(show: fn(&mut Ui, &mut StoryData) -> StoryOutput) -> Fixture {
    let mut fixture = Fixture::story(&Story::new("dials", show));
    fixture.settle();
    fixture
}

fn angle(fixture: &Fixture) -> f64 {
    *fixture.data().value("angle", f64::NAN)
}

fn key(fixture: &mut Fixture, key: Key, modifiers: Modifiers) {
    fixture.key(key, modifiers);
    fixture.idle(2);
}

fn focused(show: fn(&mut Ui, &mut StoryData) -> StoryOutput) -> Fixture {
    let mut fixture = shown(show);
    key(&mut fixture, Key::Tab, Modifiers::NONE);
    fixture
}

#[trial]
fn a_dial_wraps_round_a_turn_and_stops_at_the_ends_of_a_narrower_range() {
    let mut turn = focused(at_turn_end);
    key(&mut turn, Key::ArrowRight, Modifiers::NONE);
    assert_eq!(angle(&turn), -179.0, "one step past 180 continues from -180 by the overshoot");

    let mut stops = focused(quarter_end);
    key(&mut stops, Key::ArrowRight, Modifiers::NONE);
    assert_eq!(angle(&stops), 90.0);

    let mut wraps = focused(quarter_end_wrapping);
    key(&mut wraps, Key::ArrowRight, Modifiers::NONE);
    assert_eq!(angle(&wraps), 1.0, "a range that wraps by choice continues from its start");
}
