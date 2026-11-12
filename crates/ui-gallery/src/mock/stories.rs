//! The stories of the mock editor window and its Move tool.

use super::model::Document;
use super::panels::show_layers;
use super::tools::MoveTool;
use super::{EditorMock, MockState};
use craft_behavior::geometry::Directions;
use craft_ui::contract::Bind;
use craft_ui::runtime::env::Env;
use craft_ui::shell::ScreenMode;
use craft_ui::testing::canvas_story::StoryDocument;
use craft_ui::testing::{Drive, StoryData, StoryOutput, ToolStory, room_height, stage_fitting, story, tool_story};
use egui::Ui;

craft_ui::words! {
    /// The readout of the Move tool's story.
    #[non_exhaustive]
    pub(crate) struct MoveStoryWords {
        /// No move runs.
        idle: "moveStory.idle" = "Drag to move the current layer";
        /// A move runs: `{lock}` is the direction Shift holds it to.
        moving: template "moveStory.moving" = "Lock: {lock}";
        /// A move of a copy runs: `{lock}` is the direction Shift holds it to.
        copying: template "moveStory.copying" = "Lock: {lock} · Copy";
        /// The move is free.
        free: "moveStory.free" = "Free";
        /// Shift holds the move level.
        horizontal: "moveStory.horizontal" = "Horizontal";
        /// Shift holds the move upright.
        vertical: "moveStory.vertical" = "Vertical";
        /// Shift holds the move on a diagonal.
        diagonal: "moveStory.diagonal" = "Diagonal";
    }
}

/// The view's zoom, in percent: the mock page fits a card.
const ZOOM: f64 = 50.0;

#[story(EditorMock<'static>, covers = [])]
fn editor_window(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let state = data.value("mock", MockState::default());
    let height = room_height(ui);
    let room = stage_fitting(ui, |_| height);
    EditorMock::new("Editor", state).show_at(ui, room).into()
}

/// The window in full screen: the body only. F cycles the screen modes, Tab and the edges reveal the bars.
#[story(EditorMock<'static>, covers = [], drive = Drive::Never)]
fn editor_window_full_screen(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let mut mode = *data.value("mode", ScreenMode::FullScreen);
    let height = room_height(ui);
    let room = stage_fitting(ui, |_| height);
    let state = data.value("mock", MockState::default());
    let shown = EditorMock::new("Editor", state).screen_mode(Bind::bound(&mut mode)).show_at(ui, room);
    *data.value("mode", ScreenMode::FullScreen) = mode;
    shown.into()
}

#[story(MoveTool<'static>, covers = [], drive = Drive::Never)]
fn move_tool(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let directions = *data.choice("directions", Directions::Four, &[("Layers", Directions::Four), ("Pixels", Directions::Eight)]);
    let words = Env::of(ui).words::<MoveStoryWords>();
    let document = Document::default();
    let size = document.pixel_size();
    let page = StoryDocument::new(size.width, size.height);
    let story = ToolStory::new("Document", &page).view(page.centre(), ZOOM);
    tool_story(
        ui,
        data,
        story,
        || (),
        document,
        |ui, at, _, _, document| show_layers(ui, at, &document.layers, size),
        |run| {
            let _ = run.host.run(&mut MoveTool::new(run.document, directions));
            StoryOutput::default().readout(readout(&words, run.document))
        },
    )
}

/// The lock and the copy flag of the move in progress.
fn readout(words: &MoveStoryWords, document: &Document) -> String {
    let Some(moving) = &document.moving else { return words.idle.as_str().to_owned() };
    let lock = match moving.gesture.locked() {
        None => &words.free,
        Some(d) if d.y == 0.0 => &words.horizontal,
        Some(d) if d.x == 0.0 => &words.vertical,
        Some(_) => &words.diagonal,
    };
    let template = if moving.gesture.value().copy { &words.copying } else { &words.moving };
    craft_i18n::fmt(template.as_str(), &[("lock", lock.as_str())])
}
