//! The Swatches panel menu, its commands and file import and export, and the edits the panel's events make.

use craft_behavior::EditEvent;
use craft_theme::Srgba;
use craft_theme::variant::Tone;
use craft_ui::bars::StatusMessage;
use craft_ui::contract::CommandSet;
use craft_ui::menu::PanelRow;
use craft_ui::patterns::presets::{PresetCommands, PresetEvent, PresetExport, PresetFileRequest, PresetFiles, PresetFooterEvent};
use craft_ui::runtime::commands::{CommandRef, CommandState};

use super::library::{find_swatch, free_name, group_of, group_rename, reorder_edits, swatch_rename, swatches_full, unique_group_name};
use super::{MAX_GROUPS, SwatchEdit, SwatchKey, SwatchesState, SwatchesWords};
use crate::palette::{SwatchColor, SwatchFormat, SwatchGroup, SwatchReport};

/// The purpose of the swatch file pickers: their folder is remembered under it.
const PICK_PURPOSE: &str = "swatches";

/// What `SwatchesPanel::apply_file` did.
#[derive(Debug)]
#[non_exhaustive]
#[must_use = "the host runs the edit and shows the status message"]
pub struct SwatchesFileOutput {
    /// The edit the host runs: a read of a file imports its groups.
    pub edit: Option<SwatchEdit>,
    /// The notes of the swatch file that was read or written, once the panel has handled it.
    pub report: Option<SwatchReport>,
    /// The status message for the notes that lost something or for a failure.
    pub status: Option<StatusMessage>,
}

/// The commands of the Swatches panel menu, in Photoshop's words.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, CommandSet)]
#[command(prefix = "swatches")]
#[non_exhaustive]
pub enum SwatchesCommand {
    /// Add the foreground colour as a swatch.
    #[command(label = "New Swatch…")]
    NewSwatch,
    /// Add an empty group.
    #[command(label = "New Swatch Group…")]
    NewGroup,
    /// Add the swatches of a file.
    #[command(label = "Import Swatches…")]
    Import,
    /// Replace the swatches with those of a file.
    #[command(label = "Replace Swatches…")]
    Replace,
    /// Write the swatches to a Photoshop swatch file.
    #[command(label = "Export Swatches…")]
    Export,
    /// Write the swatches to an exchange file with their groups.
    #[command(label = "Export Swatches for Exchange…")]
    ExportExchange,
    /// Put the default swatches back.
    #[command(label = "Reset Swatches")]
    Reset,
}

impl SwatchesCommand {
    /// Every command, in menu order.
    pub const ALL: [SwatchesCommand; 7] = [
        SwatchesCommand::NewSwatch,
        SwatchesCommand::NewGroup,
        SwatchesCommand::Import,
        SwatchesCommand::Replace,
        SwatchesCommand::Export,
        SwatchesCommand::ExportExchange,
        SwatchesCommand::Reset,
    ];
}

/// What [`SwatchesMenu::run_command`] did.
#[derive(Debug)]
#[non_exhaustive]
#[must_use = "the host runs the edit and answers the file request"]
pub struct SwatchesMenuOutput {
    /// The edit a command makes, for the host to run (New Swatch: `Add` with the foreground and the next numbered name in
    /// the selected swatch's group).
    pub edit: Option<SwatchEdit>,
    /// The file dialog the command asks the host for; the host answers with `SwatchesPanel::apply_file`.
    pub file: Option<PresetFileRequest>,
    /// The status message for a command that failed before it asked for a file.
    pub status: Option<StatusMessage>,
}

/// The Swatches panel menu: the rows for the dock's panel menu, the states the host publishes, and the carrying out
/// of a fired command.
pub struct SwatchesMenu;

impl SwatchesMenu {
    /// The rows: New Swatch…, New Swatch Group…, a separator, Import, Replace, Export and Export for Exchange, a
    /// separator, Reset Swatches.
    pub fn rows() -> Vec<PanelRow> {
        let row = |command: SwatchesCommand| PanelRow::Command(CommandRef::from(command));
        vec![
            row(SwatchesCommand::NewSwatch),
            row(SwatchesCommand::NewGroup),
            PanelRow::Separator,
            row(SwatchesCommand::Import),
            row(SwatchesCommand::Replace),
            row(SwatchesCommand::Export),
            row(SwatchesCommand::ExportExchange),
            PanelRow::Separator,
            row(SwatchesCommand::Reset),
        ]
    }

