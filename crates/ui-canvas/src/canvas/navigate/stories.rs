//! Stories of the compass.

use crate::canvas::rotate::RIM;
use crate::canvas::testing::canvas_story::on_canvas_with;
use crate::canvas::{Canvas, RotateView, RotateViewOutput};
use crate::contract::{Bind, Show as _, Validity};
use crate::runtime::env::Message;
use crate::testing::{Drive, Fixture, StoryData, StoryOutput, story};
use craft_behavior::Mixed;
use egui::{Ui, Vec2};

/// The angle the stories start at, in degrees.
const TURNED: f64 = 30.0;

/// The largest angle the invalid story accepts, in degrees.
const MOST: f64 = 90.0;

/// Shows `compass` in the corner of a canvas and returns its output.
fn in_corner(ui: &mut Ui, mut compass: impl FnMut(&mut Ui, egui::Rect) -> RotateViewOutput) -> StoryOutput {
    let room = crate::testing::stage(ui);
    on_canvas_with(ui, room, Canvas::new("Document"), None, |ui, rect| {
        let at = RotateView::compass_rect(ui, rect);
        compass(ui, at)
    })
}

/// Drags the compass rim to the right by twice the drag threshold. The compass is a member of its canvas's arena, so
/// the automatic drive finds no drag part.
fn drag_rim(fixture: &mut Fixture) {
    let Some(id) = fixture.root_id() else { return };
    let Some(rim) = fixture.part(id, RIM) else { return };
    let travel = 2.0 * crate::testing::input::max_click_dist(fixture.ctx());
    fixture.drag(&rim, Vec2::new(travel, 0.0));
}

#[story(RotateView<'static>, covers = [Default], drive = Drive::Custom(drag_rim))]
fn compass(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let angle = data.value("angle", TURNED);
    in_corner(ui, |ui, at| RotateView::new("Rotation", Bind::bound(&mut *angle), false).show_at(ui, at))
}

#[story(RotateView<'static>, covers = [Mixed], drive = Drive::Custom(drag_rim))]
fn views_that_differ(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let angle = data.value("angle", Mixed::<f64>::Mixed);
    in_corner(ui, |ui, at| RotateView::new("Rotation", Bind::bound_mixed(&mut *angle), false).show_at(ui, at))
}

#[story(RotateView<'static>, covers = [Invalid], drive = Drive::Custom(drag_rim))]
fn too_far(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let angle = data.value("angle", MOST + TURNED);
    in_corner(ui, |ui, at| {
        let near =
            |angle: &f64| if angle.abs() > MOST { Validity::Refuse(Message::new("The view turns at most a quarter turn")) } else { Validity::Ok };
        RotateView::new("Rotation", Bind::bound(&mut *angle).check(near), false).show_at(ui, at)
    })
}
