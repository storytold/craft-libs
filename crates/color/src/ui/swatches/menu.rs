//! The Swatches panel menu, its commands and file import and export.

use craft_behavior::EditEvent;
use craft_cms::convert::CmykConverter;
use craft_theme::Srgba;
use craft_theme::variant::Tone;
use craft_ui::contract::CommandSet;
use craft_ui::menu::PanelRow;
use craft_ui::patterns::presets::{PresetCommands, PresetEvent, PresetExport, PresetFileRequest, PresetFiles, PresetFooterEvent};
use craft_ui::runtime::commands::{CommandRef, CommandState};
use craft_ui::runtime::toast::Toast;

use super::{SwatchAt, SwatchKey, SwatchLibrary, SwatchesState, SwatchesWords};
use crate::palette::{SwatchColor, SwatchFormat, SwatchReport};

/// The purpose of the swatch file pickers: their folder is remembered under it.
const PICK_PURPOSE: &str = "swatches";

/// What [`SwatchesPanel::apply_file`] did.
#[derive(Debug)]
#[non_exhaustive]
#[must_use = "the host saves a changed library and posts the toast"]
pub struct SwatchesFileOutput {
    /// The library changed: the app saves it.
    pub changed: bool,
    /// The notes of the swatch file that was read or written, once the panel has handled it.
    pub report: Option<SwatchReport>,
    /// The toast for the notes that lost something or for a failure.
    pub toast: Option<Toast>,
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
#[must_use = "the app saves a changed library and answers the file request"]
pub struct SwatchesMenuOutput {
    /// The library changed: the app saves it.
    pub changed: bool,
    /// The file dialog the command asks the host for; the host answers with [`SwatchesPanel::apply_file`].
    pub file: Option<PresetFileRequest>,
    /// A toast for a command that failed before it asked for a file.
    pub toast: Option<Toast>,
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

    /// The state of each command: both exports are disabled while the library has no swatch.
    pub fn states(library: &SwatchLibrary) -> Vec<(SwatchesCommand, CommandState)> {
        let exports = library.total() > 0;
        SwatchesCommand::ALL
            .into_iter()
            .map(|command| {
                let enabled = !matches!(command, SwatchesCommand::Export | SwatchesCommand::ExportExchange) || exports;
                (command, CommandState::new().enabled(enabled))
            })
            .collect()
    }

    /// Carries out `command`. New Swatch adds the foreground to the selected swatch's (or group's) group, as the
    /// footer's New does, and the panel selects it and opens its name for editing in its next pass; New Swatch Group
    /// and Reset Swatches act on `library`. Import, Replace and the two exports give the file request for the host.
    pub fn run_command(
        library: &mut SwatchLibrary,
        state: &mut SwatchesState,
        command: SwatchesCommand,
        foreground: Srgba,
        words: &SwatchesWords,
    ) -> SwatchesMenuOutput {
        let mut output = SwatchesMenuOutput { changed: false, file: None, toast: None };
        let readable: Vec<&str> = SwatchFormat::ALL.iter().filter(|f| f.can_read()).map(|f| f.extension()).collect();
        let import = || PresetFiles::<SwatchKey>::new(PICK_PURPOSE, words.file.as_str(), &readable).replace(true);
        match command {
            SwatchesCommand::NewSwatch => {
                let group = state.selected.as_ref().and_then(|key| library.group_of(key));
                let color = SwatchColor::Rgb(foreground.to_rgb_array().map(|v| v.clamp(0.0, 1.0)));
                let at = library.add_numbered(color, None, group, words.swatch_n.as_str());
                output.changed = at.is_some();
                if let Some(entry) = at.and_then(|at| library.get(at)) {
                    let key = SwatchKey::Swatch(entry.name.clone());
                    state.selected = Some(key.clone());
                    state.rename = Some(key);
                }
            }
            SwatchesCommand::NewGroup => {
                let at = library.new_group(words.group.as_str());
                output.changed = at.is_some();
                if let Some(at) = at {
                    state.rename = Some(SwatchKey::Group(library.groups[at].name.clone()));
                }
            }
            SwatchesCommand::Reset => {
                library.groups = crate::palette::default_swatches();
                output.changed = true;
            }
            SwatchesCommand::Import => output.file = import().request(PresetCommands::Load, None),
            SwatchesCommand::Replace => output.file = import().request(PresetCommands::Replace, None),
            SwatchesCommand::Export => Self::export(library, state, SwatchFormat::Aco, words, &mut output),
            SwatchesCommand::ExportExchange => Self::export(library, state, SwatchFormat::Ase, words, &mut output),
        }
        output
    }

