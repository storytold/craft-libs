//! Stories of `CommandPalette`: PhotoCraft's menu commands and tools, with no command run yet and with three run.

use super::CommandPalette;
use crate::contract::Bind;
use crate::menu::search::HelpEntry;
use crate::runtime::commands::{CommandRef, EditCommands};
use crate::testing::{Drive, StoryData, StoryOutput, story};
use egui::Ui;
use std::sync::LazyLock;

/// The PhotoCraft commands of the stories that the library's edit commands do not cover.
#[derive(Clone, Copy, Debug, PartialEq, Eq, craft_ui_macros::CommandSet)]
#[command(prefix = "paletteStory")]
enum PhotoCraftCommand {
    #[command(label = "Search", icon = "search", default = "Cmd+F")]
    Search,
    #[command(label = "Levels…")]
    Levels,
    #[command(label = "Curves…")]
    Curves,
    #[command(label = "Hue/Saturation…")]
    HueSaturation,
    #[command(label = "Deselect")]
    Deselect,
    #[command(label = "Gaussian Blur…")]
    GaussianBlur,
    #[command(label = "Brush", kind = "tool", group = "tools")]
    Brush,
    #[command(label = "Eraser", kind = "tool", group = "tools")]
    Eraser,
    #[command(label = "Move", kind = "tool", group = "tools")]
    Move,
}

/// PhotoCraft's menu commands with their menu paths, Redo disabled, then its tools.
static ENTRIES: LazyLock<Vec<HelpEntry>> = LazyLock::new(|| {
    vec![
        HelpEntry::new(PhotoCraftCommand::Search, "Search").with_path(["Edit"]),
        HelpEntry::new(EditCommands::Redo, "Redo").with_path(["Edit"]),
        HelpEntry::new(PhotoCraftCommand::Levels, "Levels…").with_path(["Image", "Adjustments"]),
        HelpEntry::new(PhotoCraftCommand::Curves, "Curves…").with_path(["Image", "Adjustments"]),
        HelpEntry::new(PhotoCraftCommand::HueSaturation, "Hue/Saturation…").with_path(["Image", "Adjustments"]),
        HelpEntry::new(EditCommands::SelectAll, "All").with_path(["Select"]),
        HelpEntry::new(PhotoCraftCommand::Deselect, "Deselect").with_path(["Select"]),
        HelpEntry::new(PhotoCraftCommand::GaussianBlur, "Gaussian Blur…").with_path(["Filter", "Blur"]),
        HelpEntry::new(PhotoCraftCommand::Brush, "Brush").with_path(["Tool"]),
        HelpEntry::new(PhotoCraftCommand::Eraser, "Eraser").with_path(["Tool"]),
        HelpEntry::new(PhotoCraftCommand::Move, "Move").with_path(["Tool"]),
    ]
});

/// The open palette over [`ENTRIES`], starting with `recents` as the commands last run, and the last command picked
/// as the readout.
fn palette(ui: &mut Ui, data: &mut StoryData, recents: Vec<CommandRef>) -> StoryOutput {
    // The palette binds two story values; it edits a copy of `open`, which goes back after the pass.
    let mut open = *data.value("open", true);
    let out = CommandPalette::new("Command palette", ENTRIES.as_slice(), Bind::bound(&mut open))
        .recents(Bind::bound(data.value("recents", recents)))
        .show(ui);
    *data.value("open", true) = open;
    let last = data.value("last", None::<String>);
    if let Some(picked) = &out.picked
        && let Some(entry) = ENTRIES.iter().find(|entry| entry.command() == picked)
    {
        *last = Some(entry.label().as_str().to_owned());
    }
    let readout = format!("Last run: {}", last.as_deref().unwrap_or("none"));
    StoryOutput::from(out).readout(readout)
}

#[story(CommandPalette<'static>, covers = [], drive = Drive::Never, title = "PhotoCraft")]
fn command_palette(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    palette(ui, data, Vec::new())
}

#[story(CommandPalette<'static>, covers = [], drive = Drive::Never, title = "Recent commands")]
fn command_palette_recents(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let recents = [PhotoCraftCommand::Levels, PhotoCraftCommand::Brush, PhotoCraftCommand::Search].map(CommandRef::from).to_vec();
    palette(ui, data, recents)
}
