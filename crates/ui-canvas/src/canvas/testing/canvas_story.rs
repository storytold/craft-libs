//! The canvas the stories of canvas controls show their control on: a canvas over the [`stage`] with a document under
//! the control, drawn by [`RasterDisplay`] on the story document's ground: white with grey squares of
//! [`StoryDocument::SQUARE`] document pixels, which scale and turn with the view.

use crate::canvas::{Canvas, PixelRect, RasterDisplay, RasterSource};
use crate::contract::{Show as _, ShowContainer as _};
use crate::primitives::Revision;
use crate::testing::{StoryOutput, stage};
use craft_behavior::geometry::quad::Homography;
use craft_behavior::kurbo::{Point, Rect as DocRect};
use craft_behavior::{CanvasView, PixelSize};
use egui::{Color32, ColorImage, Rect, Ui};

/// A document with no picture: the ground shows over all of it. [`StoryDocument::STANDARD`] is the document of a
/// story that shows no picture.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct StoryDocument(PixelSize);

impl StoryDocument {
    /// The story document: 640 by 400 pixels.
    pub const STANDARD: StoryDocument = StoryDocument(PixelSize::new(640, 400));

    /// The side of a ground square, in document pixels.
    pub const SQUARE: f64 = 40.0;

    /// The ground's colour at the document point `p`: grey squares on white, the first square grey.
    pub fn ground(p: Point) -> Color32 {
        let (column, row) = ((p.x / Self::SQUARE).floor() as i64, (p.y / Self::SQUARE).floor() as i64);
        if (column + row).rem_euclid(2) == 0 { GROUND_DARK } else { GROUND_LIGHT }
    }

    /// A transparent document of `width` by `height` pixels.
    pub const fn new(width: u32, height: u32) -> Self {
        StoryDocument(PixelSize::new(width, height))
    }

    /// The document's rect, in document units.
    pub fn rect(&self) -> DocRect {
        DocRect::new(0.0, 0.0, f64::from(self.0.width), f64::from(self.0.height))
    }

    /// The document's middle, in document units.
    pub fn centre(&self) -> Point {
        self.rect().center()
    }

    /// The view of `room` that shows the document at `percent` centred on its middle ([`story_view`]).
    pub fn view(&self, room: Rect, pixels_per_point: f32, percent: f64) -> CanvasView {
        story_view(room, pixels_per_point, self.centre(), percent)
    }
}

impl RasterSource for StoryDocument {
    fn size(&self) -> PixelSize {
        self.0
    }

    fn revision(&self) -> Revision {
        Revision::new(1)
    }

    fn damage(&self, _: Revision) -> Option<PixelRect> {
        Some(PixelRect::default())
    }

    fn read(&self, _: PixelRect, _: u32, out: &mut ColorImage) {
        out.pixels.fill(Color32::TRANSPARENT);
    }
}

/// The ground's light and dark colours.
const GROUND_LIGHT: Color32 = Color32::from_rgb(255, 255, 255);
const GROUND_DARK: Color32 = Color32::from_rgb(204, 204, 204);

/// Calls `paint` with the document point at the centre of each texel of `out`, which holds `region` at one texel per
/// `factor` pixels, and the texel.
fn each_pixel(region: PixelRect, factor: u32, out: &mut ColorImage, mut paint: impl FnMut(Point, &mut Color32)) {
    let width = out.size[0];
    let step = f64::from(factor);
    for (index, pixel) in out.pixels.iter_mut().enumerate() {
        let (column, row) = ((index % width) as f64, (index / width) as f64);
        paint(Point::new(f64::from(region.x0) + (column + 0.5) * step, f64::from(region.y0) + (row + 0.5) * step), pixel);
    }
}

/// `picture` on the story document's ground: the picture's pixels over [`StoryDocument::ground`].
pub fn on_ground(picture: &dyn RasterSource) -> impl RasterSource + '_ {
    OnGround(picture)
}

struct OnGround<'a>(&'a dyn RasterSource);

impl RasterSource for OnGround<'_> {
    fn size(&self) -> PixelSize {
        self.0.size()
    }

    fn revision(&self) -> Revision {
        self.0.revision()
    }

    fn damage(&self, since: Revision) -> Option<PixelRect> {
        self.0.damage(since)
    }

    fn read(&self, region: PixelRect, factor: u32, out: &mut ColorImage) {
        self.0.read(region, factor, out);
        each_pixel(region, factor, out, |p, pixel| *pixel = StoryDocument::ground(p).blend(*pixel));
    }
}

