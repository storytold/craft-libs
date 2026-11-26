//! Docking commands: the panel, arrange and workspace command sets, and the pure functions that turn a command into
//! the dock model's `Action`s or the host's next step.
//!
//! The commands carry no payload. A panel command acts on the shown panel of the dock group it reaches: `DockArea`
//! gives each group the `dockGroup` scope of [`DockCommands`], publishes [`dock_states`] there and runs [`actions`]
//! for the commands it takes, so a menu row, a key and the command palette run the same code. Nothing here reads
//! `Ui` or egui memory; a host that shows its own dock calls the same functions and applies the result with
//! `Workspace::apply_all`.

mod arrange;
mod dock;
mod workspace;

pub use arrange::{ArrangeCommands, ArrangeSpace, Arranged, arrange, arrange_states};
pub use dock::{DockCommands, DockScene, DockTo, actions, dock_states, move_actions};
pub use workspace::{WorkspaceAsk, WorkspaceCommands, run_workspace, workspace_states};
