//! The story host of canvas tools: a tool is no component, so its story shows it on a canvas over the story document.

use crate::canvas::{Canvas, CanvasTool, ToolHost};
use crate::contract::ShowContainer as _;
use crate::testing::{StoryOutput, stage};
use egui::Ui;

/// Shows `tool` on a canvas named `label` over the [`stage`] of `ui` and the story document, and runs it as the canvas's active tool. The
/// output is the canvas's, so the suite reads the nodes the tool and the canvas left. The story builds the tool
/// anew each pass: it shows the tool's idle state, and a gesture belongs in a trial on [`ToolFixture`](super::tool_fixture::ToolFixture).
///
/// The tool's type registers as a story host with `#[derive(Pattern)]` and `#[pattern(tool)]`.
pub fn tool_story(ui: &mut Ui, label: &'static str, tool: &mut dyn CanvasTool) -> StoryOutput {
    let room = stage(ui);
    let document = &super::canvas_story::StoryDocument::STANDARD;
    let shown = Canvas::new(label)
        .view(document.view(room, ui.pixels_per_point(), 100.0))
        .show_at(ui, room, |ui, body| super::canvas_story::show_document(ui, body.rect, document));
    let _ = ToolHost::new().run(ui, &shown.output, tool);
    shown.output.into()
}
