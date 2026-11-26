//! Stories of `SmartGuides`.

use super::{Measure, SmartGuides, SmartLine};
use crate::canvas::RasterSource as _;
use crate::canvas::testing::canvas_story::{OUTLINE, StoryDocument, StoryPicture, StoryShape, on_canvas_view, paint_shapes};
use crate::contract::Show as _;
use crate::testing::{StoryData, StoryOutput, story};
use craft_behavior::kurbo::{Point, Rect as DocRect};
use egui::{Color32, Ui};

/// The box that stays put.
const STILL: DocRect = DocRect::new(120.0, 80.0, 280.0, 200.0);

/// The box being moved: its top aligned with the still box's, 80 units to its right.
const MOVING: DocRect = DocRect::new(360.0, 80.0, 480.0, 160.0);

/// Where the distance between the boxes runs, below both.
const MEASURE_Y: f64 = 240.0;

/// The colour the boxes are outlined in.
const INK: Color32 = Color32::from_gray(60);

/// The story document: [`StoryDocument::STANDARD`] with the two boxes outlined on it.
fn boxes() -> StoryPicture<impl Fn(Point) -> Color32> {
    let shapes = [STILL, MOVING].map(|r| StoryShape::line(&r, OUTLINE, INK));
    let size = StoryDocument::STANDARD.size();
    StoryPicture::new(size.width, size.height, move |p| paint_shapes(&shapes, Color32::TRANSPARENT, p))
}

/// The guides of the move: the boxes' tops on one line, the facing edges extended down to the distance between them.
fn lines() -> Vec<SmartLine> {
    vec![
        SmartLine::new(Point::new(STILL.x0, STILL.y0), Point::new(MOVING.x1, MOVING.y0)),
        SmartLine::new(Point::new(STILL.x1, STILL.y1), Point::new(STILL.x1, MEASURE_Y)).dashed(),
        SmartLine::new(Point::new(MOVING.x0, MOVING.y1), Point::new(MOVING.x0, MEASURE_Y)).dashed(),
    ]
}

/// Shows the guides of the move over the boxes, the moving box outlined by `SmartGuides` when `bounds` is set.
fn on_boxes(ui: &mut Ui, bounds: bool) -> StoryOutput {
    let lines = lines();
    let measures = [Measure::new(Point::new(STILL.x1, MEASURE_Y), Point::new(MOVING.x0, MEASURE_Y))];
    let view = |room, ppp| StoryDocument::STANDARD.view(room, ppp, 100.0);
    on_canvas_view(ui, view, Some(&boxes()), |ui, at, _| {
        let guides = SmartGuides::new(&lines, &measures);
        if bounds { guides.bounds(MOVING) } else { guides }.show_at(ui, at)
    })
}

#[story(SmartGuides<'static>, covers = [Default])]
fn move_a_box(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    on_boxes(ui, false)
}

#[story(SmartGuides<'static>, covers = [Default])]
fn move_a_box_with_its_bounds(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    on_boxes(ui, true)
}
