//! Stories of the canvas family.

use super::{Canvas, CropOverlay};
use crate::canvas::testing::canvas_story::{StoryDocument, show_document};
use crate::contract::{Bind, Show as _, ShowContainer as _};
use crate::testing::{StoryData, StoryOutput, story};
use craft_behavior::geometry::CropBox;
use craft_behavior::kurbo::Rect as DocRect;
use egui::Ui;

/// The zoomed story's zoom, in percent of one device pixel per document pixel.
const ZOOMED: f64 = 200.0;

fn crop() -> CropBox {
    CropBox::upright(DocRect::new(240.0, 145.0, 400.0, 255.0))
}

#[story(Canvas, covers = [Default])]
fn crop_tool(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    let room = crate::testing::stage(ui);
    let view = StoryDocument::STANDARD.view(room, ui.pixels_per_point(), 100.0);
    let shown = Canvas::new("Document").view(view).show_at(ui, room, |ui, body| {
        show_document(ui, body.rect, &StoryDocument::STANDARD);
        CropOverlay::new("Crop", Bind::given(crop())).show_at(ui, body.rect)
    });
    shown.output.into()
}

#[story(Canvas, covers = [Default])]
fn zoomed(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    let room = crate::testing::stage(ui);
    let view = StoryDocument::STANDARD.view(room, ui.pixels_per_point(), ZOOMED);
    let shown = Canvas::new("Document").view(view).show_at(ui, room, |ui, body| {
        show_document(ui, body.rect, &StoryDocument::STANDARD);
        CropOverlay::new("Crop", Bind::given(crop())).show_at(ui, body.rect)
    });
    shown.output.into()
}

#[story(Canvas, covers = [Default])]
fn freehand(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    let room = crate::testing::stage(ui);
    Canvas::new("Painting")
        .view(StoryDocument::STANDARD.view(room, ui.pixels_per_point(), 100.0))
        .show_at(ui, room, |ui, body| show_document(ui, body.rect, &StoryDocument::STANDARD))
        .output
        .into()
}
