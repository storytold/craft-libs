//! Stories of `SmartGuides`.

use super::{Measure, SmartGuides, SmartLine};
use crate::canvas::testing::canvas_story::{StoryDocument, on_canvas_view};
use crate::canvas::{PixelRect, RasterSource};
use crate::contract::Show as _;
use crate::primitives::Revision;
use crate::testing::{StoryData, StoryOutput, story};
use craft_behavior::PixelSize;
use craft_behavior::kurbo::{Point, Rect as DocRect};
use egui::{Color32, ColorImage, Ui};

/// The box that stays put.
const STILL: DocRect = DocRect::new(120.0, 80.0, 280.0, 200.0);

/// The box being moved: its top aligned with the still box's, 80 units to its right.
const MOVING: DocRect = DocRect::new(360.0, 80.0, 480.0, 160.0);

/// Where the distance between the boxes runs, below both.
const MEASURE_Y: f64 = 240.0;

/// The story document: [`StoryDocument::STANDARD`] with the two boxes outlined on it.
struct Boxes;

impl Boxes {
    /// The outline's width, in document pixels.
    const OUTLINE: f64 = 2.0;
    const INK: Color32 = Color32::from_gray(60);

    /// Whether the document point `p` is on a box's outline.
    fn inked(p: Point) -> bool {
        [STILL, MOVING].iter().any(|r| r.contains(p) && !r.inset(-Self::OUTLINE).contains(p))
    }
}

impl RasterSource for Boxes {
    fn size(&self) -> PixelSize {
        StoryDocument::STANDARD.size()
    }

    fn revision(&self) -> Revision {
        Revision::new(1)
    }

    fn damage(&self, _: Revision) -> Option<PixelRect> {
        Some(PixelRect::default())
    }

    fn read(&self, region: PixelRect, factor: u32, out: &mut ColorImage) {
        let [width, height] = out.size;
        for row in 0..height {
            for column in 0..width {
                let x = f64::from(region.x0) + (column as f64 + 0.5) * f64::from(factor);
                let y = f64::from(region.y0) + (row as f64 + 0.5) * f64::from(factor);
                out.pixels[row * width + column] = if Self::inked(Point::new(x, y)) { Self::INK } else { Color32::TRANSPARENT };
            }
        }
    }
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
    on_canvas_view(ui, view, Some(&Boxes), |ui, at, _| {
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
