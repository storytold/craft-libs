//! Stories of the transform overlay: the open session's box in the transform phase.

use super::TransformOverlay;
use crate::ui::transform::{TransformSession, TransformSettings};
use craft_behavior::kurbo::{Line, Rect as DocRect};
use craft_ui::canvas::{Canvas, RasterSource as _};
use craft_ui::contract::ShowContainer as _;
use craft_ui::testing::canvas_story::{OUTLINE, StoryDocument, StoryPicture, StoryShape, accent, paint_shapes, show_document};
use craft_ui::testing::{StoryData, StoryOutput, story};
use egui::{Color32, Ui};

/// The layer's source frame, in document units: the story shows the layer as a box in the accent with a diagonal.
const FRAME: DocRect = DocRect::new(200.0, 120.0, 440.0, 280.0);

#[story(TransformOverlay<'static, ()>, covers = [Default])]
fn transform_phase(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    let mut session = TransformSession::<()>::new(FRAME);
    let settings = TransformSettings::default();
    let stage = craft_ui::testing::stage(ui);
    let document = StoryDocument::STANDARD;
    let line = accent(ui, 1.0);
    let layer = [
        StoryShape::filled(&FRAME, accent(ui, 0.5)).outlined(OUTLINE, line),
        StoryShape::line(&Line::new(FRAME.origin(), (FRAME.x1, FRAME.y1)), OUTLINE, line),
    ];
    let size = document.size();
    let picture = StoryPicture::new(size.width, size.height, move |p| paint_shapes(&layer, Color32::TRANSPARENT, p));
    let shown = Canvas::new("Document").view(document.view(stage, ui.pixels_per_point(), 100.0)).show_at(ui, stage, |ui, body| {
        show_document(ui, body.rect, &picture);
        TransformOverlay::new("Free Transform", &session, &settings).show_at(ui, body.rect)
    });
    if let Some(output) = shown.inner {
        for edit in output.edits {
            session.apply(edit, 0.0);
        }
    }
    StoryOutput::default()
}