    /// The file request that writes the library as `format`; a library the format cannot hold is a toast.
    fn export(library: &SwatchLibrary, state: &mut SwatchesState, format: SwatchFormat, words: &SwatchesWords, output: &mut SwatchesMenuOutput) {
        match crate::palette::write_swatches(&library.groups, format) {
            Ok((bytes, report)) => {
                let file_name = format!("{}.{}", words.file.as_str(), format.extension());
                let files = PresetFiles::<SwatchKey>::new(PICK_PURPOSE, words.file.as_str(), &[format.extension()])
                    .export(move |_| Some(PresetExport::new(file_name.clone(), bytes.clone())));
                output.file = files.request(PresetCommands::Save, None);
                state.export_notes = Some(report);
            }
            Err(error) => output.toast = Some(Toast::new(words.export_failed.fill(&[("reason", &error.to_string())])).tone(Tone::Bad)),
        }
    }
}

/// The events of one pass applied to the library and the panel's state.
pub(super) struct Actions<'a> {
    pub(super) library: &'a mut SwatchLibrary,
    pub(super) st: &'a mut SwatchesState,
    pub(super) used: &'a mut Option<(SwatchAt, Srgba, bool)>,
    pub(super) changed: &'a mut bool,
    pub(super) foreground: Srgba,
    pub(super) template: &'a str,
    pub(super) group_name: &'a str,
    pub(super) cmyk: &'a CmykConverter,
}

impl Actions<'_> {
    pub(super) fn event(&mut self, event: PresetEvent<SwatchKey>) {
        match event {
            PresetEvent::Select { key: SwatchKey::Swatch(name), mods, .. } => {
                self.st.selected = Some(SwatchKey::Swatch(name.clone()));
                if let Some(at) = self.library.find(&name)
                    && let Some(entry) = self.library.get(at)
                {
                    let background = mods.alt || mods.cmd || mods.ctrl;
                    *self.used = Some((at, entry.color.to_srgb(self.cmyk), background));
                }
            }
            PresetEvent::Select { key, .. } | PresetEvent::Activate { key, .. } => {
                self.st.selected = Some(key).filter(|k| matches!(k, SwatchKey::Swatch(_)))
            }
            PresetEvent::Rename { key: SwatchKey::Swatch(old), text, .. } => {
                if let Some(at) = self.library.find(&old)
                    && self.library.rename(at, &text)
                {
                    *self.changed = true;
                    if self.st.selected == Some(SwatchKey::Swatch(old)) {
                        self.st.selected = Some(SwatchKey::Swatch(text.trim().to_owned()));
                    }
                }
            }
            PresetEvent::Rename { key: SwatchKey::Group(old), text, .. } => {
                if let Some(group) = self.library.groups.iter().position(|g| g.name == old)
                    && self.library.rename_group(group, &text)
                {
                    *self.changed = true;
                }
            }
            PresetEvent::Delete(key) => self.delete(&key),
            PresetEvent::MoveTo { key: SwatchKey::Swatch(name), group: SwatchKey::Group(to), .. } => {
                if let (Some(at), Some(to)) = (self.library.find(&name), self.library.groups.iter().position(|g| g.name == to)) {
                    self.library.move_to(at, to);
                    *self.changed = true;
                }
            }
            PresetEvent::Reorder(EditEvent::Commit { after, .. } | EditEvent::Set { after, .. }) => {
                *self.changed |= self.library.apply_reorder(&after);
            }
            PresetEvent::Footer(footer) => self.footer(footer),
            _ => {}
        }
    }

    fn footer(&mut self, event: PresetFooterEvent<SwatchKey>) {
        match event {
            PresetFooterEvent::New => self.new_swatch(),
            PresetFooterEvent::NewGroup => {
                if let Some(at) = self.library.new_group(self.group_name) {
                    self.st.rename = Some(SwatchKey::Group(self.library.groups[at].name.clone()));
                    *self.changed = true;
                }
            }
            PresetFooterEvent::Delete => {
                if let Some(key) = self.st.selected.clone() {
                    self.delete(&key);
                }
            }
            _ => {}
        }
    }

    /// The foreground colour into the selected swatch's group, named and selected, then renamed in place.
    fn new_swatch(&mut self) {
        let group = self.st.selected.as_ref().and_then(|key| self.library.group_of(key));
        let color = SwatchColor::Rgb(self.foreground.to_rgb_array().map(|v| v.clamp(0.0, 1.0)));
        let Some(at) = self.library.add_numbered(color, None, group, self.template) else {
            return;
        };
        if let Some(entry) = self.library.get(at) {
            let key = SwatchKey::Swatch(entry.name.clone());
            self.st.selected = Some(key.clone());
            self.st.rename = Some(key);
        }
        *self.changed = true;
    }

    fn delete(&mut self, key: &SwatchKey) {
        let removed = match key {
            SwatchKey::Swatch(name) => self.library.find(name).and_then(|at| self.library.remove(at)).is_some(),
            SwatchKey::Group(name) => match self.library.groups.iter().position(|g| g.name == *name) {
                Some(group) => {
                    self.library.groups.remove(group);
                    true
                }
                None => false,
            },
        };
        if removed {
            *self.changed = true;
        }
    }
}
