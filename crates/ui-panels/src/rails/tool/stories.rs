//! Stories of `ToolRail`, and the commands its stories and the flyout's use as tools.

use super::{LastUsed, RailTool, ToolRail, ToolSlot};
use crate::testing::{Drive, StoryData, StoryOutput, story};
use egui::Ui;

/// The tools of the stories.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, craft_ui_macros::CommandSet)]
#[command(prefix = "railStory", story)]
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

/// The height of the story's short room: lower than its rows at one column.
const SHORT_ROOM: f32 = 120.0;

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

/// The rail in a room as wide as its grid at `columns` columns: the stage a dock column of that many steps gives it. A
/// short room is lower than the rail's rows, so they scroll under the overlay bar.
#[story(ToolRail<'static, u8>, covers = [Default], drive = Drive::Never)]
fn tool_rail(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let columns = *data.ranged("columns", 1_u8, 1.0..=4.0);
    let short = *data.choice("room", false, &[("Tall", false), ("Short", true)]);
    let hidden =
        data.choice("hidden", Vec::<u8>::new(), &[("None", Vec::new()), ("Pencil", vec![2]), ("Stamp", vec![4]), ("Eraser", vec![3])]).clone();
    let slots = slots();
    let last = LastUsed::default();
    let width = ToolRail::<u8>::dock_grid(ui.ctx(), slots.len() + 1).width(u16::from(columns));
    // design: allow[contract/memory](the story gives the rail the width of its chosen columns, as a dock column does)
    let out = ui
        .scope(|ui| {
            ui.set_max_width(width);
            if short {
                ui.set_max_height(SHORT_ROOM);
            }
            ToolRail::new("Tools", &slots, &BRUSH, &last).hidden(&hidden).edit_toolbar("Edit Toolbar").show_rail(ui, |_, _| {})
        })
        .inner;
    let note = data.note("note", "Nothing yet".to_owned());
    if let Some(tool) = out.select {
        *note = format!("selected {tool}");
    } else if let Some(tool) = out.double_clicked {
        *note = format!("double clicked {tool}");
    } else if out.edit_toolbar.as_ref().is_some_and(crate::contract::clicked) {
        *note = "edit toolbar".to_owned();
    }
    let flyout = out.flyout.map_or_else(|| "closed".to_owned(), |slot| format!("of slot {slot} open"));
    let readout = format!("{note}; flyout {flyout}");
    StoryOutput::from(out).readout(readout)
}
