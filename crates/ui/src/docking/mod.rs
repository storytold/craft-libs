//! Shared panel docking: transactional layout changes and egui drag/drop presentation.
//!
//! Panel IDs identify application-owned contents, never document state or native windows. A
//! layout can be adapted to an existing workspace schema or serialized with the `serde` feature.
//! Validate deserialized layouts before rendering them. Every transaction checks its complete
//! input and commits only a valid result; a rejected move cannot lose the source panel.

pub mod chrome;
#[cfg(feature = "serde")]
mod command;
#[cfg(feature = "native-windows")]
mod desktop;
mod dock_model;
mod hits;
mod labels;
mod model;
#[cfg(feature = "native-windows")]
mod placement;
mod render;
mod resize;
mod surface;
mod targets;
#[cfg(any(test, feature = "testing"))]
pub mod testing;
mod workspace;

#[cfg(feature = "serde")]
pub use command::{DockCommand, DockOperation, UiDock, ZoneName};
pub use dock_model::{Column, ColumnLocation, Group, GroupLocation, PanelState, Side, Target};
pub use hits::{Divider, Hit, ResizeEdge};
pub use model::{Action, DockError, Floating, Layout, Location, MAX_DEPTH, MAX_PANELS, Monitor, Node, Permissions, Placement, StackEntry, Zone};
pub use render::{close_flyout, flyout, open_flyout};
pub use surface::{
    DockArea, DockContent, DockEvent, DockOutput, DockStyle, DropFeedback, DropRequest, FloatSpace, HidePanels, PanelLimits, WindowFiles, begin_drag,
    cancel_drag, float_space, is_dragging,
};
pub use targets::{DropProposal, DropTarget, drop_zone, resolve_drop, tab_insertion_marker, zone_rect, zone_shape};
pub use workspace::Workspace;

use surface::{Drag, GroupDrag, GroupGesture};

#[cfg(test)]
mod command_tests;
#[cfg(test)]
mod dock_tests;
#[cfg(test)]
mod floating_tests;
#[cfg(test)]
mod group_tests;
#[cfg(test)]
mod iconic_tests;
#[cfg(test)]
mod model_tests;
#[cfg(all(test, feature = "native-windows"))]
mod native_tests;
#[cfg(test)]
mod split_tests;
#[cfg(test)]
mod tests;
