//! Stories of `ToolRail`, and the commands its stories and the flyout's use as tools.

use super::{LastUsed, RailDensity, RailTool, ToolRail, ToolRailOutput, ToolSlot, ToolTips};
use crate::buttons::ActiveLook;
use crate::contract::{MemoProps, Show as _, Tracked};
use crate::display::{Shade, Text};
use crate::runtime::env::Env;
use crate::style::TypeRole;
use crate::testing::{Drive, StoryData, StoryOutput, story};
use craft_behavior::Flow;
use egui::{Ui, UiBuilder};

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

/// The id of the eraser tool.
const ERASER: u8 = 3;

/// Tools with the given ids, each running the story command of its id.
pub(crate) fn tools(ids: &[u8]) -> Vec<RailTool<u8>> {
    const ALL: [StoryTools; 5] = [StoryTools::Move, StoryTools::Brush, StoryTools::Pencil, StoryTools::Eraser, StoryTools::Stamp];
    ids.iter().map(|id| RailTool::new(*id, ALL[usize::from(*id) % ALL.len()])).collect()
}

fn slots() -> Vec<ToolSlot<u8>> {
    vec![
        ToolSlot::new(0, tools(&[0])),
        ToolSlot::new(1, tools(&[BRUSH, BRUSH + 1])),
        ToolSlot::new(2, tools(&[ERASER])),
        ToolSlot::new(3, tools(&[4])),
    ]
}

/// The story slots in two captioned groups: Move alone, then the painting tools.
fn captioned_slots() -> Vec<ToolSlot<u8>> {
    vec![
        ToolSlot::ending_group(0, tools(&[0])).captioned("Select"),
        ToolSlot::new(1, tools(&[BRUSH, BRUSH + 1])).captioned("Paint and Retouch"),
        ToolSlot::new(2, tools(&[ERASER])),
        ToolSlot::new(3, tools(&[4])),
    ]
}

/// Shows `rail` in a room as wide as its grid of `cells` cells at `columns` columns, the stage a dock column of that
/// many steps gives it, at the reading start and at most `height` high.
fn in_columns(ui: &mut Ui, columns: u8, cells: usize, height: Option<f32>, rail: ToolRail<'_, u8>) -> ToolRailOutput<u8> {
    let width = rail.grid(ui, cells).width(u16::from(columns));
    let room = Flow::new(ui.available_rect_before_wrap(), Env::of(ui).direction.is_rtl()).span(0.0, width);
    ui.scope_builder(UiBuilder::new().max_rect(room), |ui| {
        if let Some(height) = height {
            ui.set_max_height(height);
        }
        rail.show_rail(ui, |_, _| {})
    })
    .inner
}

/// The rail in a room as wide as its grid at `columns` columns. A short room is lower than the rail's rows, so they
/// scroll under the overlay bar.
#[story(ToolRail<'static, u8>, covers = [Default], drive = Drive::Never)]
fn tool_rail(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let columns = *data.ranged("columns", 1_u8, 1.0..=4.0);
    let short = *data.choice("room", false, &[("Tall", false), ("Short", true)]);
    let hidden =
        data.choice("hidden", Vec::<u8>::new(), &[("None", Vec::new()), ("Pencil", vec![2]), ("Stamp", vec![4]), ("Eraser", vec![3])]).clone();
    let hidden = Tracked::new(hidden);
    let slots = Tracked::new(slots());
    let last = Tracked::new(LastUsed::default());
    let rail = ToolRail::new("Tools", &slots, &BRUSH, &last).hidden(&hidden).edit_toolbar("Edit Toolbar");
    let out = in_columns(ui, columns, slots.len() + 1, short.then_some(SHORT_ROOM), rail);
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

/// The rail with its groups' captions in one or two columns: each caption heads its row group, cut at the rail's
/// width.
#[story(ToolRail<'static, u8>, covers = [], drive = Drive::Never)]
fn tool_rail_captions(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let columns = *data.ranged("columns", 1_u8, 1.0..=2.0);
    let slots = Tracked::new(captioned_slots());
    let last = Tracked::new(LastUsed::default());
    in_columns(ui, columns, slots.len(), None, ToolRail::new("Tools", &slots, &0, &last).captions()).into()
}

/// The rail at each density in one or two columns: a dense rail's buttons are a control high, and its columns narrow
/// with them.
#[story(ToolRail<'static, u8>, covers = [], drive = Drive::Never)]
fn tool_rail_density(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let columns = *data.ranged("columns", 1_u8, 1.0..=2.0);
    let density = *data.choice("density", RailDensity::Dense, &[("Regular", RailDensity::Regular), ("Dense", RailDensity::Dense)]);
    let slots = Tracked::new(slots());
    let last = Tracked::new(LastUsed::default());
    in_columns(ui, columns, slots.len(), None, ToolRail::new("Tools", &slots, &BRUSH, &last).density(density)).into()
}

/// The current tool (the brush) in each active look: the accent's soft fill and edge, or the pressed fill with no
/// edge.
#[story(ToolRail<'static, u8>, covers = [], drive = Drive::Never)]
fn tool_rail_active_look(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let look = *data.choice("active look", ActiveLook::Pressed, &[("Accent", ActiveLook::Accent), ("Pressed", ActiveLook::Pressed)]);
    let slots = Tracked::new(slots());
    let last = Tracked::new(LastUsed::default());
    in_columns(ui, 1, slots.len(), None, ToolRail::new("Tools", &slots, &BRUSH, &last).active_look(look)).into()
}

/// The tips of the brush and the eraser: the tool's name over what it does.
#[derive(MemoProps)]
struct StoryTips;

impl ToolTips<u8> for StoryTips {
    fn has(&self, tool: &u8) -> bool {
        matches!(*tool, BRUSH | ERASER)
    }

    fn show(&self, tool: &u8, ui: &mut Ui) {
        let (name, does) = if *tool == BRUSH { ("Brush", "Paints with the foreground colour.") } else { ("Eraser", "Erases to transparency.") };
        Text::new(name).show(ui);
        Text::new(does).role(TypeRole::Caption).shade(Shade::Muted).show(ui);
    }
}

/// The rail whose brush and eraser show a tip in place of their command tooltip, on their buttons and in the brush
/// slot's flyout.
#[story(ToolRail<'static, u8>, covers = [], drive = Drive::Never)]
fn tool_rail_tips(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    let slots = Tracked::new(slots());
    let last = Tracked::new(LastUsed::default());
    in_columns(ui, 1, slots.len(), None, ToolRail::new("Tools", &slots, &0, &last).tips(&StoryTips)).into()
}

/// The pencil marked while the brush slot shows the brush: the reveal of the first pass opens the slot's flyout, whose
/// pencil row wears the mark; closed again, the slot's button wears it.
#[story(ToolRail<'static, u8>, covers = [], drive = Drive::Never)]
fn tool_rail_marked_in_a_closed_group(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let slots = Tracked::new(slots());
    let last = Tracked::new(LastUsed::default());
    let pencil = BRUSH + 1;
    // The reveal is a request of one pass.
    let revealed = std::mem::replace(data.note("revealed", false), true);
    let mut rail = ToolRail::new("Tools", &slots, &0, &last).mark(&pencil);
    if !revealed {
        rail = rail.reveal(&pencil);
    }
    in_columns(ui, 1, slots.len(), None, rail).into()
}
