//! Stories of `CommandPalette`: PhotoCraft's menu commands and tools, with no command run yet and with three run.

use super::{CommandPalette, PaletteFace};
use crate::buttons::Button;
use crate::contract::{Bind, Show as _};
use crate::list::CommandHighlight;
use crate::menu::search::{HelpEntry, SearchIndex, SearchWords, Synonyms};
use crate::runtime::commands::{CommandRef, CommandState, CommandStates, EditCommands};
use crate::runtime::env::{Label, WordTemplate, with_env};
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

/// The tools of [`ENTRIES`] with the toolbox as their home, which a tool in no view has instead of a menu path.
static HOMED: LazyLock<Vec<HelpEntry>> = LazyLock::new(|| {
    ["Brush", "Eraser", "Move"]
        .into_iter()
        .zip([PhotoCraftCommand::Brush, PhotoCraftCommand::Eraser, PhotoCraftCommand::Move])
        .map(|(label, command)| HelpEntry::new(command, label).with_home("Toolbox"))
        .collect()
});

/// More commands than a result list holds, every label with an "a" so one typed letter matches them all.
static MANY: LazyLock<Vec<HelpEntry>> = LazyLock::new(|| {
    (1..=64).map(|n| HelpEntry::new(CommandRef::new(format!("paletteStory.many{n}")), format!("Layer action {n}")).with_path(["Layer"])).collect()
});

/// The open palette over `entries`, starting with `recents` as the commands last run, and the last command picked
/// as the readout.
fn palette(ui: &mut Ui, data: &mut StoryData, entries: &[HelpEntry], recents: Vec<CommandRef>) -> StoryOutput {
    configured(ui, data, entries, recents, true, |palette| palette)
}

/// The palette over `entries` as `configure` sets it, open from the start when `open_at_start`; the last command
/// picked and the last query the fallback row handed back are the readout.
fn configured(
    ui: &mut Ui,
    data: &mut StoryData,
    entries: &[HelpEntry],
    recents: Vec<CommandRef>,
    open_at_start: bool,
    configure: fn(CommandPalette<'_>) -> CommandPalette<'_>,
) -> StoryOutput {
    // The palette binds two story values; it edits a copy of `open`, which goes back after the pass.
    let mut open = *data.value("open", open_at_start);
    let env = crate::runtime::env::Env::of(ui);
    let view = env.view();
    let index = SearchIndex::new(&view, entries, &Synonyms::of(&view.words::<SearchWords>()));
    let palette = CommandPalette::new("Command palette", entries, &index, Bind::bound(&mut open)).recents(data.bind("recents", recents));
    let out = configure(palette).show(ui);
    *data.value("open", open_at_start) = open;
    let last = data.value("last", None::<String>);
    if let Some(picked) = &out.picked
        && let Some(entry) = entries.iter().find(|entry| entry.command() == picked)
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
    palette(ui, data, ENTRIES.as_slice(), Vec::new())
}

#[story(CommandPalette<'static>, covers = [], drive = Drive::Never, title = "Recent commands", commands = commands)]
fn command_palette_recents(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let recents = [PhotoCraftCommand::Levels, PhotoCraftCommand::Brush, PhotoCraftCommand::Search].map(CommandRef::from).to_vec();
    palette(ui, data, ENTRIES.as_slice(), recents)
}

#[story(CommandPalette<'static>, covers = [], drive = Drive::Never, title = "In place", commands = commands)]
fn command_palette_in_place(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    configured(ui, data, ENTRIES.as_slice(), Vec::new(), false, in_place_face)
}

#[story(CommandPalette<'static>, covers = [], drive = Drive::Never, title = "Suggested and fallback", commands = commands)]
fn suggested_and_fallback(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    configured(ui, data, ENTRIES.as_slice(), Vec::new(), true, with_suggested_and_fallback)
}

/// The active row's command (Brush, the newest run) has a button in view, the rail slot: it gets the ring and a box.
#[story(CommandPalette<'static>, covers = [], drive = Drive::Never, title = "Callout", commands = commands)]
fn command_palette_callout(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    Button::command(PhotoCraftCommand::Brush).show(ui);
    palette(ui, data, ENTRIES.as_slice(), vec![PhotoCraftCommand::Brush.into()])
}

/// The active row's command (Eraser) is shown nowhere: the footer names its home.
#[story(CommandPalette<'static>, covers = [], drive = Drive::Never, title = "Hidden home", commands = commands)]
fn command_palette_hidden_home(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    palette(ui, data, HOMED.as_slice(), vec![PhotoCraftCommand::Eraser.into()])
}

/// More commands than the list holds. The empty query lists the first rows; typing "a" matches every command and the
/// footer counts the 50 listed of all.
#[story(CommandPalette<'static>, covers = [], drive = Drive::Never, title = "Many results", commands = commands)]
fn command_palette_many_results(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    // PLAN: the sheet has the story data type "a"; the query is the palette's own session memory, which story data cannot set.
    palette(ui, data, MANY.as_slice(), Vec::new())
}

/// The active row under `CommandHighlight::Hover`: the hover tint with the row's own ink, as an app sets it at its root
/// for a quiet command highlight.
#[story(CommandPalette<'static>, covers = [], drive = Drive::Never, title = "Hover highlight", commands = commands)]
fn command_palette_hover_highlight(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let recents = [PhotoCraftCommand::Levels, PhotoCraftCommand::Brush].map(CommandRef::from).to_vec();
    with_env(
        ui,
        |overrides| {
            overrides.values.set(CommandHighlight::Hover);
        },
        |ui| palette(ui, data, ENTRIES.as_slice(), recents),
    )
}
