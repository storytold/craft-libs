//! Stories of the crop overlay.

use super::{CropOverlay, CropOverlayOutput, FRAME};
use crate::canvas::RasterSource as _;
use crate::canvas::testing::canvas_story::{self, StoryDocument, StoryPicture, StoryShape, paint_shapes};
use crate::contract::{Bind, Show as _, Validity};
use crate::runtime::env::Message;
use crate::testing::{Drive, Fixture, StoryData, StoryOutput, story};
use craft_behavior::Mixed;
use craft_behavior::geometry::{BoxPart, CropBox, CropGuide, GuideShow};
use craft_behavior::kurbo::{BezPath, Circle, Point, Rect as DocRect};
use egui::{Color32, Rect, Ui, Vec2};

/// How far the straighten story's horizon leans, in degrees.
const STRAIGHTEN_TILT: f64 = 6.0;

/// The narrowest crop the invalid story accepts, in document units.
const LEAST_WIDTH: f64 = 100.0;

fn crop() -> CropBox {
    CropBox::upright(DocRect::new(240.0, 145.0, 400.0, 255.0))
}

/// Shows an overlay as the only canvas control of a canvas and returns its output.
fn on_canvas(ui: &mut Ui, mut control: impl FnMut(&mut Ui, Rect) -> CropOverlayOutput) -> StoryOutput {
    canvas_story::on_canvas(ui, |ui, at, _| StoryOutput::from(control(ui, at).edit))
}

/// Shows an overlay over [`landscape`] tilted by `tilt` degrees and returns its output.
fn on_landscape(ui: &mut Ui, tilt: f64, mut control: impl FnMut(&mut Ui, Rect) -> CropOverlayOutput) -> StoryOutput {
    canvas_story::on_picture(ui, &landscape(tilt), |ui, at, _| StoryOutput::from(control(ui, at).edit))
}

/// The picture a crop tool crops, on the story document: sky, sun, ground with its horizon tilted by `tilt` degrees,
/// and a hill.
fn landscape(tilt: f64) -> StoryPicture<impl Fn(Point) -> Color32> {
    let size = StoryDocument::STANDARD;
    let (width, height) = (size.rect().width(), size.rect().height());
    let rise = width / 2.0 * tilt.to_radians().tan();
    let horizon = height * 0.6;
    let ground = [Point::new(0.0, horizon - rise), Point::new(width, horizon + rise), Point::new(width, height), Point::new(0.0, height)];
    let hill = [Point::new(width * 0.1, height), Point::new(width * 0.3, height * 0.62), Point::new(width * 0.55, height)];
    let shapes = [
        StoryShape::filled(&Circle::new((width * 0.75, height * 0.22), height * 0.1), Color32::from_rgb(250, 220, 110)),
        StoryShape::filled(&polygon(&ground), Color32::from_rgb(96, 150, 90)),
        StoryShape::filled(&polygon(&hill), Color32::from_rgb(70, 120, 70)),
    ];
    let sky = Color32::from_rgb(150, 200, 235);
    let pixels = size.size();
    StoryPicture::new(pixels.width, pixels.height, move |p| paint_shapes(&shapes, sky, p))
}

/// The closed path through `points`.
fn polygon(points: &[Point]) -> BezPath {
    let mut path = BezPath::new();
    for (index, point) in points.iter().enumerate() {
        if index == 0 { path.move_to(*point) } else { path.line_to(*point) }
    }
    path.close_path();
    path
}

/// Drags the top left corner towards the middle of the crop by twice the drag threshold.
fn drag_top_left(fixture: &mut Fixture) {
    let Some(id) = fixture.root_id() else { return };
    let Some(grip) = fixture.part(id, FRAME.at(BoxPart::Corner(0).index())) else { return };
    let travel = 2.0 * crate::testing::input::max_click_dist(fixture.ctx());
    fixture.drag(&grip, Vec2::new(travel, 0.0));
}

#[story(CropOverlay<'static>, covers = [Default], drive = Drive::Custom(drag_top_left))]
fn upright(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let rect = data.value("crop", crop());
    on_canvas(ui, |ui, at| CropOverlay::new("Crop", Bind::bound(&mut *rect)).show_at(ui, at))
}

#[story(CropOverlay<'static>, covers = [Mixed], drive = Drive::Custom(drag_top_left))]
fn crops_that_differ(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let rect = data.value("crop", Mixed::<CropBox>::Mixed);
    on_canvas(ui, |ui, at| CropOverlay::new("Crop", Bind::bound_mixed(&mut *rect)).show_at(ui, at))
}

#[story(CropOverlay<'static>, covers = [Invalid], drive = Drive::Custom(drag_top_left))]
fn too_narrow(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let rect = data.value("crop", CropBox::upright(DocRect::new(240.0, 145.0, 240.0 + LEAST_WIDTH / 2.0, 225.0)));
    on_canvas(ui, |ui, at| {
        let narrow = |crop: &CropBox| {
            if crop.rect.width() < LEAST_WIDTH { Validity::Refuse(Message::new("The crop is too narrow")) } else { Validity::Ok }
        };
        CropOverlay::new("Crop", Bind::bound(&mut *rect).check(narrow)).show_at(ui, at)
    })
}

#[story(CropOverlay<'static>, covers = [Default])]
fn turned(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let rect = data.value("crop", CropBox::new(DocRect::new(260.0, 165.0, 380.0, 235.0), 15.0));
    on_canvas(ui, |ui, at| CropOverlay::new("Crop", Bind::bound(&mut *rect)).show_at(ui, at))
}

#[story(CropOverlay<'static>, covers = [Default])]
fn ratio_held(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let rect = data.value("crop", CropBox::upright(DocRect::new(240.0, 145.0, 400.0, 235.0)));
    on_landscape(ui, 0.0, |ui, at| CropOverlay::new("Crop", Bind::bound(&mut *rect)).ratio(16.0 / 9.0).show_at(ui, at))
}

#[story(CropOverlay<'static>, covers = [Default])]
fn guide_shown(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let rect = data.value("crop", crop());
    on_canvas(ui, |ui, at| {
        CropOverlay::new("Crop", Bind::bound(&mut *rect)).guide((CropGuide::Thirds, 0)).guide_show(GuideShow::Always).show_at(ui, at)
    })
}

#[story(CropOverlay<'static>, covers = [Default])]
fn straighten_armed(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let rect = data.value("crop", crop());
    on_landscape(ui, STRAIGHTEN_TILT, |ui, at| CropOverlay::new("Crop", Bind::bound(&mut *rect)).straighten().show_at(ui, at))
}

#[story(CropOverlay<'static>, covers = [Default])]
fn image_mode(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let rect = data.value("crop", crop());
    on_landscape(ui, 0.0, |ui, at| CropOverlay::new("Crop", Bind::bound(&mut *rect)).image_mode().auto_center().show_at(ui, at))
}
