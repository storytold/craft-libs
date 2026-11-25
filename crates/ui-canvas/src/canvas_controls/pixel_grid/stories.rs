//! Stories of `PixelGrid`.

use super::PixelGrid;
use crate::canvas::testing::canvas_story::{StoryPicture, on_canvas_view, story_view};
use crate::contract::Show as _;
use crate::testing::{Drive, StoryData, StoryOutput, story};
use craft_behavior::kurbo::Point;
use egui::{Color32, Ui};

/// The zoom the story shows the document at, in percent of one device pixel per document pixel: above the grid's
/// threshold.
const ZOOM: f64 = 800.0;

const SIDE: u32 = 16;

/// A document of two grey halves.
fn halves() -> StoryPicture<impl Fn(Point) -> Color32> {
    StoryPicture::new(SIDE, SIDE, |p| if p.x < f64::from(SIDE) / 2.0 { Color32::from_gray(30) } else { Color32::from_gray(225) })
}

#[story(PixelGrid<'static>, covers = [Default], drive = Drive::Never)]
fn magnified(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    let centre = Point::new(f64::from(SIDE) / 2.0, f64::from(SIDE) / 2.0);
    let halves = halves();
    on_canvas_view(
        ui,
        |room, ppp| story_view(room, ppp, centre, ZOOM),
        Some(&halves),
        |ui, at, _| PixelGrid::new("Pixel grid", &halves).show_at(ui, at),
    )
}
