//! Stories of `ToolRail`, and the commands its stories and the flyout's use as tools.

use super::{LastUsed, RailTool, ToolRail, ToolSlot};
use crate::testing::{Drive, StoryData, StoryOutput, story};
use egui::Ui;
use std::num::NonZeroU8;

/// The tools of the stories.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, craft_ui_macros::CommandSet)]
#[command(prefix = "railStory")]
#[non_exhaustive]
pub(crate) enum StoryTools {
    /// Move.
    #[command(label = "Move", icon = "move", default = "V")]
    Move,
    /// Brush.
    #[command(label = "Brush", icon = "brush", default = "B")]
    Brush,
    /// Pencil.
    #[command(label = "Pencil", icon = "pencil")]
    Pencil,
    /// Eraser.
    #[command(label = "Eraser", icon = "eraser", default = "E")]
    Eraser,
    /// Stamp.
    #[command(label = "Clone Stamp", icon = "stamp", default = "S")]
    Stamp,
}

/// The id of the brush tool; the tools that follow it are the pencil and the eraser.
pub(crate) const BRUSH: u8 = 1;

/// Tools with the given ids, each running the story command of its id.
pub(crate) fn tools(ids: &[u8]) -> Vec<RailTool<u8>> {
    const ALL: [StoryTools; 5] = [StoryTools::Move, StoryTools::Brush, StoryTools::Pencil, StoryTools::Eraser, StoryTools::Stamp];
    ids.iter().map(|id| RailTool::new(*id, ALL[usize::from(*id) % ALL.len()])).collect()
}

fn slots() -> Vec<ToolSlot<u8>> {
    vec![ToolSlot::new(0, tools(&[0])), ToolSlot::new(1, tools(&[BRUSH, BRUSH + 1])), ToolSlot::new(2, tools(&[3])), ToolSlot::new(3, tools(&[4]))]
}

#[story(ToolRail<'static, u8>, covers = [Default], drive = Drive::Never)]
fn tool_rail(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let columns = *data.ranged("columns", 1_u8, 1.0..=2.0);
    let hidden =
        data.choice("hidden", Vec::<u8>::new(), &[("None", Vec::new()), ("Pencil", vec![2]), ("Stamp", vec![4]), ("Eraser", vec![3])]).clone();
    let slots = slots();
    let last = LastUsed::default();
    let out = ToolRail::new("Tools", &slots, &BRUSH, &last)
        .columns(NonZeroU8::new(columns.max(1)).unwrap_or(NonZeroU8::MIN))
        .hidden(&hidden)
        .column_toggle("Toolbar")
        .edit_toolbar("Edit Toolbar")
        .show_rail(ui, |_, _| {});
    out.into()
}
