//! Shared panel docking: transactional layout changes and egui drag/drop presentation.
//!
//! Panel IDs identify application-owned contents, never document state or native windows. A
//! layout can be adapted to an existing workspace schema or serialized with the `serde` feature.
//! Validate deserialized layouts before rendering them. Every transaction checks its complete
//! input and commits only a valid result; a rejected move cannot lose the source panel.

mod between;
mod geometry;
mod model;
mod resize;
mod strips;
mod surface;
mod targets;

pub use between::{BetweenRequest, insert_between};
pub use geometry::geometry_transition_active;
pub use model::{Action, DockError, Floating, Layout, Location, MAX_DEPTH, MAX_PANELS, Node, Permissions, Placement, StackEntry, Zone};
pub use resize::AnchoredResize;
pub use surface::{
    DockArea, DockContent, DockOutput, DockStyle, DropFeedback, DropRequest, PanelLimits, begin_drag, cancel_drag, is_dragging, measure_layout,
};
pub use surface::{LayoutMeasurement, SplitBoundary, measure_layout_with_boundaries};
pub use targets::{DropProposal, DropTarget, drop_zone, resolve_drop, tab_insertion_marker, zone_rect};

#[cfg(test)]
mod tests;
