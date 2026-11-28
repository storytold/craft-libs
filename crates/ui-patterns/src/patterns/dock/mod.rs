//! The docking pattern: `DockArea` and `DockPanels` on `SplitArea` and `DockGroup`, and `PaneArea` for task panes.

mod words;

pub(crate) mod area;
mod hide;
pub(crate) mod switcher;
pub(crate) mod workspace_dialogs;
// Task panes: docked panes as shell panes, floating panes as floating frames.
pub(crate) mod panes;

pub use area::{DockArea, DockAreaOutput, InvalidGeometry, WindowFiles};
pub use panes::{
    PaneArea, PaneAreaOutput, PaneCommands, PaneContent, PaneEdges, PaneFrame, PaneFrameOutput, PaneFrameStyle, PanePlace, PaneRequest, PaneSpec,
    PaneState, SidePolicy,
};
pub use switcher::{WorkspaceSwitcher, WorkspaceSwitcherOutput};
pub use words::WorkspaceDialogWords;
pub use workspace_dialogs::{
    DeleteWorkspaceDialog, DeleteWorkspaceDialogOutput, HostedDeleteWorkspace, HostedNewWorkspace, NewWorkspace, NewWorkspaceDialog,
    NewWorkspaceDialogOutput,
};
