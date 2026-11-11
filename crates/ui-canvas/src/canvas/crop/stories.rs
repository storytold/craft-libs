//! Stories of the crop overlay.

use super::{CropOverlay, CropOverlayOutput, FRAME};
use crate::contract::{Bind, Show as _, Validity};
use crate::runtime::env::Message;
use crate::testing::{Drive, Fixture, StoryData, StoryOutput, story};
use craft_behavior::Mixed;
use craft_behavior::geometry::{BoxPart, CropBox, CropGuide, GuideShow};
use craft_behavior::kurbo::Rect as DocRect;
use egui::{Rect, Ui, Vec2};

/// The narrowest crop the invalid story accepts, in document units.
const LEAST_WIDTH: f64 = 100.0;

fn crop() -> CropBox {
    CropBox::upright(DocRect::new(240.0, 145.0, 400.0, 255.0))
}

/// Shows an overlay as the only canvas control of a canvas and returns its output.
fn on_canvas(ui: &mut Ui, mut control: impl FnMut(&mut Ui, Rect) -> CropOverlayOutput) -> StoryOutput {
    crate::canvas::testing::canvas_story::on_canvas(ui, |ui, at, _| StoryOutput::from(control(ui, at).edit))
}

/// Drags the top left corner towards the middle of the crop by twice the drag threshold.
fn drag_top_left(fixture: &mut Fixture) {
    let Some(id) = fixture.root_id() else { return };
    let Some(grip) = fixture.part(id, FRAME.at(BoxPart::Corner(0).index())) else { return };
    let travel = 2.0 * crate::testing::input::max_click_dist(fixture.ctx());
    fixture.drag(&grip, Vec2::new(travel, 0.0));
}

#[story(CropOverlay<'static>, covers = [Default], drive = Drive::Custom(drag_top_left))]
fn crop_rect(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
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
    on_canvas(ui, |ui, at| CropOverlay::new("Crop", Bind::bound(&mut *rect)).ratio(16.0 / 9.0).show_at(ui, at))
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
    on_canvas(ui, |ui, at| CropOverlay::new("Crop", Bind::bound(&mut *rect)).straighten().show_at(ui, at))
}

#[story(CropOverlay<'static>, covers = [Default])]
fn image_mode(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let rect = data.value("crop", crop());
    on_canvas(ui, |ui, at| CropOverlay::new("Crop", Bind::bound(&mut *rect)).image_mode().auto_center().show_at(ui, at))
}
