//! `EditMenu`: the rows of a text control's context menu.

use super::{Menu, MenuItem, MenuWords};
use crate::contract::{Component, Cx, OverlayUi, Overlays as _, Show as _, ShowContainer as _, Trigger, WordSetCx as _};
use crate::display::Divider;
use crate::primitives::TextOptions;
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
/// Menu-body content: every text control shows it as its edit menu, on a right click in its text.
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

/// The context menu of a component's text part, the one every text control opens: a context request on the component
/// (a right click, the Menu key, Shift+F10) opens the [`EditMenu`] at the pointer, or at the control for a key. Its
/// rows run their `EditCommands` through the command board, which hands each to the focused text part once the pick has
/// closed the menu and given the focus back. Every text control calls this from its `overlay` hook with its text
/// part's options; it opens nothing for a control whose caller hangs a context menu of its own on it
/// (`TextOptions::own_menu`).
pub(crate) fn text_menu<C: Component>(ui: &mut OverlayUi<'_>, cx: &mut Cx<'_, C>, options: &TextOptions) {
    let Some(text) = cx.text_part() else { return };
    let popup = cx.popup(crate::contract::EDIT_MENU);
    // A request at another part (a chip, a chevron) is that part's.
    let at_text = cx.context_part().is_none_or(|part| part.key == text);
    if !options.own_menu && cx.context_requested() && at_text && !popup.is_open(cx) {
        popup.open(cx);
    }
    let name = cx.words::<MenuWords>().edit.clone();
    let layer = super::layer(false).opened_by(&cx.core.response, Trigger::Context);
    ui.show(popup, layer, |ui, _| Menu::inline(name.clone()).show(ui, |ui, _| EditMenu.show(ui)));
}
