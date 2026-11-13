//! Stories of the raster display.

use super::RasterDisplayOutput;
use super::{Extent, RasterDisplay};
use crate::canvas::testing::canvas_story::{StoryPicture, on_canvas_view, story_view};
use crate::canvas::{AppPaint, PageFrame, Pasteboard, PixelRect};
use crate::canvas_controls::PixelGrid;
use crate::contract::Show as _;
use crate::primitives::{Checker, ContentPainterCx};
use crate::testing::{Drive, StoryData, StoryOutput, story};
use craft_behavior::geometry::quad::Homography;
use craft_behavior::kurbo::{Point, Rect as Rect64};
use craft_behavior::{CanvasView, PixelSize};
use egui::epaint::{Mesh, Vertex};
use egui::{Color32, Rect, Shape, Ui};

/// The document's size in pixels.
const DOCUMENT: PixelSize = PixelSize::new(640, 400);

/// The document's centre, where the views look.
const CENTER: Point = Point::new(320.0, 200.0);

/// How far the zoomed story magnifies the document, so single pixels show sharp.
const MAGNIFIED: f64 = 4.0;

/// The turn of the turned story, in degrees.
const TURN: f64 = 30.0;

/// A diagonal gradient with a transparent top left quarter, so the checkerboard shows through.
fn gradient_pixel(p: Point) -> Color32 {
    let (full_width, full_height) = (f64::from(DOCUMENT.width), f64::from(DOCUMENT.height));
    let seen = p.x > full_width / 2.0 || p.y > full_height / 2.0;
    let level = ((p.x / full_width + p.y / full_height) * 127.0) as u8;
    if seen { Color32::from_rgb(level, 160, u8::MAX - level) } else { Color32::TRANSPARENT }
}

/// The gradient as the app paints it: one colour mesh per seen part of the document through `view`. The colours are
/// linear in the document point, so the mesh's interpolation gives `gradient_pixel`'s pixels.
fn paint_gradient(painter: &ContentPainterCx<'_>, view: CanvasView) {
    paint_gradient_moved(painter, view, |p| p);
}

