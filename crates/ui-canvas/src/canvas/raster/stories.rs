//! Stories of the raster display.

use super::RasterDisplay;
use super::RasterDisplayOutput;
use crate::canvas::testing::canvas_story::{on_canvas_view, story_view};
use crate::canvas::{PixelRect, RasterSource};
use crate::contract::Show as _;
use crate::primitives::{Checker, Revision};
use crate::testing::{Drive, StoryData, StoryOutput, story};
use craft_behavior::kurbo::Point;
use craft_behavior::{CanvasView, PixelSize};
use egui::{Color32, ColorImage, Rect, Ui};

/// The document's size in pixels.
const DOCUMENT: PixelSize = PixelSize::new(640, 400);

/// The document's centre, where the views look.
const CENTER: Point = Point::new(320.0, 200.0);

/// How far the zoomed story magnifies the document, so single pixels show sharp.
const MAGNIFIED: f64 = 4.0;

/// The turn of the turned story, in degrees.
const TURN: f64 = 30.0;

/// A diagonal gradient with a transparent top left quarter, so the checkerboard shows through.
struct Gradient;

impl RasterSource for Gradient {
    fn size(&self) -> PixelSize {
        DOCUMENT
    }

    fn revision(&self) -> Revision {
        Revision::new(1)
    }

    fn damage(&self, _: Revision) -> Option<PixelRect> {
        Some(PixelRect::default())
    }

    fn read(&self, region: PixelRect, factor: u32, out: &mut ColorImage) {
        let [width, height] = out.size;
        let (full_width, full_height) = (DOCUMENT.width as f32, DOCUMENT.height as f32);
        for row in 0..height {
            for column in 0..width {
                let x = region.x0 as f32 + (column as f32 + 0.5) * factor as f32;
                let y = region.y0 as f32 + (row as f32 + 0.5) * factor as f32;
                let seen = x > full_width / 2.0 || y > full_height / 2.0;
                let level = ((x / full_width + y / full_height) * 127.0) as u8;
                out.pixels[row * width + column] = if seen { Color32::from_rgb(level, 160, u8::MAX - level) } else { Color32::TRANSPARENT };
            }
        }
    }
}

/// The checkerboard cell sizes the stories offer.
#[derive(Clone, Copy, Debug, PartialEq)]
enum CheckerSize {
    None,
    Small,
    Medium,
    Large,
}

/// The view values the Properties panel edits: zoom in percent, rotation in degrees, flip and pixel aspect.
fn view_values(data: &mut StoryData, zoom: f64, rotation: f64) -> impl Fn(Rect, f32) -> CanvasView + Copy + use<> {
    let zoom = *data.ranged("zoom", zoom, 10.0..=800.0);
    let rotation = *data.ranged("rotation", rotation, -180.0..=180.0);
    let flip = *data.value("flip", false);
    let aspect = *data.ranged("aspect", 1.0, 0.5..=2.0);
    move |room, ppp| story_view(room, ppp, CENTER, zoom).rotation(rotation).flip_x(flip).aspect(aspect)
}

/// The transparency grid under the document, from the story's `checker` value.
fn checker_of(data: &mut StoryData) -> Option<Checker> {
    let sizes = [("None", CheckerSize::None), ("Small", CheckerSize::Small), ("Medium", CheckerSize::Medium), ("Large", CheckerSize::Large)];
    match *data.choice("checker", CheckerSize::Medium, &sizes) {
        CheckerSize::None => None,
        CheckerSize::Small => Some(Checker::new().size(4.0)),
        CheckerSize::Medium => Some(Checker::new()),
        CheckerSize::Large => Some(Checker::new().size(16.0)),
    }
}

/// The gradient, filling the canvas, over `checker`.
fn gradient(ui: &mut Ui, at: Rect, checker: Option<Checker>) -> RasterDisplayOutput {
    static SOURCE: Gradient = Gradient;
    let display = RasterDisplay::new("Picture", &SOURCE);
    match checker {
        Some(checker) => display.checkerboard(checker),
        None => display,
    }
    .show_at(ui, at)
}

#[story(RasterDisplay<'static>, covers = [Default], drive = Drive::Never)]
fn document(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let view = view_values(data, 100.0, 0.0);
    let checker = checker_of(data);
    on_canvas_view(ui, view, None, |ui, at, _| gradient(ui, at, checker))
}

#[story(RasterDisplay<'static>, covers = [Default], drive = Drive::Never)]
fn magnified(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let view = view_values(data, MAGNIFIED * 100.0, 0.0);
    let checker = checker_of(data);
    on_canvas_view(ui, view, None, |ui, at, _| gradient(ui, at, checker))
}

#[story(RasterDisplay<'static>, covers = [Default], drive = Drive::Never)]
fn turned(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let view = view_values(data, 100.0, TURN);
    let checker = checker_of(data);
    on_canvas_view(ui, view, None, |ui, at, _| gradient(ui, at, checker))
}
