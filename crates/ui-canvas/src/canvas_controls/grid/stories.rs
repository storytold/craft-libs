//! Stories of `CanvasGrid`.

use super::{CanvasGrid, GridLine};
use crate::canvas::testing::canvas_story::{StoryDocument, on_canvas};
use crate::contract::Show as _;
use crate::testing::{Drive, StoryData, StoryOutput, story};
use egui::Ui;

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