/// A view of `room` with the document point `centre` at its middle, at `percent` of one device pixel per document
/// unit (100: one device pixel per document pixel) for `pixels_per_point` device pixels per point.
pub fn story_view(room: Rect, pixels_per_point: f32, centre: Point, percent: f64) -> CanvasView {
    CanvasView::new(room, centre, percent / 100.0 / f64::from(pixels_per_point))
}

/// Shows `control` as the only canvas control of a canvas over the [`stage`] of `ui`, over [`StoryDocument::STANDARD`]
/// at 100 % centred on its middle ([`story_view`]), and returns the control's output. The control gets the canvas's
/// rect and the view it shows.
pub fn on_canvas<R: Into<StoryOutput>>(ui: &mut Ui, control: impl FnMut(&mut Ui, Rect, &CanvasView) -> R) -> StoryOutput {
    on_canvas_view(ui, |room, ppp| StoryDocument::STANDARD.view(room, ppp, 100.0), Some(&StoryDocument::STANDARD), control)
}

/// The same over the view `view` makes of the room left in `ui` and its pixels per point, and over `document`
/// ([`on_canvas_with`]).
pub fn on_canvas_view<R: Into<StoryOutput>>(
    ui: &mut Ui,
    view: impl FnOnce(Rect, f32) -> CanvasView,
    document: Option<&dyn RasterSource>,
    mut control: impl FnMut(&mut Ui, Rect, &CanvasView) -> R,
) -> StoryOutput {
    let room = stage(ui);
    let view = view(room, ui.pixels_per_point());
    on_canvas_with(ui, room, Canvas::new("Document").view(view), document, |ui, rect| control(ui, rect, &view))
}

/// Shows `control` as the only canvas control of `canvas`, which the caller sets up over `room` (from [`stage`]), over
/// `document` (none: the bare canvas, for a story that draws its own document), and returns the control's output.
/// The body runs once to measure and once to draw, so `control` builds the control on each call. The document shows
/// only under a canvas with a view.
pub fn on_canvas_with<R: Into<StoryOutput>>(
    ui: &mut Ui,
    room: Rect,
    canvas: Canvas<'_>,
    document: Option<&dyn RasterSource>,
    mut control: impl FnMut(&mut Ui, Rect) -> R,
) -> StoryOutput {
    let shown = canvas.show_at(ui, room, |ui, body| {
        if let Some(document) = document {
            show_document(ui, body.rect, document);
        }
        control(ui, body.rect)
    });
    shown.inner.map(Into::into).unwrap_or_default()
}

/// Shows `document` on the story document's ground ([`on_ground`]) at the canvas rect `at`, in the body of a canvas
/// with a view, under the canvas controls shown after it.
pub fn show_document(ui: &mut Ui, at: Rect, document: &dyn RasterSource) {
    RasterDisplay::new("", &on_ground(document)).show_at(ui, at);
}

/// A picture of one colour: a flat layer or page that a story shows on its document with [`show_quad`].
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FlatPicture(Color32);

impl FlatPicture {
    /// A picture of `color`.
    pub const fn new(color: Color32) -> Self {
        FlatPicture(color)
    }
}

impl RasterSource for FlatPicture {
    fn size(&self) -> PixelSize {
        PixelSize::new(1, 1)
    }

    fn revision(&self) -> Revision {
        Revision::new(1)
    }

    fn damage(&self, _: Revision) -> Option<PixelRect> {
        Some(PixelRect::default())
    }

    fn read(&self, _: PixelRect, _: u32, out: &mut ColorImage) {
        out.pixels.fill(self.0);
    }
}

/// Shows `picture`, named `label`, filling the document quad `quad` (its corners clockwise from the top left) at the
/// canvas rect `at`, over the document and under the canvas controls shown after it. A quad with no area shows nothing.
pub fn show_quad(ui: &mut Ui, at: Rect, label: &'static str, quad: [Point; 4], picture: &FlatPicture) {
    if let Some(frame) = Homography::unit_to_quad(quad) {
        RasterDisplay::new(label, picture).mapped(frame, 1).show_at(ui, at);
    }
}
