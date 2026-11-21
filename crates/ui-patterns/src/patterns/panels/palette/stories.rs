//! Stories of `CommandPalette`: PhotoCraft's menu commands and tools, with no command run yet and with three run.

use super::{CommandPalette, PaletteFace};
use crate::contract::Bind;
use crate::menu::search::HelpEntry;
use crate::runtime::commands::{CommandRef, CommandState, CommandStates, EditCommands};
use crate::runtime::env::{Label, WordTemplate};
use crate::testing::{Drive, StoryData, StoryOutput, story};
use egui::Ui;
use std::sync::LazyLock;

/// The PhotoCraft commands of the stories that the library's edit commands do not cover.
#[derive(Clone, Copy, Debug, PartialEq, Eq, craft_ui_macros::CommandSet)]
#[command(prefix = "paletteStory", story)]
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
    configured(ui, data, recents, true, |palette| palette)
}

/// The palette over [`ENTRIES`] as `configure` sets it, open from the start when `open_at_start`; the last command
/// picked and the last query the fallback row handed back are the readout.
fn configured(
    ui: &mut Ui,
    data: &mut StoryData,
    recents: Vec<CommandRef>,
    open_at_start: bool,
    configure: fn(CommandPalette<'_>) -> CommandPalette<'_>,
) -> StoryOutput {
    // The palette binds two story values; it edits a copy of `open`, which goes back after the pass.
    let mut open = *data.value("open", open_at_start);
    let palette = CommandPalette::new("Command palette", ENTRIES.as_slice(), Bind::bound(&mut open)).recents(data.bind("recents", recents));
    let out = configure(palette).show(ui);
    *data.value("open", open_at_start) = open;
    let last = data.value("last", None::<String>);
    if let Some(picked) = &out.picked
        && let Some(entry) = ENTRIES.iter().find(|entry| entry.command() == picked)
    {
        *last = Some(entry.label().as_str().to_owned());
    }
    let mut readout = format!("Last run: {}", last.as_deref().unwrap_or("none"));
    let searched = data.value("searched", None::<String>);
    if let Some(query) = &out.fallback {
        *searched = Some(query.clone());
    }
    if let Some(query) = searched.as_deref() {
        readout.push_str(&format!(". Searched the document for: {query}"));
    }
    StoryOutput::from(out).readout(readout)
}

/// The in-place face with its hint.
fn in_place_face(palette: CommandPalette<'_>) -> CommandPalette<'_> {
    palette.face(PaletteFace::InPlace).placeholder(Label::from("Search commands"))
}

/// Suggested commands for the empty query and the fallback row.
fn with_suggested_and_fallback(palette: CommandPalette<'_>) -> CommandPalette<'_> {
    let suggested = [PhotoCraftCommand::Levels, PhotoCraftCommand::Brush].map(CommandRef::from).to_vec();
    palette.suggested((Label::from("Suggested"), suggested)).fallback(WordTemplate::new("Find “{text}” in the document"))
}

/// The command states the stories run under: Redo is disabled, as the entries list it.
fn commands(states: &mut CommandStates) {
    states.set(EditCommands::Redo, CommandState::new().enabled(false));
}

#[story(CommandPalette<'static>, covers = [], drive = Drive::Never, title = "PhotoCraft", commands = commands)]
fn command_palette(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    palette(ui, data, Vec::new())
}

#[story(CommandPalette<'static>, covers = [], drive = Drive::Never, title = "Recent commands", commands = commands)]
fn command_palette_recents(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let recents = [PhotoCraftCommand::Levels, PhotoCraftCommand::Brush, PhotoCraftCommand::Search].map(CommandRef::from).to_vec();
    palette(ui, data, recents)
}

#[story(CommandPalette<'static>, covers = [], drive = Drive::Never, title = "In place", commands = commands)]
fn command_palette_in_place(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    configured(ui, data, Vec::new(), false, in_place_face)
}

#[story(CommandPalette<'static>, covers = [], drive = Drive::Never, title = "Suggested and fallback", commands = commands)]
fn suggested_and_fallback(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    configured(ui, data, Vec::new(), true, with_suggested_and_fallback)
}
