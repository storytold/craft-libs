//! `PanelMenu`: the rows of a dock panel's menu (the button at the end of a group's tab strip and a tab's context
//! menu): the host's rows, then the dock's rows.

use super::MenuItem;
use super::popup::{Menu, entry_rows};
use crate::contract::{Entry, Show as _};
use crate::display::Divider;
use crate::runtime::commands::CommandRef;
use crate::runtime::env::Label;
use egui::Ui;

/// A dock row of a panel or column menu that a command cannot name: it goes back to the dock as the pick. The panel
/// commands themselves are `PanelRow::Command` rows.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum GroupItem {
    /// A target of the Move Panel To submenu, by its index in the dock's move targets.
    MoveTo(usize),
    /// Collapse to Icons, or Expand Panels on an iconic column.
    CollapseToIcons,
    /// Auto-Collapse Iconic Panels.
    AutoCollapse,
}

/// One row of a panel menu.
#[derive(Clone, Debug)]
#[non_exhaustive]
pub enum PanelRow {
    /// A command row: its label, chord, check and enabled state come from the command, and it runs by itself.
    Command(CommandRef),
    /// A dock row a command cannot name, reported through `PanelMenuOutput::picked`. The entry can be disabled or
    /// checked.
    Item(Entry<GroupItem>),
    /// A row that opens a submenu.
    Submenu(Label, Vec<PanelRow>),
    /// A separator.
    Separator,
}

/// What a [`PanelMenu`] reports.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct PanelMenuOutput {
    /// The dock row that ran in this pass.
    pub picked: Option<GroupItem>,
}

/// The rows of one panel's menu: the host's rows, a separator, then the dock's rows. Menu-body content: show it in
/// the body of a `Menu`.
#[derive(crate::contract::Pattern)]
pub struct PanelMenu {
    host: Vec<PanelRow>,
    group: Vec<PanelRow>,
}

impl PanelMenu {
    /// A menu of the host's `host` rows followed by the dock's `group` rows.
    pub fn new(host: Vec<PanelRow>, group: Vec<PanelRow>) -> Self {
        PanelMenu { host, group }
    }

    /// Shows the rows in the menu body `ui` and reports the dock row that ran.
    pub fn show(self, ui: &mut Ui) -> PanelMenuOutput {
        let mut picked = None;
        Self::rows(ui, &self.host, &mut picked);
        if !self.host.is_empty() && !self.group.is_empty() {
            Divider::horizontal().show(ui);
        }
        Self::rows(ui, &self.group, &mut picked);
        PanelMenuOutput { picked }
    }

    fn rows(ui: &mut Ui, rows: &[PanelRow], picked: &mut Option<GroupItem>) {
        for row in rows {
            match row {
                PanelRow::Command(command) => {
                    MenuItem::command(command.clone()).show(ui);
                }
                PanelRow::Item(entry) => {
                    if entry_rows(ui, std::slice::from_ref(entry)).is_some() {
                        *picked = entry.value().copied();
                    }
                }
                PanelRow::Submenu(label, rows) => {
                    Menu::submenu_row(ui, label.clone(), |ui, _| Self::rows(ui, rows, picked));
                }
                PanelRow::Separator => {
                    Divider::horizontal().show(ui);
                }
            }
        }
    }
}
