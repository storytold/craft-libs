//! Groups, columns and the targets of whole-container moves.

use std::hash::Hash;

use emath::Rect;

use super::{DockSide, action::DockPlacement, edit::node_at, layout::Layout, node::Node, policy::DockError};

/// The window edge a [`Node::Dock`] sits on, in reading order so right-to-left interfaces
/// mirror. `Start` is the edge of a horizontal split's first child; the renderer maps it to a physical edge from
/// the layout direction.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[non_exhaustive]
pub enum DockEdge {
    /// The first edge in reading order (left in a left-to-right layout).
    Start,
    /// The last edge in reading order (right in a left-to-right layout).
    End,
}

/// Tabs that move, minimize and size together in a dock column.
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(bound(deserialize = "P: serde::Deserialize<'de>"))]
#[non_exhaustive]
pub struct Group<P> {
    /// The panels, in tab order.
    pub panels: Vec<P>,
    /// Index into `panels` of the selected tab.
    pub active: usize,
    /// Only the tab strip shows. `height` is kept for when the group expands again.
    #[serde(default)]
    pub minimized: bool,
    /// Preferred height in points, set when the user resizes the group; `None` shares the free
    /// height of the column or stack, or in a dock column takes the renderer's default height.
    #[serde(default)]
    pub height: Option<f32>,
    /// Width in points of the group's flyout, set when the user resizes it; `None` takes the column's expanded width.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub flyout_width: Option<f32>,
}

impl<P> Group<P> {
    /// A group of `panels` with tab `active` selected, expanded and with no preferred height or flyout width.
    pub fn new(panels: Vec<P>, active: usize) -> Self {
        Self { panels, active, minimized: false, height: None, flyout_width: None }
    }

    /// The selected panel, or `None` when `active` is out of range.
    pub fn active_panel(&self) -> Option<&P> {
        self.panels.get(self.active)
    }
}

/// One column of a dock: groups from top to bottom.
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(bound(deserialize = "P: serde::Deserialize<'de>"))]
#[non_exhaustive]
pub struct Column<P> {
    /// The groups, top to bottom.
    pub groups: Vec<Group<P>>,
    /// Expanded width in points once the user has resized the column; the theme's column width until then.
    #[serde(default)]
    pub width: Option<f32>,
    /// Collapsed to one icon per panel.
    #[serde(default)]
    pub iconic: bool,
    /// Width while iconic; labels show beside the icons when it is wide enough.
    #[serde(default)]
    pub iconic_width: Option<f32>,
}

impl GroupLocation {
    /// A group location with the given state and column.
    pub const fn new(minimized: bool, height: Option<f32>, column: Option<ColumnLocation>) -> Self {
        Self { minimized, height, flyout_width: None, column }
    }

    /// The location with the group's flyout `width` points wide.
    #[must_use]
    pub const fn with_flyout_width(mut self, width: Option<f32>) -> Self {
        self.flyout_width = width;
        self
    }
}

impl ColumnLocation {
    /// A column location on `side`, `width` points wide when expanded (the theme's column width for `None`).
    pub const fn new(side: DockEdge, width: Option<f32>, iconic: bool, iconic_width: Option<f32>) -> Self {
        Self { side, width, iconic, iconic_width }
    }
}

impl<P> Column<P> {
    /// A column of `groups`, expanded at the theme's column width until the user resizes it.
    pub fn new(groups: Vec<Group<P>>) -> Self {
        Self { groups, width: None, iconic: false, iconic_width: None }
    }

    /// The column resized to `width` points.
    #[must_use]
    pub fn with_width(mut self, width: f32) -> Self {
        self.width = Some(width);
        self
    }
}

/// Where a group, column or floating window goes.
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
#[non_exhaustive]
pub enum Target<P> {
    /// Relative to the panel `anchor`, which must not be part of what moves.
    #[non_exhaustive]
    At {
        /// The panel to place the moved groups next to.
        anchor: P,
        /// Where, relative to `anchor`.
        placement: DockPlacement<P>,
    },
    /// A new dock on an edge that has none (or an empty one). The root becomes the other side
    /// of a horizontal split.
    #[non_exhaustive]
    NewDock {
        /// The edge to put the dock on.
        side: DockEdge,
    },
    /// A new floating window.
    #[non_exhaustive]
    Float {
        /// Window rect in logical points, which the caller takes from its `FloatSpace`. Saved as
        /// `[x, y, width, height]`.
        #[serde(with = "crate::dock::rect_serde")]
        rect: Rect,
    },
}

