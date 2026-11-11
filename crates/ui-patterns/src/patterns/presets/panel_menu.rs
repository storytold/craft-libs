//! `PresetPanelMenu`: the rows every preset panel adds to its panel menu: Grid View, List View, New Group… and the
//! file entries.

use super::files::{PresetFileRequest, PresetFiles};
use super::state::PresetView;
use crate::menu::PanelRow;
use crate::runtime::commands::{CommandRef, CommandState};

/// The commands of a preset panel's menu. They are app commands: the host gates them with the states
/// [`PresetPanelMenu::states`] returns, and runs them with [`PresetPanelMenu::run_command`] when they fire.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, craft_ui_macros::CommandSet)]
#[command(prefix = "presets")]
#[non_exhaustive]
pub enum PresetCommands {
    /// Show the presets as a grid of thumbnails.
    #[command(label = "Grid View", kind = "radio", group = "presets.view")]
    GridView,
    /// Show the presets as a list of rows.
    #[command(label = "List View", kind = "radio", group = "presets.view")]
    ListView,
    /// Make a group and open its rename.
    #[command(label = "New Group…")]
    NewGroup,
    /// Add the presets of a file.
    #[command(label = "Load…")]
    Load,
    /// Replace the presets with those of a file.
    #[command(label = "Replace…")]
    Replace,
    /// Save the presets, or the selected one, to a file.
    #[command(label = "Save…")]
    Save,
}

/// What [`PresetPanelMenu::run_command`] decided.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum PresetMenuAction {
    /// Grid View or List View: the host stores the view in its `PresetState`.
    View(PresetView),
    /// New Group… was chosen: the app makes the group and opens its rename.
    NewGroup,
    /// A file entry asks the host for a dialog.
    File(PresetFileRequest),
}

/// The panel-menu rows of a preset panel: Grid View and List View as radio rows, a separator, New Group…, then the
/// file entries. The app adds its own rows (Swatches' Reset). Pass [`rows`](Self::rows) as the host rows of a
/// `PanelMenu`.
pub struct PresetPanelMenu<'a, K> {
    view: PresetView,
    new_group: Option<bool>,
    files: Option<&'a PresetFiles<'a, K>>,
    selected: Option<&'a K>,
}

impl<'a, K> PresetPanelMenu<'a, K> {
    /// The two view rows, `view` checked.
    pub fn new(view: PresetView) -> Self {
        PresetPanelMenu { view, new_group: None, files: None, selected: None }
    }

    /// Lists New Group… (for a panel with groups), disabled when `enabled` is false (the library's nesting is
    /// reached).
    #[must_use]
    pub fn new_group(mut self, enabled: bool) -> Self {
        self.new_group = Some(enabled);
        self
    }

    /// Lists the file entries of `files`.
    #[must_use]
    pub fn files(mut self, files: &'a PresetFiles<'a, K>) -> Self {
        self.files = Some(files);
        self
    }

    /// The selected preset, for Save….
    #[must_use]
    pub fn selected(mut self, selected: Option<&'a K>) -> Self {
        self.selected = selected;
        self
    }

    /// The commands the menu lists, in order.
    pub fn commands(&self) -> Vec<PresetCommands> {
        let mut commands = vec![PresetCommands::GridView, PresetCommands::ListView];
        if self.new_group.is_some() {
            commands.push(PresetCommands::NewGroup);
        }
        commands.extend(self.files.map(PresetFiles::commands).unwrap_or_default());
        commands
    }

    /// The rows: the view rows, a separator when more follow, New Group…, then the file entries. A command row takes
    /// its label, check and enabled state from the command.
    pub fn rows(&self) -> Vec<PanelRow> {
        let mut rows = Vec::new();
        for (at, command) in self.commands().into_iter().enumerate() {
            if at == 2 {
                rows.push(PanelRow::Separator);
            }
            rows.push(PanelRow::Command(CommandRef::from(command)));
        }
        rows
    }

    /// The state of each listed command: the current view checked, New Group… and Save… enabled as asked.
    pub fn states(&self) -> Vec<(PresetCommands, CommandState)> {
        self.commands()
            .into_iter()
            .map(|command| {
                let state = match command {
                    PresetCommands::GridView => CommandState::new().checked(self.view == PresetView::Grid),
                    PresetCommands::ListView => CommandState::new().checked(self.view == PresetView::List),
                    PresetCommands::NewGroup => CommandState::new().enabled(self.new_group.unwrap_or(false)),
                    other => CommandState::new().enabled(self.files.is_none_or(|files| files.enabled(other, self.selected))),
                };
                (command, state)
            })
            .collect()
    }

    /// Carries out a command of the menu: a view row names the view, New Group… reports `NewGroup` (nothing when it is
    /// disabled), a file entry gives its dialog request.
    pub fn run_command(&self, command: PresetCommands) -> Option<PresetMenuAction> {
        match command {
            PresetCommands::GridView => Some(PresetMenuAction::View(PresetView::Grid)),
            PresetCommands::ListView => Some(PresetMenuAction::View(PresetView::List)),
            PresetCommands::NewGroup => self.new_group.filter(|enabled| *enabled).map(|_| PresetMenuAction::NewGroup),
            file => self.files?.request(file, self.selected).map(PresetMenuAction::File),
        }
    }
}
