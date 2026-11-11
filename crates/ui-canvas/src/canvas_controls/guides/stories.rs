//! Stories of `Guides`.

use super::{GUIDE, Guides};
use crate::canvas::testing::canvas_story::on_canvas;
use crate::canvas::{Guide, GuideDrag, GuideKey, GuideLine};
use crate::contract::{Bind, Show as _, Validity};
use crate::runtime::env::Message;
use crate::testing::{Drive, Fixture, StoryData, StoryOutput, story};
use craft_behavior::kurbo::Rect as DocRect;
use craft_behavior::{Axis, GesturePhase, Mixed};
use craft_keymap::Mods;
use egui::{Ui, Vec2};

/// The most guides the invalid story accepts.
const MOST_GUIDES: usize = 2;

fn guides() -> Vec<Guide> {
    let mut dashed = Guide::new(GuideKey::new(3), Axis::X, 280.0);
    dashed.line = GuideLine::Dashed;
    let mut locked = Guide::new(GuideKey::new(4), Axis::Y, 480.0);
    locked.locked = true;
    vec![Guide::new(GuideKey::new(1), Axis::Y, 160.0), Guide::new(GuideKey::new(2), Axis::X, 120.0), dashed, locked]
}

/// Drags the first guide across itself by twice the drag threshold.
fn drag_first(fixture: &mut Fixture) {
    let Some(id) = fixture.root_id() else { return };
    let Some(line) = fixture.part(id, GuideKey::new(1).part(GUIDE)) else { return };
    let travel = 2.0 * crate::testing::input::max_click_dist(fixture.ctx());
    fixture.drag(&line, Vec2::new(travel, 0.0));
}

#[story(Guides<'static>, covers = [Default], drive = Drive::Custom(drag_first))]
fn guides_over_a_canvas(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let guides = data.value("guides", guides());
    on_canvas(ui, |ui, at, _| Guides::new("Guides", Bind::bound(&mut *guides)).interactive(true).selected_guide(GuideKey::new(2)).show_at(ui, at))
}

#[story(Guides<'static>, covers = [Mixed], drive = Drive::Custom(drag_first))]
fn guides_that_differ(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let guides = data.value("guides", Mixed::<Vec<Guide>>::Mixed);
    on_canvas(ui, |ui, at, _| Guides::new("Guides", Bind::bound_mixed(&mut *guides)).interactive(true).show_at(ui, at))
}

#[story(Guides<'static>, covers = [Invalid], drive = Drive::Custom(drag_first))]
fn too_many_guides(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let guides = data.value("guides", guides());
    let few = |guides: &Vec<Guide>| {
        if guides.len() > MOST_GUIDES { Validity::Refuse(Message::new("There are too many guides")) } else { Validity::Ok }
    };
    on_canvas(ui, |ui, at, _| Guides::new("Guides", Bind::bound(&mut *guides).check(few)).interactive(true).show_at(ui, at))
}

#[story(Guides<'static>, covers = [Default], drive = Drive::Custom(drag_first))]
fn guides_that_only_draw(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let guides = data.value("guides", guides());
    on_canvas(ui, |ui, at, _| Guides::new("Guides", Bind::bound(&mut *guides)).show_at(ui, at))
}

#[story(Guides<'static>, covers = [Default], drive = Drive::Custom(drag_first))]
fn guide_dragged_out_of_a_ruler(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let guides = data.value("guides", guides());
    let drag = GuideDrag::new_guide(Axis::Y, GesturePhase::Update, Some(200.0), Mods::NONE);
    on_canvas(ui, |ui, at, _| Guides::new("Guides", Bind::bound(&mut *guides)).interactive(true).pending(drag).show_at(ui, at))
}

#[story(Guides<'static>, covers = [Default], drive = Drive::Custom(drag_first))]
fn bounded_guides(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let guides = data.value("guides", guides());
    let page = DocRect::new(120.0, 80.0, 520.0, 320.0);
    on_canvas(ui, |ui, at, _| Guides::new("Guides", Bind::bound(&mut *guides)).interactive(true).bounds(page).show_at(ui, at))
}
