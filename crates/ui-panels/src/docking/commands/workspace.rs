//! The Window › Workspace commands and how they change the workspace set.

use crate::runtime::commands::CommandState;
use craft_behavior::dock::{Workspace, WorkspaceSet};
use std::hash::Hash;

/// The Window › Workspace commands over the workspace set.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, craft_ui_macros::CommandSet)]
#[command(prefix = "workspace")]
#[non_exhaustive]
pub enum WorkspaceCommands {
    /// Return the current workspace to its stored layout.
    #[command(label = "Reset Workspace")]
    Reset,
    /// Ask for a name and save the live layout under it.
    #[command(label = "New Workspace…")]
    New,
    /// Ask which saved workspace to delete.
    #[command(label = "Delete Workspace…")]
    Delete,
    /// Refuse moves and closes until it is unlocked; resizing still works.
    #[command(label = "Lock Workspace", kind = "toggle")]
    Lock,
}

/// What the host asks the user after a workspace command.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum WorkspaceAsk {
    /// A name for `WorkspaceSet::save`.
    Name,
    /// A saved workspace for `WorkspaceSet::delete`.
    Which,
}

/// Runs a workspace command: Reset and Lock take effect, New and Delete say what to ask.
pub fn run_workspace<P: Clone + Eq + Hash>(set: &mut WorkspaceSet<P>, live: &mut Workspace<P>, command: WorkspaceCommands) -> Option<WorkspaceAsk> {
    match command {
        WorkspaceCommands::Reset => {
            let _ = set.reset(live);
            None
        }
        WorkspaceCommands::Lock => {
            set.locked = !set.locked;
            None
        }
        WorkspaceCommands::New => Some(WorkspaceAsk::Name),
        WorkspaceCommands::Delete => Some(WorkspaceAsk::Which),
    }
}

/// The state of each workspace command: Reset when the current workspace has edits, Delete while a saved workspace
/// exists, Lock checked while the set is locked.
pub fn workspace_states<P: Clone + Eq + Hash>(set: &WorkspaceSet<P>) -> Vec<(WorkspaceCommands, CommandState)> {
    vec![
        (WorkspaceCommands::Reset, CommandState::new().enabled(set.edited.contains_key(&set.current))),
        (WorkspaceCommands::New, CommandState::new()),
        (WorkspaceCommands::Delete, CommandState::new().enabled(!set.saved.is_empty())),
        (WorkspaceCommands::Lock, CommandState::new().checked(set.locked)),
    ]
}
