//! Stories of the hover and selection outlines.

use super::{HoverMask, HoverOutline, HoverShape, SelectionOutline};
use crate::canvas::RasterSource as _;
use crate::canvas::testing::canvas_story::{StoryDocument, StoryPicture, StoryShape, accent, on_canvas, on_picture, paint_shapes};
use crate::contract::Show as _;
use crate::testing::{Drive, StoryData, StoryOutput, story};
use craft_behavior::kurbo::{self, BezPath, Circle, Point, Rect, Shape as _};
use egui::{Color32, Ui};

fn bounds() -> Rect {
    Rect::new(240.0, 145.0, 400.0, 255.0)
}

fn paths() -> Vec<BezPath> {
    vec![bounds().to_path(0.1), Rect::new(270.0, 165.0, 320.0, 215.0).to_path(0.1)]
}

/// The disc the coverage story's mask covers.
const DISC: Circle = Circle { center: Point::new(320.0, 200.0), radius: 50.0 };

/// The light picture the hovered objects sit on.
const PICTURE: Color32 = Color32::from_gray(235);

/// The story document with a light picture and `object` on it, filled in the accent.
fn objects<S: kurbo::Shape>(ui: &Ui, object: &S) -> StoryPicture<impl Fn(Point) -> Color32 + use<S>> {
    let shapes = [StoryShape::filled(object, accent(ui, 0.35))];
    let size = StoryDocument::STANDARD.size();
    StoryPicture::new(size.width, size.height, move |p| paint_shapes(&shapes, PICTURE, p))
}

#[story(HoverOutline, covers = [Default], drive = Drive::Never)]
fn rollover(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    let picture = objects(ui, &bounds());
    on_picture(ui, &picture, |ui, at, _| HoverOutline::new("Rollover", Some(HoverShape::Rect(bounds()))).show_at(ui, at))
}

#[story(HoverOutline, covers = [Default], drive = Drive::Never)]
fn coverage_tint(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    let mask = HoverMask::sampled(bounds(), |x, y| if DISC.contains(Point::new(x, y)) { 1.0 } else { 0.0 });
    let picture = objects(ui, &DISC);
    on_picture(ui, &picture, |ui, at, _| HoverOutline::new("Object", Some(HoverShape::Mask(mask.clone()))).show_at(ui, at))
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
