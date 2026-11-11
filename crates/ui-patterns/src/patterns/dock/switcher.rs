//! `WorkspaceSwitcher`: a select over the workspace set's names with the workspace commands in its list.

#[cfg(feature = "testing")]
mod stories;

use crate::contract::{Bind, Entry, NoValue, Outcome, Props as _, Show as _};
use crate::display::Divider;
use crate::docking::{DockWords, WorkspaceCommands};
use crate::menu::MenuItem;
use crate::pickers::Select;
use crate::runtime::env::Env;
use craft_behavior::dock::WorkspaceSet;
use craft_theme::ControlSize;
use egui::{Response, Ui};

/// What a [`WorkspaceSwitcher`] reports.
#[derive(Debug)]
#[non_exhaustive]
#[must_use = "the host carries out the switch"]
pub struct WorkspaceSwitcherOutput {
    /// The select's response.
    pub response: Response,
    /// The id of the workspace the user picked this pass.
    pub switched: Option<String>,
}

impl Outcome for WorkspaceSwitcherOutput {
    type Value = NoValue;

    fn response(&self) -> &Response {
        &self.response
    }
}

/// The switcher at the end of an options bar: a small select showing the current workspace's name. Its list has the
/// built-in and saved workspaces, then Reset, New, Delete and Lock Workspace as the `WorkspaceCommands` rows. A row
/// fires its command, which the host takes with its other app commands and runs with `run_workspace`; the host
/// switches with `WorkspaceSet::select`. The switcher edits nothing.
#[must_use = "show the switcher with `show`"]
#[derive(crate::contract::Pattern)]
#[pattern(probe(WorkspaceSwitcher<'static, u32>))]
pub struct WorkspaceSwitcher<'a, P> {
    set: &'a WorkspaceSet<P>,
}

impl<'a, P> WorkspaceSwitcher<'a, P> {
    /// A switcher over `set`.
    pub fn new(set: &'a WorkspaceSet<P>) -> Self {
        WorkspaceSwitcher { set }
    }

    /// Shows the switcher and reports the workspace picked.
    pub fn show(self, ui: &mut Ui) -> WorkspaceSwitcherOutput {
        let set = self.set;
        let words = Env::of(ui).words::<DockWords>();
        let entries: Vec<Entry<String>> = set.builtin.iter().chain(&set.saved).map(|w| Entry::new(w.id.clone(), w.name.clone())).collect();
        let shown = Select::new(words.choose_workspace.clone(), Bind::given(set.current.clone()), &entries)
            .size(ControlSize::Sm)
            .header_menu(|ui, _| {
                for ran in [WorkspaceCommands::Reset, WorkspaceCommands::New, WorkspaceCommands::Delete, WorkspaceCommands::Lock] {
                    if ran == WorkspaceCommands::Lock {
                        Divider::horizontal().show(ui);
                    }
                    MenuItem::command(ran).show(ui);
                }
            })
            .show(ui);
        let switched = shown.value().cloned();
        WorkspaceSwitcherOutput { response: shown.response, switched }
    }
}
