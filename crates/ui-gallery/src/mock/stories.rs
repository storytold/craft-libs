//! The story of the mock editor window.

use super::tools::MoveTool;
use super::{EditorMock, MockState};
use craft_ui::testing::{Drive, StoryData, StoryOutput, story, tool_story};
use egui::Ui;

#[story(EditorMock<'static>, covers = [])]
fn editor_window(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let state = data.value("mock", MockState::default());
    EditorMock::new("Editor", state).show(ui).into()
}

#[story(MoveTool, covers = [], drive = Drive::Never)]
fn move_tool(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    tool_story(ui, "Document", &mut MoveTool)
}
