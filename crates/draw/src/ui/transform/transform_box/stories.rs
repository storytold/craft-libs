//! Stories of the transform box: Free Transform with its reference point, the Move tool's controls and Distort.

use super::{BOX, RotateZone, TransformBox};
use craft_behavior::geometry::quad::{BoxPart, BoxRules, BoxState, TransformMode};
use craft_behavior::kurbo::Rect as DocRect;
use craft_ui::canvas::Canvas;
use craft_ui::contract::{Bind, Show as _, ShowContainer as _};
use craft_ui::testing::{Drive, Grip, StoryData, StoryOutput, story};
use egui::{Ui, vec2};

/// The picture before any transform, in document units.
const PICTURE: DocRect = DocRect::new(200.0, 120.0, 440.0, 280.0);
/// The layer the box transforms, shown where the box is.
const LAYER: craft_ui::testing::canvas_story::FlatPicture =
    craft_ui::testing::canvas_story::FlatPicture::new(egui::Color32::from_rgba_premultiplied(60, 112, 155, 220));

/// Shows the box `configure` builds over the story's box as the only canvas control of a canvas.
fn on_canvas(ui: &mut Ui, data: &mut StoryData, configure: impl for<'a> Fn(TransformBox<'a>) -> TransformBox<'a>) -> StoryOutput {
    let state = data.value("box", BoxState::from_rect(PICTURE));
    let room = craft_ui::testing::stage(ui);
    let view = craft_ui::testing::canvas_story::story_view(room, ui.pixels_per_point(), PICTURE.center(), 100.0);
    let shown = Canvas::new("Document").view(view).show_at(ui, room, |ui, body| {
        craft_ui::testing::canvas_story::show_document(ui, body.rect, &craft_ui::testing::canvas_story::StoryDocument::STANDARD);
        craft_ui::testing::canvas_story::show_quad(ui, body.rect, "Layer", state.quad, &LAYER);
        configure(TransformBox::new("Free Transform", Bind::bound(&mut *state), PICTURE, 0.0)).show_at(ui, body.rect)
    });
    shown.inner.map(|output| StoryOutput::from(output.edit)).unwrap_or_default()
}

#[story(TransformBox<'static>, covers = [Default], drive = Drive::Drag(Grip::part(BOX.at(BoxPart::Corner(2).index())).toward(vec2(1.0, 1.0))))]
fn free_transform(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    on_canvas(ui, data, |control| control.reference_point(true))
}

#[story(TransformBox<'static>, covers = [Default])]
fn move_tool_controls(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    on_canvas(ui, data, |control| control.rotate_zone(RotateZone::Band).inside_moves(false).reference_point(false))
}

#[story(TransformBox<'static>, covers = [Default])]
fn distort(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    on_canvas(ui, data, |control| control.rules(BoxRules::in_mode(TransformMode::Distort)))
}
