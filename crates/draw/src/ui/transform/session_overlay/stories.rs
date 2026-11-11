//! Stories of the transform overlay: the open session's box in the transform phase.

use super::TransformOverlay;
use crate::ui::transform::{TransformSession, TransformSettings};
use craft_behavior::kurbo::Rect as DocRect;
use craft_ui::canvas::Canvas;
use craft_ui::contract::ShowContainer as _;
use craft_ui::testing::{StoryData, StoryOutput, story};
use egui::Ui;

/// The layer's source frame, in document units.
const FRAME: DocRect = DocRect::new(200.0, 120.0, 440.0, 280.0);

#[story(TransformOverlay<'static, ()>, covers = [Default])]
fn transform_phase(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    let mut session = TransformSession::<()>::new(FRAME);
    let settings = TransformSettings::default();
    let stage = craft_ui::testing::stage(ui);
    let document = &craft_ui::testing::canvas_story::StoryDocument::STANDARD;
    let shown = Canvas::new("Document").view(document.view(stage, ui.pixels_per_point(), 100.0)).show_at(ui, stage, |ui, body| {
        craft_ui::testing::canvas_story::show_document(ui, body.rect, document);
        TransformOverlay::new("Free Transform", &session, &settings).show_at(ui, body.rect)
    });
    if let Some(output) = shown.inner {
        for edit in output.edits {
            session.apply(edit, 0.0);
        }
    }
    StoryOutput::default()
}
