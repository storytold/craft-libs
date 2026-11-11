//! Stories of the scroll bars.

use super::{CanvasScrollbars, HORIZONTAL};
use crate::canvas::Canvas;
use crate::canvas::testing::canvas_story::{StoryDocument, on_canvas_with, story_view};
use crate::contract::{Bind, Show as _, Validity};
use crate::runtime::env::Message;
use crate::testing::stage;
use crate::testing::{Drive, Fixture, StoryData, StoryOutput, Target, story};
use craft_behavior::kurbo::{Point, Rect as DocRect};
use craft_behavior::{CanvasView, Mixed};
use egui::{Rect, Ui, Vec2, vec2};

/// How far the thumb is dragged, in drag thresholds.
const DRAG_THRESHOLDS: f32 = 3.0;

/// Where the thumb is pressed, in drag thresholds from the start of its track: the views of the stories start at the
/// document's start, so the thumb begins there and is longer than this.
const GRIP_THRESHOLDS: f32 = 1.5;

/// The document the bars scroll, in document units.
const DOCUMENT: DocRect = DocRect::new(0.0, 0.0, 800.0, 600.0);
/// The document the canvas shows, the same size as [`DOCUMENT`].
const SHOWN: StoryDocument = StoryDocument::new(800, 600);

/// The zoom that shows part of the document in the story's canvas, in percent of one device pixel per document pixel.
const ZOOM: f64 = 200.0;

/// The view centre of the stories: the document's start, which puts each thumb at the start of its track.
const CENTRE: Point = Point::new(0.0, 0.0);

/// The view centre the invalid story refuses to leave to the right.
const LAST_CENTRE_X: f64 = 0.0;

fn view_over(ui: &Ui, rect: Rect) -> CanvasView {
    story_view(rect, ui.pixels_per_point(), CENTRE, ZOOM)
}

fn drag_thumb(fixture: &mut Fixture) {
    let Some(id) = fixture.root_id() else { return };
    let Some(track) = fixture.part(id, HORIZONTAL) else { return };
    let threshold = crate::testing::input::max_click_dist(fixture.ctx());
    let grip =
        Target { node: track.node, rect: Rect::from_center_size(track.rect.left_center() + vec2(GRIP_THRESHOLDS * threshold, 0.0), Vec2::ZERO) };
    fixture.drag(&grip, Vec2::new(DRAG_THRESHOLDS * threshold, 0.0));
}

#[story(CanvasScrollbars<'static>, covers = [Default], drive = Drive::Custom(drag_thumb))]
fn scrolled_view(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let full = stage(ui);
    let shown = view_over(ui, full);
    let overscroll = *data.value("overscroll", false);
    let view = data.value("view", shown);
    on_canvas_with(ui, full, Canvas::new("Document").view(shown), Some(&SHOWN), |ui, at| {
        CanvasScrollbars::new("Scroll bars", Bind::bound(&mut *view), DOCUMENT).overscroll(overscroll).show_at(ui, at)
    })
}

#[story(CanvasScrollbars<'static>, covers = [Mixed], drive = Drive::Custom(drag_thumb))]
fn views_that_differ(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let full = stage(ui);
    let shown = view_over(ui, full);
    let overscroll = *data.value("overscroll", false);
    let view = data.value("view", Mixed::<CanvasView>::Mixed);
    on_canvas_with(ui, full, Canvas::new("Document").view(shown), Some(&SHOWN), |ui, at| {
        CanvasScrollbars::new("Scroll bars", Bind::bound_mixed(&mut *view), DOCUMENT).overscroll(overscroll).show_at(ui, at)
    })
}

#[story(CanvasScrollbars<'static>, covers = [Invalid], drive = Drive::Custom(drag_thumb))]
fn view_scrolled_too_far(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let full = stage(ui);
    let shown = view_over(ui, full);
    let overscroll = *data.value("overscroll", false);
    let view = data.value("view", shown);
    on_canvas_with(ui, full, Canvas::new("Document").view(shown), Some(&SHOWN), |ui, at| {
        let kept = |view: &CanvasView| {
            if view.center.x > LAST_CENTRE_X { Validity::Refuse(Message::new("The view cannot scroll further right")) } else { Validity::Ok }
        };
        CanvasScrollbars::new("Scroll bars", Bind::bound(&mut *view).check(kept), DOCUMENT).overscroll(overscroll).show_at(ui, at)
    })
}
