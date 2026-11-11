//! Stories of the hover and selection outlines.

use super::{HoverMask, HoverOutline, HoverShape, SelectionOutline};
use crate::canvas::testing::canvas_story::on_canvas;
use crate::contract::Show as _;
use crate::testing::{Drive, StoryData, StoryOutput, story};
use craft_behavior::kurbo::{BezPath, Rect, Shape as _};
use egui::Ui;

fn bounds() -> Rect {
    Rect::new(240.0, 145.0, 400.0, 255.0)
}

fn paths() -> Vec<BezPath> {
    vec![bounds().to_path(0.1), Rect::new(270.0, 165.0, 320.0, 215.0).to_path(0.1)]
}

#[story(HoverOutline, covers = [Default], drive = Drive::Never)]
fn rollover(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    on_canvas(ui, |ui, at, _| HoverOutline::new("Rollover", Some(HoverShape::Rect(bounds()))).show_at(ui, at))
}

#[story(HoverOutline, covers = [Default], drive = Drive::Never)]
fn coverage_tint(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    let mask = HoverMask::sampled(bounds(), |x, y| if (x - 320.0).hypot(y - 200.0) < 50.0 { 1.0 } else { 0.0 });
    on_canvas(ui, |ui, at, _| HoverOutline::new("Object", Some(HoverShape::Mask(mask.clone()))).show_at(ui, at))
}

#[story(SelectionOutline<'static>, covers = [Default], drive = Drive::Never)]
fn solid(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    let paths = paths();
    on_canvas(ui, |ui, at, _| SelectionOutline::solid("Selected paths", &paths).show_at(ui, at))
}

#[story(SelectionOutline<'static>, covers = [Default], drive = Drive::Never)]
fn dashed(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    let paths = paths();
    on_canvas(ui, |ui, at, _| SelectionOutline::dashed("Selected paths", &paths).show_at(ui, at))
}
