//! Stories of `Guides`.

use super::{GUIDE, Guides, GuidesOutput};
use crate::canvas::testing::canvas_story::{StoryDocument, on_canvas_with, story_view};
use crate::canvas::{Canvas, Guide, GuideDrag, GuideKey, GuideLine, RasterSource};
use crate::contract::{Bind, Props as _, Show as _, Validity};
use crate::runtime::env::Message;
use crate::testing::{Drive, Fixture, StoryData, StoryOutput, stage, story};
use craft_behavior::kurbo::{Point, Rect as DocRect};
use craft_behavior::{Axis, GesturePhase, Mixed};
use craft_keymap::Mods;
use egui::{Id, Rect, Ui, Vec2};

/// The most guides the invalid story accepts.
const MOST_GUIDES: usize = 2;

/// Shows the guides `control` builds as the only canvas control of a canvas over the [`stage`] of `ui`, over
/// [`StoryDocument::STANDARD`] at 100 % centred on its middle, as `on_canvas` does; the control gets the canvas's rect
/// and its id, which the guides are keyed by.
fn on_guides_canvas(ui: &mut Ui, mut control: impl FnMut(&mut Ui, Rect, Id) -> GuidesOutput) -> StoryOutput {
    let canvas = ui.scope_id().with("guides story canvas");
    let document: &dyn RasterSource = &StoryDocument::STANDARD;
    let room = stage(ui);
    let size = document.size();
    let centre = Point::new(f64::from(size.width) / 2.0, f64::from(size.height) / 2.0);
    let view = story_view(room, ui.pixels_per_point(), centre, 100.0);
    on_canvas_with(ui, room, Canvas::new("Document").id(canvas).view(view), Some(document), |ui, at| control(ui, at, canvas))
}

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
    let travel = 2.0 * crate::testing::input::slop(fixture.ctx());
    fixture.drag(&line, Vec2::new(travel, 0.0));
}

#[story(Guides<'static>, covers = [Default], drive = Drive::Custom(drag_first))]
fn guides_over_a_canvas(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let guides = data.value("guides", guides());
    on_guides_canvas(ui, |ui, at, canvas| {
        Guides::new("Guides", canvas, Bind::bound(&mut *guides)).interactive(true).selected_guide(GuideKey::new(2)).show_at(ui, at)
    })
}

#[story(Guides<'static>, covers = [Mixed], drive = Drive::Custom(drag_first))]
fn guides_that_differ(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let guides = data.value("guides", Mixed::<Vec<Guide>>::Mixed);
    on_guides_canvas(ui, |ui, at, canvas| Guides::new("Guides", canvas, Bind::bound_mixed(&mut *guides)).interactive(true).show_at(ui, at))
}

#[story(Guides<'static>, covers = [Invalid], drive = Drive::Custom(drag_first))]
fn too_many_guides(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let guides = data.value("guides", guides());
    let few = |guides: &Vec<Guide>| {
        if guides.len() > MOST_GUIDES { Validity::Refuse(Message::new("There are too many guides")) } else { Validity::Ok }
    };
    on_guides_canvas(ui, |ui, at, canvas| Guides::new("Guides", canvas, Bind::bound(&mut *guides).check(few)).interactive(true).show_at(ui, at))
}

#[story(Guides<'static>, covers = [Default], drive = Drive::Custom(drag_first))]
fn guides_that_only_draw(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let guides = data.value("guides", guides());
    on_guides_canvas(ui, |ui, at, canvas| Guides::new("Guides", canvas, Bind::bound(&mut *guides)).show_at(ui, at))
}

#[story(Guides<'static>, covers = [Default], drive = Drive::Custom(drag_first))]
fn guide_dragged_out_of_a_ruler(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let guides = data.value("guides", guides());
    let drag = GuideDrag::new_guide(Axis::Y, GesturePhase::Update, Some(200.0), Mods::NONE);
    on_guides_canvas(ui, |ui, at, canvas| Guides::new("Guides", canvas, Bind::bound(&mut *guides)).interactive(true).pending(drag).show_at(ui, at))
}

#[story(Guides<'static>, covers = [Default], drive = Drive::Custom(drag_first))]
fn bounded_guides(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let guides = data.value("guides", guides());
    let page = DocRect::new(120.0, 80.0, 520.0, 320.0);
    on_guides_canvas(ui, |ui, at, canvas| Guides::new("Guides", canvas, Bind::bound(&mut *guides)).interactive(true).bounds(page).show_at(ui, at))
}
