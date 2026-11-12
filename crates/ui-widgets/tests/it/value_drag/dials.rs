//! `AngleDial` points at the pointer in its orientation and steps round or against the ends of its range;
//! `PointPicker` sets the point under a press in the press pass, moves one axis per arrow key, pages y and takes x to
//! its edges.

use craft_behavior::Mixed;
use craft_ui::contract::{Bind, Props as _, Show as _};
use craft_ui::testing::{EditKind, Fixture, Story, StoryData, StoryOutput, trial};
use craft_ui::value_drag::{AngleDial, PointPicker};
use egui::{Key, Modifiers, Ui, Vec2};

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

fn point(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let start = [Mixed::Same(50.0), Mixed::Same(50.0)];
    PointPicker::new(Bind::bound(data.value("point", start)), 0.0..=100.0, 0.0..=100.0).step(1.0).label("Point").show(ui).into()
}

fn edits(fixture: &mut Fixture) -> Vec<EditKind> {
    fixture.take_outputs().into_iter().flat_map(|output| output.edits).collect()
}

/// An absolute value drag (`Cx::follow_press`): a press held still puts the point under the pointer in the press
/// pass, the release commits that gesture as one edit, and Escape before the release goes back to the start.
#[trial]
fn a_press_on_the_well_sets_the_point_before_any_drag_and_the_release_commits_it() {
    let mut fixture = shown(point);
    let well = fixture.by_label("Point").expect("the picker").rect;
    let quarter = well.min + well.size() * 0.25;
    edits(&mut fixture);
    fixture.drag_hold_at(quarter, Vec2::ZERO);
    let held = edits(&mut fixture);
    assert!(held.contains(&EditKind::Begin) && held.contains(&EditKind::Update), "the press proposes the point: {held:?}");
    assert!(!held.contains(&EditKind::Commit), "the press held still commits nothing yet");
    fixture.release_at(quarter);
    assert_eq!(edits(&mut fixture), vec![EditKind::Commit]);
    assert_eq!(*fixture.data().value("point", [Mixed::Mixed; 2]), [Mixed::Same(25.0), Mixed::Same(25.0)]);

    fixture.drag_hold_at(well.max - well.size() * 0.25, Vec2::ZERO);
    fixture.key(Key::Escape, Modifiers::NONE);
    fixture.release_at(well.max - well.size() * 0.25);
    assert_eq!(*fixture.data().value("point", [Mixed::Mixed; 2]), [Mixed::Same(25.0), Mixed::Same(25.0)], "Escape restores the start");
}

/// Page Up and Page Down move the point a tenth of the height and back; End and Home take it to the right and left
/// edges, leaving y where it is.
#[trial]
fn page_keys_move_the_point_a_tenth_up_and_down_and_home_and_end_reach_the_side_edges() {
    let mut fixture = focused(point);
    let at = |fixture: &Fixture| *fixture.data().value("point", [Mixed::Mixed; 2]);
    let start = at(&fixture);
    key(&mut fixture, Key::PageUp, Modifiers::NONE);
    assert_eq!(at(&fixture), [start[0], Mixed::Same(40.0)], "y grows downwards, so up is a tenth of 100 less");
    key(&mut fixture, Key::PageDown, Modifiers::NONE);
    assert_eq!(at(&fixture), start);
    key(&mut fixture, Key::End, Modifiers::NONE);
    assert_eq!(at(&fixture), [Mixed::Same(100.0), start[1]]);
    key(&mut fixture, Key::Home, Modifiers::NONE);
    assert_eq!(at(&fixture), [Mixed::Same(0.0), start[1]]);
}
