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

/// Tabs that move, minimize and size together in a column, docked or floating.
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
    /// Offset in points of the flyout's top from its icons' top (negative is above), set when the user
    /// resizes the flyout from its top edge; `None` aligns the flyout with its icons.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub flyout_offset: Option<f32>,
}

impl<P> Group<P> {
    /// A group of `panels` with tab `active` selected, expanded and with no preferred height or flyout geometry.
    pub fn new(panels: Vec<P>, active: usize) -> Self {
        Self { panels, active, minimized: false, height: None, flyout_width: None, flyout_offset: None }
    }

    /// The selected panel, or `None` when `active` is out of range.
    pub fn active_panel(&self) -> Option<&P> {
        self.panels.get(self.active)
    }
}

/// One column of groups from top to bottom: a column of a dock, or the whole of a floating window.
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(bound(deserialize = "P: serde::Deserialize<'de>"))]
#[non_exhaustive]
pub struct Column<P> {
    /// The groups, top to bottom.
    pub groups: Vec<Group<P>>,
    /// Expanded width in points once the user has resized the column; the theme's column width until then.
    /// A floating window's width is its rect's, so a floating column leaves this `None`.
    #[serde(default)]
    pub width: Option<f32>,
    /// Collapsed to one icon per panel. The one collapsed state of every column, docked or floating.
    #[serde(default)]
    pub iconic: bool,
    /// Width while iconic; labels show beside the icons when it is wide enough.
    #[serde(default)]
    pub iconic_width: Option<f32>,
    /// The column's width as a count of its panel's own content columns, for a panel that lays its content in a
    /// grid (a toolbar). When set, `width` is unused and the column is as tall as its content, docked or floating;
    /// the renderer computes the shown size from the panel. At least 1.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub steps: Option<u16>,
}

impl GroupLocation {
    /// A group location with the given state and column.
    pub const fn new(minimized: bool, height: Option<f32>, column: Option<ColumnLocation>) -> Self {
        Self { minimized, height, flyout_width: None, flyout_offset: None, column }
    }

    /// The location with the group's flyout `width` points wide.
    #[must_use]
    pub const fn with_flyout_width(mut self, width: Option<f32>) -> Self {
        self.flyout_width = width;
        self
    }

    /// The location with the group's flyout top `offset` points from its icons' top.
    #[must_use]
    pub const fn with_flyout_offset(mut self, offset: Option<f32>) -> Self {
        self.flyout_offset = offset;
        self
    }
}

impl ColumnLocation {
    /// A column location on `side` (`None` for a floating window's column), `width` points wide when expanded (the
    /// theme's column width for `None`).
    pub const fn new(side: Option<DockEdge>, width: Option<f32>, iconic: bool, iconic_width: Option<f32>) -> Self {
        Self { side, width, iconic, iconic_width, steps: None }
    }

    /// The location with the column `steps` content columns wide ([`Column::steps`]).
    #[must_use]
    pub const fn with_steps(mut self, steps: Option<u16>) -> Self {
        self.steps = steps;
        self
    }

    /// The location of `column`, docked on `side` or floating (`None`).
    pub(super) fn of<P>(column: &Column<P>, side: Option<DockEdge>) -> Self {
        Self { side, width: column.width, iconic: column.iconic, iconic_width: column.iconic_width, steps: column.steps }
    }
}

impl<P> Column<P> {
    /// A column of `groups`, expanded at the theme's column width until the user resizes it.
    pub fn new(groups: Vec<Group<P>>) -> Self {
        Self { groups, width: None, iconic: false, iconic_width: None, steps: None }
    }

    /// The column resized to `width` points.
    #[must_use]
    pub fn with_width(mut self, width: f32) -> Self {
        self.width = Some(width);
        self
    }

    /// The column collapsed to icons, or expanded.
    #[must_use]
    pub fn with_iconic(mut self, iconic: bool) -> Self {
        self.iconic = iconic;
        self
    }

    /// The column `steps` content columns wide ([`Self::steps`]).
    #[must_use]
    pub fn with_steps(mut self, steps: u16) -> Self {
        self.steps = Some(steps);
        self
    }

    /// Every panel of the column, top group first.
    pub fn panels(&self) -> impl Iterator<Item = &P> {
        self.groups.iter().flat_map(|g| &g.panels)
    }

    /// The column's state without its groups: what a new column made from groups taken out of this one keeps.
    pub(super) fn emptied(&self) -> Self {
        Self { groups: Vec::new(), width: self.width, iconic: self.iconic, iconic_width: self.iconic_width, steps: self.steps }
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
        /// The new window's [`Floating::edge`](super::Floating::edge) when its column is collapsed: the edge the area
        /// decided from where the strip was dropped. `None` (and any expanded column) leaves the edge to the area,
        /// nearer `rect`.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        edge: Option<DockEdge>,
    },
}

