//! Stories of `CanvasGrid`.

use super::{CanvasGrid, GridCrowding, GridLine};
use crate::canvas::testing::canvas_story::{StoryDocument, StoryPicture, on_canvas, on_canvas_view, on_canvas_with, story_view};
use crate::canvas::{Canvas, RasterDisplay, RasterSource as _};
use crate::contract::Show as _;
use crate::testing::{Drive, StoryData, StoryOutput, stage, story};
use craft_behavior::kurbo::Point;
use egui::{Color32, Ui};

const MAJOR: f64 = 100.0;
const SUBDIVISIONS: u32 = 4;

fn grid() -> CanvasGrid {
    CanvasGrid::new("Grid").spacing((MAJOR, MAJOR).into()).subdivisions(SUBDIVISIONS).bounds(StoryDocument::STANDARD.rect())
}

#[story(CanvasGrid, covers = [Default], drive = Drive::Never)]
fn solid(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    on_canvas(ui, |ui, at, _| grid().show_at(ui, at))
}

#[story(CanvasGrid, covers = [Default], drive = Drive::Never)]
fn dashed(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    on_canvas(ui, |ui, at, _| grid().line(GridLine::Dashed).show_at(ui, at))
}

#[story(CanvasGrid, covers = [Default], drive = Drive::Never)]
fn dots(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    on_canvas(ui, |ui, at, _| grid().line(GridLine::Dots).show_at(ui, at))
}

#[story(CanvasGrid, covers = [Default], drive = Drive::Never)]
fn crossings(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    on_canvas(ui, |ui, at, _| grid().line(GridLine::Crossings).show_at(ui, at))
}

#[story(CanvasGrid, covers = [Default], drive = Drive::Never)]
fn grid_in_back(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    let size = StoryDocument::STANDARD.size();
    // A picture over the left half of the document: the grid shows only beside it.
    let picture = StoryPicture::new(size.width / 2, size.height, |_| Color32::from_gray(200));
    let room = stage(ui);
    let centre = Point::new(f64::from(size.width) / 2.0, f64::from(size.height) / 2.0);
    let view = story_view(room, ui.pixels_per_point(), centre, 100.0);
    on_canvas_with(ui, room, Canvas::new("Document").view(view), None, |ui, at| {
        let shown = grid().picture().show_at(ui, at);
        let _ = RasterDisplay::new("Picture", &picture).picture().show_at(ui, at);
        shown
    })
}

#[story(CanvasGrid, covers = [Default], drive = Drive::Never)]
fn thinned(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    // At 5 % the 100-unit step is five points, under the minimum, so `Thin` doubles it until the lines fit.
    on_canvas_view(
        ui,
        |room, ppp| StoryDocument::STANDARD.view(room, ppp, 5.0),
        Some(&StoryDocument::STANDARD),
        |ui, at, _| grid().crowding(GridCrowding::Thin).show_at(ui, at),
    )
}
