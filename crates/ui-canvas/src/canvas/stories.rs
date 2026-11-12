//! Stories of the canvas family.

use super::{Canvas, CropOverlay};
use crate::buttons::Button;
use crate::canvas::testing::canvas_story::{StoryDocument, show_document};
use crate::contract::{Bind, BodyCx, CanvasGesture, Gap, Sampler as _, Show as _, ShowContainer as _, clicked};
use crate::dialog::Dialog;
use crate::display::Text;
use crate::layout_components::Stack;
use crate::testing::{Drive, StoryData, StoryOutput, stage, story};
use craft_behavior::geometry::CropBox;
use craft_behavior::kurbo::{Point, Rect as DocRect};
use egui::Ui;

/// The zoomed story's zoom, in percent of one device pixel per document pixel.
const ZOOMED: f64 = 200.0;

fn crop() -> CropBox {
    CropBox::upright(DocRect::new(240.0, 145.0, 400.0, 255.0))
}

#[story(Canvas, covers = [Default])]
fn crop_tool(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    let room = stage(ui);
    let view = StoryDocument::STANDARD.view(room, ui.pixels_per_point(), 100.0);
    let shown = Canvas::new("Document").view(view).show_at(ui, room, |ui, body| {
        show_document(ui, body.rect, &StoryDocument::STANDARD);
        CropOverlay::new("Crop", Bind::given(crop())).show_at(ui, body.rect)
    });
    shown.output.into()
}

#[story(Canvas, covers = [Default])]
fn zoomed(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    let room = stage(ui);
    let view = StoryDocument::STANDARD.view(room, ui.pixels_per_point(), ZOOMED);
    let shown = Canvas::new("Document").view(view).show_at(ui, room, |ui, body| {
        show_document(ui, body.rect, &StoryDocument::STANDARD);
        CropOverlay::new("Crop", Bind::given(crop())).show_at(ui, body.rect)
    });
    shown.output.into()
}

#[story(Canvas, covers = [Default])]
fn freehand(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    let room = stage(ui);
    Canvas::new("Painting")
        .view(StoryDocument::STANDARD.view(room, ui.pixels_per_point(), 100.0))
        .show_at(ui, room, |ui, body| show_document(ui, body.rect, &StoryDocument::STANDARD))
        .output
        .into()
}

/// The canvas under an open modal dialog that keeps it live: the wheel scrolls and zooms it, the middle button and
/// Space pan it, and a click on the image picks a point into the dialog. Every other input stays the dialog's.
#[story(Canvas, covers = [], drive = Drive::Never)]
fn under_a_dialog(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let room = stage(ui);
    let pixels_per_point = ui.pixels_per_point();
    let mut open = *data.value("open", false);
    let mut picked = *data.value::<Option<Point>>("picked", None);
    let row = |ui: &mut Ui, _: &mut BodyCx<'_>| {
        if clicked(&Button::new("Pick a Point…").show(ui)) {
            open = true;
        }
    };
    let stage = Stack::new().fill().footer(row).footer_gap(Gap::Stack).show_at(ui, room, |ui, stack| {
        let view = data.value("view", StoryDocument::STANDARD.view(stack.rect, pixels_per_point, 100.0));
        Canvas::new("Document")
            .view(Bind::bound(view))
            .show_at(ui, stack.rect, |ui, body| show_document(ui, body.rect, &StoryDocument::STANDARD))
            .output
    });
    let Some(canvas) = stage.inner else { return StoryOutput::default() };
    // The press the dialog lets through reaches the canvas's background, where the app's tool would take it.
    if open && let (Some(at), Some(space)) = (canvas.click_at(), canvas.space) {
        picked = Some(space.view.to_doc(at));
    }
    let id = canvas.response.id;
    Dialog::new("Pick a Point").opened(Bind::bound(&mut open)).show(ui, |ui, cx| {
        cx.keep_view_open(id);
        cx.pass_canvas_gesture(id, CanvasGesture::PRIMARY);
        let text = picked.map_or_else(|| "Click the image to pick a point.".to_owned(), |at| format!("Picked {:.0}, {:.0}", at.x, at.y));
        Text::new(text).show(ui);
    });
    *data.value("open", false) = open;
    *data.value::<Option<Point>>("picked", None) = picked;
    canvas.into()
}