impl<P: Clone + Eq + Hash> DockPlacement<P> {
    /// The placement for a drop on the `side` of `anchor`'s group, in the vocabulary its container accepts.
    ///
    /// | Anchor in | `Center` | `Top`, `Bottom` | `Start`, `End` |
    /// |---|---|---|---|
    /// | the split tree or a floating window | [`DockPlacement::Split`] | `Split` | `Split` |
    /// | a dock column | `Split(Center)` (join as tabs) | [`DockPlacement::Group`] above or below | [`DockPlacement::Column`] on that side |
    ///
    /// A dock's `Column { after }` counts from its window edge, which is the dock's [`DockEdge`] in reading order;
    /// this picks `after` so the new column lands on the side of the anchor's column.
    pub fn for_zone(layout: &Layout<P>, anchor: &P, side: DockSide) -> Result<Self, DockError> {
        let place = layout.place(anchor).ok_or(DockError::MissingTarget)?;
        Ok(match (place, side) {
            (PanelPlace::Node(_) | PanelPlace::Floating { .. }, side) | (_, side @ DockSide::Center) => DockPlacement::Split(side),
            (PanelPlace::Dock { .. }, DockSide::Top | DockSide::Bottom) => DockPlacement::Group { below: side == DockSide::Bottom, height: None },
            (PanelPlace::Dock { path, .. }, _) => {
                let edge = match layout.root.as_ref().and_then(|root| node_at(root, &path)) {
                    Some(Node::Dock { side, .. }) => *side,
                    _ => return Err(DockError::MissingTarget),
                };
                // `after` is away from the window edge: towards the end for a start dock.
                DockPlacement::Column { after: (side == DockSide::End) == (edge == DockEdge::Start) }
            }
        })
    }
}

/// What the Window menu shows for a panel. The model does not track the z-order of floating
/// windows, so `Front` means "showing": the active tab of an expanded group in an expanded column.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum PanelState {
    /// The panel is closed.
    Hidden,
    /// The panel is in the layout but not showing: not the active tab, or in a minimized group,
    /// collapsed accordion entry or iconic column.
    Open,
    /// The panel is showing.
    Front,
}

/// The group a closed panel was in, when that group was in a dock.
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
#[non_exhaustive]
pub struct GroupLocation {
    /// Whether the group was minimized.
    pub minimized: bool,
    /// The group's preferred height in points.
    pub height: Option<f32>,
    /// The width in points of the group's flyout, if the user had resized it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub flyout_width: Option<f32>,
    /// The column, when the group was docked.
    pub column: Option<ColumnLocation>,
}

/// The column a closed panel's group was in.
#[derive(Clone, Copy, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
#[non_exhaustive]
pub struct ColumnLocation {
    /// The dock edge the column was on.
    pub side: DockEdge,
    /// Expanded width in points, if the user had resized the column.
    pub width: Option<f32>,
    /// Whether the column was collapsed to icons.
    pub iconic: bool,
    /// Width in points while iconic.
    pub iconic_width: Option<f32>,
}

/// Where a panel is. Paths lead from the root: false is a split's first child, true its second.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum PanelPlace {
    /// A `Tabs` or `Stack` node.
    Node(Vec<bool>),
    /// A group of a dock column.
    #[non_exhaustive]
    Dock {
        /// The path of the `Dock` node.
        path: Vec<bool>,
        /// The column's index.
        column: usize,
        /// The group's index in the column.
        group: usize,
    },
    /// A `Tabs` or `Stack` node of a floating window's tree.
    #[non_exhaustive]
    Floating {
        /// The window's index.
        window: usize,
        /// The path of the node in the window's tree.
        path: Vec<bool>,
    },
}

impl PanelPlace {
    /// The tree (`None` for the docked layout, else a floating window) and the path of a
    /// `Tabs` or `Stack` node.
    pub fn tree_path(&self) -> Option<(Option<usize>, &[bool])> {
        match self {
            PanelPlace::Node(path) => Some((None, path)),
            PanelPlace::Floating { window, path } => Some((Some(*window), path)),
            PanelPlace::Dock { .. } => None,
        }
    }
}

/// Which container a whole-container action names.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Unit {
    Group,
    Column,
    Window,
}

/// Groups in transit, with the size and state of the container they came from.
pub(super) struct Moving<P> {
    pub groups: Vec<Group<P>>,
    /// A floating window's whole tree, whose leaves are `groups`. A split or a new window
    /// takes the tree; docks take the groups.
    pub node: Option<Node<P>>,
    pub minimized: bool,
    pub width: Option<f32>,
    pub iconic: bool,
    pub iconic_width: Option<f32>,
    /// The id of a whole floating window in transit, which a new window keeps.
    pub id: Option<u64>,
}

impl<P> Moving<P> {
    pub(super) fn group(group: Group<P>) -> Self {
        Self { groups: vec![group], node: None, minimized: false, width: None, iconic: false, iconic_width: None, id: None }
    }

    pub(super) fn into_column(self, width: Option<f32>) -> Column<P> {
        Column { groups: self.groups, width: self.width.or(width), iconic: self.iconic, iconic_width: self.iconic_width }
    }
}

impl<P> Target<P> {
    /// Builds [`Target::At`].
    pub fn at(anchor: P, placement: DockPlacement<P>) -> Self {
        Self::At { anchor, placement }
    }

    /// Builds [`Target::NewDock`].
    pub fn new_dock(side: DockEdge) -> Self {
        Self::NewDock { side }
    }

    /// Builds [`Target::Float`].
    pub fn float(rect: Rect) -> Self {
        Self::Float { rect }
    }
}
