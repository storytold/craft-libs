//! Stories of the "+" buttons: an artboard on a pasteboard with the buttons around it, one side kept clear by a
//! neighbour's rect, and the connected-shape form.

use super::AddBesideHandles;
use craft_behavior::CanvasView;
use craft_behavior::kurbo::{Point, Rect as DocRect};
use craft_ui::canvas::{Canvas, PageFrame, Pasteboard};
use craft_ui::contract::{Show as _, ShowContainer as _};
use craft_ui::testing::{Drive, StoryData, StoryOutput, story};
use egui::{Rect, Ui};

/// The artboard the buttons surround, in document units: in the middle of the story document.
fn artboard() -> DocRect {
    DocRect::new(220.0, 130.0, 420.0, 270.0)
}

/// The view of the canvas the story shows: the story document at 100 %, centred.
fn view(ui: &Ui, rect: Rect) -> CanvasView {
    craft_ui::testing::canvas_story::StoryDocument::STANDARD.view(rect, ui.pixels_per_point(), 100.0)
}

/// Shows the artboard with `handles` over it on a canvas.
fn on_canvas<'a>(ui: &mut Ui, rect: Rect, handles: impl Fn(Rect) -> AddBesideHandles<'a>) -> StoryOutput {
    let board = [PageFrame::new(artboard()).name("Artboard 1").active(true)];
    let view = view(ui, rect);
    let shown = Canvas::new("Document").view(view).show_at(ui, rect, |ui, body| {
        craft_ui::testing::canvas_story::show_document(ui, body.rect, &craft_ui::testing::canvas_story::StoryDocument::STANDARD);
        let name_clicked = Pasteboard::new("Pasteboard", &board).show_at(ui, body.rect).name_clicked.is_some();
        (handles(body.rect).show_at(ui, body.rect), name_clicked)
    });
    // The artboard's name is a button too: its click is a click of the story.
    shown.inner.map_or_else(StoryOutput::default, |(output, name_clicked)| {
        let mut facts = StoryOutput::from(output);
        facts.clicked |= name_clicked;
        facts
    })
}

#[story(AddBesideHandles<'static>, covers = [Default], drive = Drive::Never)]
fn add_beside(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    let rect = craft_ui::testing::stage(ui);
    let view = view(ui, rect);
    let neighbour = Rect::from_two_pos(
        view.to_screen(Point::new(artboard().x1 + 16.0, artboard().y0)),
        view.to_screen(Point::new(artboard().x1 + 80.0, artboard().y1)),
    );
    let clear = [neighbour];
    on_canvas(ui, rect, |_| AddBesideHandles::new("Add beside", artboard()).clear_of(&clear))
}

#[story(AddBesideHandles<'static>, covers = [Default], drive = Drive::Never)]
fn add_beside_connecting(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    let rect = craft_ui::testing::stage(ui);
    on_canvas(ui, rect, |_| AddBesideHandles::new("Add beside", artboard()).connecting())
}