    /// The state of each command: both exports are disabled while the groups hold no swatch.
    pub fn states(groups: &[SwatchGroup]) -> Vec<(SwatchesCommand, CommandState)> {
        let exports = groups.iter().any(|group| !group.entries.is_empty());
        SwatchesCommand::ALL
            .into_iter()
            .map(|command| {
                let enabled = !matches!(command, SwatchesCommand::Export | SwatchesCommand::ExportExchange) || exports;
                (command, CommandState::new().enabled(enabled))
            })
            .collect()
    }

    /// Carries out `command`. New Swatch reports the foreground added to the selected swatch's (or group's) group, as
    /// the footer's New does, and the panel selects it and opens its name for editing in its next pass; New Swatch
    /// Group and Reset Swatches report their edits. Import, Replace and the two exports give the file request for the
    /// host.
    pub fn run_command(
        groups: &[SwatchGroup],
        state: &mut SwatchesState,
        command: SwatchesCommand,
        foreground: Srgba,
        words: &SwatchesWords,
    ) -> SwatchesMenuOutput {
        let mut output = SwatchesMenuOutput { edit: None, file: None, status: None };
        let readable: Vec<&str> = SwatchFormat::ALL.iter().filter(|f| f.can_read()).map(|f| f.extension()).collect();
        let import = || PresetFiles::<SwatchKey>::new(PICK_PURPOSE, words.file.as_str(), &readable).replace(true);
        match command {
            SwatchesCommand::NewSwatch => output.edit = new_swatch_edit(groups, state, foreground, words),
            SwatchesCommand::NewGroup => output.edit = new_group_edit(groups, state, words),
            SwatchesCommand::Reset => output.edit = Some(SwatchEdit::Reset),
            SwatchesCommand::Import => output.file = import().request(PresetCommands::Load, None),
            SwatchesCommand::Replace => output.file = import().request(PresetCommands::Replace, None),
            SwatchesCommand::Export => Self::export(groups, state, SwatchFormat::Aco, words, &mut output),
            SwatchesCommand::ExportExchange => Self::export(groups, state, SwatchFormat::Ase, words, &mut output),
        }
        output
    }

    /// The file request that writes `groups` as `format`; a library the format cannot hold is a status message.
    fn export(groups: &[SwatchGroup], state: &mut SwatchesState, format: SwatchFormat, words: &SwatchesWords, output: &mut SwatchesMenuOutput) {
        match crate::palette::write_swatches(groups, format) {
            Ok((bytes, report)) => {
                let file_name = format!("{}.{}", words.file.as_str(), format.extension());
                let files = PresetFiles::<SwatchKey>::new(PICK_PURPOSE, words.file.as_str(), &[format.extension()])
                    .export(move |_| Some(PresetExport::new(file_name.clone(), bytes.clone())));
                output.file = files.request(PresetCommands::Save, None);
                state.export_notes = Some(report);
            }
            Err(error) => output.status = Some(StatusMessage::new(words.export_failed.fill(&[("reason", &error.to_string())])).tone(Tone::Bad)),
        }
    }
}

/// The foreground colour into the selected swatch's group under the next free numbered name; the swatch is selected and
/// its name opens for editing. `None` when the library is full.
fn new_swatch_edit(groups: &[SwatchGroup], st: &mut SwatchesState, foreground: Srgba, words: &SwatchesWords) -> Option<SwatchEdit> {
    if swatches_full(groups) {
        return None;
    }
    let group = st.selection.lead.as_ref().and_then(|key| group_of(groups, key)).and_then(|index| groups.get(index)).map(|g| g.name.clone());
    let color = SwatchColor::Rgb(foreground.to_rgb_array().map(|v| v.clamp(0.0, 1.0)));
    let name = free_name(groups, words.swatch_n.as_str());
    let key = SwatchKey::Swatch(name.clone());
    st.selection = craft_behavior::Selection::single(key.clone());
    st.rename = Some(key);
    Some(SwatchEdit::add(group, name, color))
}

