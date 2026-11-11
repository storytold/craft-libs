//! A step that cannot move a value is not offered: a value at the end of its range declares no step past it, and a
//! value that wraps declares both steps at either end.

use craft_ui::choice::Rating;
use craft_ui::contract::{Bind, Props as _, Role, Show as _};
use craft_ui::testing::access::Declared;
use craft_ui::testing::{Fixture, Story, StoryData, StoryOutput, trial};
use craft_ui::value_drag::AngleDial;
use egui::Ui;

fn top_rating(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    Rating::new(Bind::bound(data.value("stars", 5u8)), 5).label("Rating").show(ui).into()
}

fn dial_at_start(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    AngleDial::new(Bind::bound(data.value("angle", 0.0))).range(0.0..=360.0).label("Angle").show(ui).into()
}

/// Whether the one node of `role` declares Increment and Decrement.
fn steps(show: fn(&mut Ui, &mut StoryData) -> StoryOutput, role: Role) -> (bool, bool) {
    let mut fixture = Fixture::story(&Story::new("steps", show));
    fixture.settle();
    let node = fixture.by_role(role).first().expect("the node").node;
    let tree = fixture.tree();
    (tree.declares_key(node, Declared::Increment), tree.declares_key(node, Declared::Decrement))
}

#[trial]
fn a_value_at_its_range_end_offers_no_step_past_it_unless_it_wraps() {
    assert_eq!(steps(top_rating, Role::RADIO_GROUP), (false, true), "a full rating offers Decrement only");
    assert_eq!(steps(dial_at_start, Role::SLIDER), (true, true), "a one-turn dial at its start wraps, so it offers both");
}