/// The gradient painted with each document point moved by `moved` before it goes on screen.
fn paint_gradient_moved(painter: &ContentPainterCx<'_>, view: CanvasView, moved: impl Fn(Point) -> Point) {
    let (width, height) = (f64::from(DOCUMENT.width), f64::from(DOCUMENT.height));
    let seen = [Rect64::new(width / 2.0, 0.0, width, height), Rect64::new(0.0, height / 2.0, width / 2.0, height)];
    let shapes = seen.map(|part| {
        let corners = [Point::new(part.x0, part.y0), Point::new(part.x1, part.y0), Point::new(part.x1, part.y1), Point::new(part.x0, part.y1)];
        let mut mesh = Mesh::default();
        let level = |p: Point| ((p.x / width + p.y / height) * 127.0) as u8;
        mesh.vertices.extend(corners.map(|p| Vertex {
            pos: view.to_screen(moved(p)),
            uv: egui::epaint::WHITE_UV,
            color: Color32::from_rgb(level(p), 160, u8::MAX - level(p)),
        }));
        mesh.indices.extend([0, 1, 2, 0, 2, 3]);
        Shape::mesh(mesh)
    });
    painter.data_shapes(shapes);
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
    let source = StoryPicture::new(DOCUMENT.width, DOCUMENT.height, gradient_pixel);
    let display = RasterDisplay::new("Picture", &source);
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

/// The gradient painted by the app, on a pasteboard with its page shadow and under the pixel grid, which reads the
/// same gradient from a source: the display draws the checkerboard and the edge around the app's picture.
#[story(RasterDisplay<'static>, covers = [Default], drive = Drive::Never)]
fn painted(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let view = view_values(data, 100.0, 0.0);
    let checker = checker_of(data);
    let pages = [PageFrame::new(Rect64::new(0.0, 0.0, f64::from(DOCUMENT.width), f64::from(DOCUMENT.height)))];
    let source = StoryPicture::new(DOCUMENT.width, DOCUMENT.height, gradient_pixel);
    on_canvas_view(ui, view, None, |ui, at, _| {
        let _ = Pasteboard::new("Pasteboard", &pages).show_at(ui, at);
        let display = RasterDisplay::painted("Picture", DOCUMENT, AppPaint::new(paint_gradient)).outlined();
        let output = match checker {
            Some(checker) => display.checkerboard(checker),
            None => display,
        }
        .show_at(ui, at);
        let _ = PixelGrid::new("Pixel grid", &source).show_at(ui, at);
        output
    })
}

/// Copies of the gradient around it, as Pattern Preview shows a seamless tile: zoomed out, the canvas fills with
/// copies a whole document apart, the edge on the document alone.
#[story(RasterDisplay<'static>, covers = [Default], drive = Drive::Never)]
fn tiled(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let view = view_values(data, 30.0, 0.0);
    let checker = checker_of(data);
    on_canvas_view(ui, view, None, |ui, at, _| {
        let source = StoryPicture::new(DOCUMENT.width, DOCUMENT.height, gradient_pixel);
        let display = RasterDisplay::new("Picture", &source).outlined().extent(Extent::Tiled);
        match checker {
            Some(checker) => display.checkerboard(checker),
            None => display,
        }
        .show_at(ui, at)
    })
}

/// Where the disc past the document's right edge lies, and its radius, in document pixels.
const DISC: (Point, f64) = (Point::new(640.0, 300.0), 160.0);

/// The gradient inside the document and a disc reaching past its right edge, as a layer moved past the canvas.
fn beyond_pixel(p: Point) -> Color32 {
    let inside = (0.0..f64::from(DOCUMENT.width)).contains(&p.x) && (0.0..f64::from(DOCUMENT.height)).contains(&p.y);
    if inside {
        gradient_pixel(p)
    } else if (p - DISC.0).hypot() <= DISC.1 {
        Color32::from_rgb(220, 120, 40)
    } else {
        Color32::TRANSPARENT
    }
}

/// The document with the pixels past its edge out to a larger region, as Crop shows layers past the canvas while a
/// frame is edited: the checkerboard reaches the region, the edge stays on the document.
#[story(RasterDisplay<'static>, covers = [Default], drive = Drive::Never)]
fn beyond(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let view = view_values(data, 60.0, 0.0);
    let checker = checker_of(data);
    on_canvas_view(ui, view, None, |ui, at, _| {
        let source = StoryPicture::new(DOCUMENT.width, DOCUMENT.height, beyond_pixel);
        let display = RasterDisplay::new("Picture", &source).outlined().extent(Extent::Region(PixelRect::new(-40, -40, 840, 480)));
        match checker {
            Some(checker) => display.checkerboard(checker),
            None => display,
        }
        .show_at(ui, at)
    })
}

/// The gradient turned 90 degrees clockwise about the document's centre, as Image > Rotation previews it before the
/// canvas changes size: the 400 x 640 picture lies off the document's origin, centred on it.
fn turned_a_quarter(p: Point) -> Point {
    Point::new(CENTER.x - (p.y - CENTER.y), CENTER.y + (p.x - CENTER.x))
}

/// The area the quarter-turned gradient covers, in document pixels.
const QUARTER_TURNED: [Point; 4] = {
    let (half_width, half_height) = (DOCUMENT.height as f64 / 2.0, DOCUMENT.width as f64 / 2.0);
    let (x0, y0, x1, y1) = (CENTER.x - half_width, CENTER.y - half_height, CENTER.x + half_width, CENTER.y + half_height);
    [Point::new(x0, y0), Point::new(x1, y0), Point::new(x1, y1), Point::new(x0, y1)]
};

/// A size-changing preview the app paints itself: the gradient turned a quarter about the document's centre. The
/// display is mapped onto the turned picture's area, so the checkerboard and the edge lie under the preview, not on
/// the document.
#[story(RasterDisplay<'static>, covers = [Default], drive = Drive::Never)]
fn rotated(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let view = view_values(data, 60.0, 0.0);
    let checker = checker_of(data);
    let frame = Homography::unit_to_quad(QUARTER_TURNED).expect("the turned area is a rectangle");
    on_canvas_view(ui, view, None, |ui, at, _| {
        let paint = AppPaint::new(|painter: &ContentPainterCx<'_>, view| paint_gradient_moved(painter, view, turned_a_quarter));
        let display = RasterDisplay::painted("Picture", DOCUMENT, paint).mapped(frame, 1).outlined();
        match checker {
            Some(checker) => display.checkerboard(checker),
            None => display,
        }
        .show_at(ui, at)
    })
}