/// An empty group under a unique name, opened for renaming. `None` when the library holds the most groups.
fn new_group_edit(groups: &[SwatchGroup], st: &mut SwatchesState, words: &SwatchesWords) -> Option<SwatchEdit> {
    if groups.len() >= MAX_GROUPS {
        return None;
    }
    let name = unique_group_name(groups, words.group.as_str().trim());
    st.rename = Some(SwatchKey::Group(name.clone()));
    Some(SwatchEdit::NewGroup { name })
}

/// The edit that deletes the row `key` names, when the library has it.
fn delete_edit(groups: &[SwatchGroup], key: &SwatchKey) -> Option<SwatchEdit> {
    match key {
        SwatchKey::Swatch(name) => find_swatch(groups, name).map(|_| SwatchEdit::Delete { swatch: name.clone() }),
        SwatchKey::Group(name) => groups.iter().any(|g| g.name == *name).then(|| SwatchEdit::DeleteGroup { group: name.clone() }),
    }
}

/// The edits one event of the browser makes over `groups`, in order; selection and rename requests go to `st`.
pub(super) fn event_edits(
    groups: &[SwatchGroup],
    st: &mut SwatchesState,
    event: PresetEvent<SwatchKey>,
    foreground: Srgba,
    words: &SwatchesWords,
) -> Vec<SwatchEdit> {
    match event {
        PresetEvent::Rename { key: SwatchKey::Swatch(old), text, .. } => {
            let Some(name) = find_swatch(groups, &old).and_then(|at| swatch_rename(groups, at, &text)).filter(|name| *name != old) else {
                return Vec::new();
            };
            let (old_key, new_key) = (SwatchKey::Swatch(old.clone()), SwatchKey::Swatch(name.clone()));
            let swap = |key: &mut SwatchKey| {
                if *key == old_key {
                    *key = new_key.clone();
                }
            };
            let selection = &mut st.selection;
            selection.items.iter_mut().chain(selection.lead.iter_mut()).chain(selection.anchor.iter_mut()).for_each(swap);
            vec![SwatchEdit::Rename { swatch: old, name }]
        }
        PresetEvent::Rename { key: SwatchKey::Group(old), text, .. } => {
            let Some(name) = groups.iter().position(|g| g.name == old).and_then(|i| group_rename(groups, i, &text)).filter(|name| *name != old)
            else {
                return Vec::new();
            };
            vec![SwatchEdit::RenameGroup { group: old, name }]
        }
        PresetEvent::Delete(key) => delete_edit(groups, &key).into_iter().collect(),
        PresetEvent::MoveTo { key: SwatchKey::Swatch(name), group: SwatchKey::Group(to), .. } => {
            let (Some(_), Some(target)) = (find_swatch(groups, &name), groups.iter().find(|g| g.name == to)) else {
                return Vec::new();
            };
            vec![SwatchEdit::MoveTo { swatch: name, group: to.clone(), at: target.entries.len() }]
        }
        PresetEvent::Reorder(EditEvent::Commit { after, .. } | EditEvent::Set { after, .. }) => reorder_edits(groups, &after),
        PresetEvent::Footer(footer) => footer_edits(groups, st, footer, foreground, words),
        _ => Vec::new(),
    }
}

/// The edit a footer button makes.
fn footer_edits(
    groups: &[SwatchGroup],
    st: &mut SwatchesState,
    event: PresetFooterEvent<SwatchKey>,
    foreground: Srgba,
    words: &SwatchesWords,
) -> Vec<SwatchEdit> {
    match event {
        PresetFooterEvent::New => new_swatch_edit(groups, st, foreground, words).into_iter().collect(),
        PresetFooterEvent::NewGroup => new_group_edit(groups, st, words).into_iter().collect(),
        PresetFooterEvent::Delete => {
            // A swatch of a selected group goes with the group: deleting it after the group would find nothing.
            let with_group = |key: &SwatchKey| {
                matches!(key, SwatchKey::Swatch(_))
                    && group_of(groups, key).and_then(|i| groups.get(i)).is_some_and(|g| st.selection.contains(&SwatchKey::Group(g.name.clone())))
            };
            st.selection.items.iter().filter(|key| !with_group(key)).filter_map(|key| delete_edit(groups, key)).collect()
        }
        _ => Vec::new(),
    }
}
