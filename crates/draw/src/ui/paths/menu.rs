//! The commands of the Paths panel.

use super::{PathKind, PathRow, PathsWords};
use craft_ui::contract::{CommandSet, Props as _, ShowContainer as _};
use craft_ui::menu::{Menu, MenuItem};
use craft_ui::runtime::env::Label;
use egui::{Response, Ui};

/// The commands of the Paths panel: its panel-menu rows, its row menu and the footer's buttons.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, CommandSet)]
#[command(prefix = "pathsPanel", scope = "pathsPanel")]
#[non_exhaustive]
pub enum PathsCommands {
    /// Load the path as a selection.
    #[command(label = "Make Selection…")]
    MakeSelection,
    /// Fill the path with the foreground colour.
    #[command(label = "Fill Path…")]
    FillPath,
    /// Stroke the path with the current tool.
    #[command(label = "Stroke Path…")]
    StrokePath,
    /// Save the work path.
    #[command(label = "Save Path…")]
    SavePath,
    /// Duplicate the path.
    #[command(label = "Duplicate Path…")]
    DuplicatePath,
    /// Rename the path in place.
    #[command(label = "Rename Path…")]
    RenamePath,
    /// Delete the path.
    #[command(label = "Delete Path")]
    DeletePath,
}

impl PathsCommands {
    /// Every command of the set, in menu order.
    pub const ALL: [PathsCommands; 7] = [
        PathsCommands::MakeSelection,
        PathsCommands::FillPath,
        PathsCommands::StrokePath,
        PathsCommands::SavePath,
        PathsCommands::DuplicatePath,
        PathsCommands::RenamePath,
        PathsCommands::DeletePath,
    ];

    /// Whether the command applies to a targeted row of `kind` (`None`: no row targeted). Fill Path and Stroke Path
    /// need `can_fill`; Save Path is the work path's; a work path cannot be duplicated; only a saved path is renamed
    /// from the menu.
    pub fn applies_to(self, kind: Option<PathKind>, can_fill: bool) -> bool {
        let Some(kind) = kind else { return false };
        match self {
            PathsCommands::MakeSelection | PathsCommands::DeletePath => true,
            PathsCommands::FillPath | PathsCommands::StrokePath => can_fill,
            PathsCommands::SavePath => kind == PathKind::Work,
            PathsCommands::DuplicatePath => kind != PathKind::Work,
            PathsCommands::RenamePath => kind == PathKind::Saved,
        }
    }
}

fn label_of(command: PathsCommands, kind: PathKind, words: &PathsWords) -> Label {
    let word = match command {
        PathsCommands::MakeSelection => &words.menu_make_selection,
        PathsCommands::FillPath => &words.menu_fill,
        PathsCommands::StrokePath => &words.menu_stroke,
        PathsCommands::SavePath => &words.menu_save,
        PathsCommands::DuplicatePath => &words.menu_duplicate,
        PathsCommands::RenamePath => &words.menu_rename,
        PathsCommands::DeletePath if kind == PathKind::VectorMask => &words.delete_vector_mask,
        PathsCommands::DeletePath => &words.menu_delete,
    };
    word.clone()
}

/// Shows the context menu of `row`, hanging from `opener`, and returns the command that ran. The menu lists the
/// commands the row's kind has; Fill Path and Stroke Path are greyed while `can_fill` is false.
pub(super) fn show_row_menu<K>(ui: &mut Ui, opener: &Response, row: &PathRow<K>, can_fill: bool, words: &PathsWords) -> Option<PathsCommands> {
    let kind = row.kind;
    let shown = Menu::context(words.row_menu.clone(), opener).show(ui, |ui, _| {
        use craft_ui::contract::Show as _;
        let mut ran = None;
        for command in PathsCommands::ALL.into_iter().filter(|command| command.applies_to(Some(kind), true)) {
            let item = MenuItem::new(label_of(command, kind, words)).enabled(command.applies_to(Some(kind), can_fill));
            if item.show(ui).ran {
                ran = Some(command);
            }
        }
        ran
    });
    shown.inner.flatten()
}
