//! The docking pattern: `DockArea` and `DockPanels` on `SplitArea` and `DockGroup`, and `PaneArea` for task panes.

mod words;

mod area;
mod hide;
mod switcher;
// Task panes: docked panes as shell panes, floating panes as floating frames.
mod panes;

pub use area::{DockArea, DockAreaOutput, InvalidGeometry};
pub use panes::{
    PaneArea, PaneAreaOutput, PaneCommands, PaneContent, PaneFrame, PaneFrameOutput, PaneFrameStyle, PanePlace, PaneRequest, PaneSpec, PaneState,
    SidePolicy,
};
pub use switcher::{WorkspaceSwitcher, WorkspaceSwitcherOutput};