impl<P: Clone + Eq + Hash> DockPlacement<P> {
    /// The placement for a drop on the `side` of `anchor`'s group, in the vocabulary its container accepts.
    ///
    /// | Anchor in | `Center` | `Top`, `Bottom` | `Start`, `End` |
    /// |---|---|---|---|
    /// | the split tree | [`DockPlacement::Split`] | `Split` | `Split` |
    /// | a dock column | `Split(Center)` (join as tabs) | [`DockPlacement::Group`] above or below | [`DockPlacement::Column`] on that side |
    /// | a floating window | `Split(Center)` (join as tabs) | `Group` above or below | refused ([`DockError::InvalidPlacement`]) |
    ///
    /// A floating window is one column, so nothing goes beside it. A dock's `Column { after }` counts from its
    /// window edge, which is the dock's [`DockEdge`] in reading order; this picks `after` so the new column lands on
    /// the side of the anchor's column.
    pub fn for_zone(layout: &Layout<P>, anchor: &P, side: DockSide) -> Result<Self, DockError> {
        let place = layout.place(anchor).ok_or(DockError::MissingTarget)?;
        Ok(match (place, side) {
            (PanelPlace::Node(_), side) | (_, side @ DockSide::Center) => DockPlacement::Split(side),
            (PanelPlace::Dock { .. } | PanelPlace::Floating { .. }, DockSide::Top | DockSide::Bottom) => {
                DockPlacement::Group { below: side == DockSide::Bottom, height: None }
            }
            (PanelPlace::Floating { .. }, _) => return Err(DockError::InvalidPlacement),
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
/// windows, so `Front` means "showing": the active tab of an expanded group in an expanded column, docked or
/// floating.
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

/// The group a closed panel was in, when that group was in a column, docked or floating.
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
    /// The offset in points of the group's flyout top from its icons' top, if the user had moved it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub flyout_offset: Option<f32>,
    /// The column the group was in.
    pub column: Option<ColumnLocation>,
}

/// The column a closed panel's group was in.
#[derive(Clone, Copy, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
#[non_exhaustive]
pub struct ColumnLocation {
    /// The dock edge the column was on; `None` for a floating window's column.
    #[serde(default)]
    pub side: Option<DockEdge>,
    /// Expanded width in points, if the user had resized the column.
    pub width: Option<f32>,
    /// Whether the column was collapsed to icons.
    pub iconic: bool,
    /// Width in points while iconic.
    pub iconic_width: Option<f32>,
    /// The column's width in content columns ([`Column::steps`]).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub steps: Option<u16>,
}

/// Where a panel is. Paths lead from the root: false is a split's first child, true its second.
/// A dock column and a floating window are both a column of groups, so `Dock` and `Floating` name a group the
/// same way.
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
    /// A group of a floating window's column.
    #[non_exhaustive]
    Floating {
        /// The window's index.
        window: usize,
        /// The group's index in the window's column.
        group: usize,
    },
}

impl PanelPlace {
    /// The path of a `Tabs` or `Stack` node of the docked tree; `None` for a group of a column.
    pub fn tree_path(&self) -> Option<&[bool]> {
        match self {
            PanelPlace::Node(path) => Some(path),
            PanelPlace::Dock { .. } | PanelPlace::Floating { .. } => None,
        }
    }

    /// The group's index in its column; `None` for a `Tabs` or `Stack` node of the docked tree.
    pub fn group_index(&self) -> Option<usize> {
        match self {
            PanelPlace::Dock { group, .. } | PanelPlace::Floating { group, .. } => Some(*group),
            PanelPlace::Node(_) => None,
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

/// Groups in transit, in a column that carries the state of the column they came from: a new column, docked or
/// floating, keeps that state (an icon group taken out of a collapsed column stays icons), and a column they
/// join keeps its own.
pub(super) struct Moving<P> {
    pub column: Column<P>,
    /// The id of a floating window whose last group is in transit, which a new window keeps.
    pub id: Option<u64>,
}

impl<P> Moving<P> {
    /// `group` with no column state: a new column made of it is expanded at the theme's width.
    pub(super) fn group(group: Group<P>) -> Self {
        Self::taken(group, Column::new(Vec::new()))
    }

    /// `group` taken out of a column whose state is `source` ([`Column::emptied`]).
    pub(super) fn taken(group: Group<P>, source: Column<P>) -> Self {
        Self { column: Column { groups: vec![group], ..source }, id: None }
    }

    /// The column a new dock column gets: the moving column, `width` points wide unless it brings a width.
    pub(super) fn into_column(self, width: Option<f32>) -> Column<P> {
        Column { width: self.column.width.or(width), ..self.column }
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

    /// Builds [`Target::Float`] with no edge: the area takes the edge nearer `rect`.
    pub fn float(rect: Rect) -> Self {
        Self::Float { rect, edge: None }
    }

    /// Builds [`Target::Float`] with the edge a collapsed window keeps to.
    pub fn float_with_edge(rect: Rect, edge: Option<DockEdge>) -> Self {
        Self::Float { rect, edge }
    }
}
