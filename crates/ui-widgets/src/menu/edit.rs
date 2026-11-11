//! `EditMenu`: the rows of a text control's context menu.

use super::MenuItem;
use crate::contract::Show as _;
use crate::display::Divider;
use crate::runtime::commands::EditCommands;
use egui::Ui;

/// What an [`EditMenu`] reports.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct EditMenuOutput {
    /// The command whose row ran in this pass.
    pub ran: Option<EditCommands>,
}

/// The edit rows of a text control: Undo and Redo, Cut, Copy, Paste and Delete, then Select All.
///
/// Each row is a `MenuItem::command` over an `EditCommands` command, so its label, icon, chord and enabled state come
/// from the command: the focused text part publishes which commands apply (no selection, no Cut, Copy or Delete).
/// Menu-body content: show it in the body of the `Menu::context` of the text control.
#[derive(crate::contract::Pattern)]
pub struct EditMenu;

impl EditMenu {
    /// Shows the rows in the menu body `ui` and reports the command that ran.
    pub fn show(self, ui: &mut Ui) -> EditMenuOutput {
        let mut ran = None;
        let mut row = |ui: &mut Ui, command: EditCommands| {
            if MenuItem::command(command).show(ui).ran {
                ran = Some(command);
            }
        };
        row(ui, EditCommands::Undo);
        row(ui, EditCommands::Redo);
        Divider::horizontal().show(ui);
        row(ui, EditCommands::Cut);
        row(ui, EditCommands::Copy);
        row(ui, EditCommands::Paste);
        row(ui, EditCommands::Delete);
        Divider::horizontal().show(ui);
        row(ui, EditCommands::SelectAll);
        EditMenuOutput { ran }
    }
}
