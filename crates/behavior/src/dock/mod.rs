//! The dock model: the tree of splits over panel frames, the side of a group a drop lands on (20.6), and the rules
//! that edit it.
//!
//! The tree and its rects are in reading order: a split's first child is its start (or top) child. A caller under
//! right-to-left maps each rect with `Flow::to_physical`, which mirrors the workspace.
//!
//! The panel layout of a dock ([`Layout`]) is the persisted model: docked tree, floating windows, the actions that
//! edit it and the rules that validate it.
//!
//! The query surface a renderer reads: [`Layout::place`] and [`Layout::places`] (where each panel sits),
//! [`Layout::node_of`], [`Layout::column`] and [`Layout::group`] (the container at a place), [`Layout::dock_path`],
//! [`Layout::edge_path`], [`Layout::edge_narrows`] and [`Layout::edge_anchor`] (the outer-edge frames),
//! [`Layout::shape`] and [`Layout::same_arrangement`] (the structure without sizes), and the tree functions
//! [`node_at`], [`first_active`] and [`tree_panels`]. Everything else in the editing files is private to the model.

use crate::flow::Axis;
use emath::{Pos2, Rect};

mod action;
mod edit;
mod groups;
mod inner;
mod layout;
mod limits;
mod node;
mod policy;
mod split;
mod zones;

mod apply;
mod arrange;
mod rect_serde;
mod windows;
mod workspace;
mod workspaces;

pub use action::{Action, DockPlacement, Location};
pub use edit::{first_active, node_at, tree_panels};
pub use groups::{Column, ColumnLocation, DockEdge, Group, GroupLocation, PanelPlace, PanelState, Target};
pub use layout::Layout;
pub use limits::{MAX_DEPTH, MAX_PANELS};
pub use node::{Floating, Node, StackEntry};
pub use policy::{DockError, Permissions};
pub use split::SplitSize;
pub use zones::{SIDE_ZONE_DEPTH_SHARE, SIDE_ZONE_FRACTION, ZoneSize, zones};

pub use arrange::{Arrangement, arrange_rects};
pub use windows::{DocumentGroup, DocumentWindows};
pub use workspace::Workspace;
pub use workspaces::{NamedWorkspace, WorkspaceChrome, WorkspaceError, WorkspaceSet};

/// The side of a group a dropped panel docks on, in reading order: `Start` is the left half in a left-to-right
/// window and the right half in a right-to-left one. Saved layouts and actions written with the physical names
/// `Left` and `Right` load as `Start` and `End`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[non_exhaustive]
pub enum DockSide {
    /// Before the group along the reading direction.
    #[serde(alias = "Left", alias = "left")]
    Start,
    /// After the group along the reading direction.
    #[serde(alias = "Right", alias = "right")]
    End,
    /// Above the group.
    Top,
    /// Below the group.
    Bottom,
    /// Into the group's tabs.
    Center,
}

impl DockSide {
    /// The axis the side splits along; `None` for `Center`.
    pub const fn axis(self) -> Option<Axis> {
        match self {
            DockSide::Start | DockSide::End => Some(Axis::X),
            DockSide::Top | DockSide::Bottom => Some(Axis::Y),
            DockSide::Center => None,
        }
    }

    /// The part of `group` a panel docked on this side takes: the half on that side, or the whole group.
    pub fn area(self, group: Rect) -> Rect {
        let center = group.center();
        match self {
            DockSide::Start => Rect::from_min_max(group.min, Pos2::new(center.x, group.bottom())),
            DockSide::End => Rect::from_min_max(Pos2::new(center.x, group.top()), group.max),
            DockSide::Top => Rect::from_min_max(group.min, Pos2::new(group.right(), center.y)),
            DockSide::Bottom => Rect::from_min_max(Pos2::new(group.left(), center.y), group.max),
            DockSide::Center => group,
        }
    }
}
