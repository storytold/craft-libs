//! Where a panel goes ([`DockPlacement`]), where it was ([`Location`]) and the transaction unit ([`Action`]).

use emath::Rect;

use crate::placement::Placement;

use super::{
    DockSide,
    groups::{GroupLocation, Target},
    policy::Permissions,
    split::SplitSize,
};

/// Where a moved panel goes relative to an anchor panel.
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
#[non_exhaustive]
pub enum DockPlacement<P> {
    /// Insert before this existing destination member, or append. Also reorders a group.
    #[non_exhaustive]
    Tab {
        /// The member to insert before; `None` appends.
        before: Option<P>,
    },
    /// Center inserts after the anchor; sides split the anchor's frame into two halves, and
    /// every other frame keeps its size. Sides apply to `Tabs` and `Stack` nodes only.
    Split(DockSide),
    /// A side [`DockPlacement::Split`] that gives the new frame its own room instead of half the
    /// anchor's frame: the new split takes `size`, and `outer` replaces the size of the nearest
    /// enclosing split whose divider runs along the anchor frame's `zone` side, so the frames
    /// beyond that divider give the room. An anchor at the edge of its area on that side takes
    /// the nearest such split along its opposite side instead. The renderer offers it beside a
    /// frame too small to halve. `Center`, and an anchor without either divider or with a dock
    /// beyond it, are invalid.
    #[non_exhaustive]
    SplitSized {
        /// The side of the anchor frame.
        zone: DockSide,
        /// Size of the new split.
        size: SplitSize,
        /// New size of the enclosing split that gives up the room.
        outer: SplitSize,
    },
    /// A new group above or below the anchor's group in a dock column. On an anchor in a
    /// floating window, a split of the anchor's group above or below.
    /// `height` is the inserted group's preferred height ([`Group::height`]); `None`
    /// keeps a moved group's own height, or shares the free height for a new group.
    #[non_exhaustive]
    Group {
        /// Places the group below the anchor's group; false places it above.
        below: bool,
        /// Preferred height in points; `None` shares the free height.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        height: Option<f32>,
    },
    /// A new column before (towards the window edge) or after the anchor's column.
    #[non_exhaustive]
    Column {
        /// Places the new column after the anchor's column; false places it before.
        after: bool,
    },
    /// A new frame along one edge of the docked layout, a quarter of its width or height: the
    /// root is split, or in a layout with docks, the part between them. The anchor names any
    /// docked panel. A protected anchor ([`Permissions::is_protected`], applied through
    /// [`Layout::apply_with_permissions`]) narrows the part to the canvas column: where neither
    /// side of a side-by-side split is a dock and only one holds a protected panel, that side;
    /// for `Top` (`Bottom`), also the first (last) row of a split of rows when only that row
    /// holds one. `Center` is invalid.
    Edge(DockSide),
}

/// A closed panel's former placement. Applications may retain this beside visibility flags.
/// Anchors are stable panel identities rather than mutable tree indices.
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(bound(deserialize = "P: serde::Deserialize<'de>"))]
#[non_exhaustive]
pub struct Location<P> {
    /// The window rect when the panel was floating. Saved as `[x, y, width, height]`.
    #[serde(with = "crate::dock::rect_serde::option", default)]
    pub floating: Option<Rect>,
    /// The panel the placement is relative to.
    pub anchor: Option<P>,
    /// Where the panel sat relative to `anchor`.
    pub placement: DockPlacement<P>,
    /// The size of the split the panel's frame was in, when it had one.
    pub size: Option<SplitSize>,
    /// Accordion state to restore if the destination is still an accordion.
    pub stack: Option<(bool, Option<f32>)>,
    /// A split's complete sibling subtree. This retains placement around a nested workspace,
    /// rather than accidentally inserting beside only its first leaf when restoring.
    #[serde(default)]
    pub sibling: Vec<P>,
    /// Group and column state when the panel was in a dock. Applies when
    /// restoring creates a new group, column or dock for the panel.
    #[serde(default)]
    pub group: Option<GroupLocation>,
}

/// Groups, columns and floating windows are named by a panel they contain.
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
#[non_exhaustive]
pub enum Action<P> {
    /// Move one panel next to `anchor`.
    #[non_exhaustive]
    Move {
        /// The panel the action applies to.
        panel: P,
        /// The panel the destination is relative to.
        anchor: P,
        /// Where the panel goes relative to the anchor.
        placement: DockPlacement<P>,
    },
    /// Float the panel's group in a window at `rect`.
    #[non_exhaustive]
    Float {
        /// The panel the action applies to.
        panel: P,
        /// The window rect in logical points. Saved as `[x, y, width, height]`.
        #[serde(with = "crate::dock::rect_serde")]
        rect: Rect,
    },
    /// Close a panel.
    #[non_exhaustive]
    Close {
        /// The panel the action applies to.
        panel: P,
    },
    /// Select a panel in its group.
    #[non_exhaustive]
    Activate {
        /// The panel the action applies to.
        panel: P,
    },
    /// Reveal a registered, currently absent panel. Applications validate their panel registry.
    /// `anchor` names the group it joins as the last tab. Without one it becomes the root of an
    /// empty layout, else joins the first dock group that accepts tabs and is shown (not
    /// minimized, not in an iconic column; the end-side dock's first), else gets a new frame a
    /// quarter wide at the end edge of the docked area
    /// ([`DockPlacement::Edge`]).
    #[non_exhaustive]
    Open {
        /// The panel the action applies to.
        panel: P,
        /// The panel the destination is relative to.
        anchor: Option<P>,
    },
    /// Reveal an absent panel beside a protected canvas or at another explicit destination.
    #[non_exhaustive]
    OpenAt {
        /// The panel the action applies to.
        panel: P,
        /// The panel the destination is relative to.
        anchor: P,
        /// Where the panel goes relative to the anchor.
        placement: DockPlacement<P>,
    },
    /// Reposition/resize a floating window, identified by one of its panels.
    #[non_exhaustive]
    MoveFloating {
        /// The panel the action applies to.
        panel: P,
        /// The window rect in logical points. Saved as `[x, y, width, height]`.
        #[serde(with = "crate::dock::rect_serde")]
        rect: Rect,
    },
    /// Path from the root: false is the first child, true the second.
    #[non_exhaustive]
    ResizeSplit {
        /// Path from the root: false is the first child, true the second.
        path: Vec<bool>,
        /// The new size.
        size: SplitSize,
    },
    /// [`Action::ResizeSplit`] in the floating window holding `panel`, with a path from its
    /// root.
    #[non_exhaustive]
    ResizeWindowSplit {
        /// The panel the action applies to.
        panel: P,
        /// Path from the root: false is the first child, true the second.
        path: Vec<bool>,
        /// The new size.
        size: SplitSize,
    },
    /// Open or close the panel's entry in its accordion stack.
    #[non_exhaustive]
    SetStackOpen {
        /// The panel the action applies to.
        panel: P,
        /// The section shows its body.
        open: bool,
    },
    /// Set the panel's accordion entry height in points; `None` clears it.
    #[non_exhaustive]
    ResizeStack {
        /// The panel the action applies to.
        panel: P,
        /// Preferred height in points; `None` shares the free height.
        height: Option<f32>,
    },
    /// Move the group containing `panel` with its order, active tab and minimized state.
    /// `Target::At` with `Tab` or `Split(Center)` merges every member. An accordion entry is a
    /// group of one. A target inside the group is a no-op.
    #[non_exhaustive]
    MoveGroup {
        /// The panel the action applies to.
        panel: P,
        /// The destination.
        to: Target<P>,
    },
    /// Move the dock group containing `panel` one place up or down its column, past the
    /// neighbouring group. Fails for a group that is not docked in a column
    /// ([`DockError::InvalidPlacement`]), at the column's end ([`DockError::MissingTarget`]), or
    /// beside a group that cannot move ([`DockError::NotAllowed`]; a canvas is never stepped past).
    #[non_exhaustive]
    StepGroup {
        /// The panel the action applies to.
        panel: P,
        /// Moves towards the start; false moves towards the end.
        up: bool,
    },
    /// Move the dock column containing `panel`. `Float` makes a window of its groups.
    #[non_exhaustive]
    MoveColumn {
        /// The panel the action applies to.
        panel: P,
        /// The destination.
        to: Target<P>,
    },
    /// Dock or merge the floating window containing `panel`, every group included.
    #[non_exhaustive]
    MoveWindow {
        /// The panel the action applies to.
        panel: P,
        /// The destination.
        to: Target<P>,
    },
    /// Close Tab Group.
    #[non_exhaustive]
    CloseGroup {
        /// The panel the action applies to.
        panel: P,
    },
    /// × on a floating window: closes all its panels.
    #[non_exhaustive]
    CloseWindow {
        /// The panel the action applies to.
        panel: P,
    },
    /// Close Other Panels in Group.
    #[non_exhaustive]
    CloseOthers {
        /// The panel the action applies to.
        panel: P,
    },
    /// Minimize a dock group to its tab strip, or the floating window holding the panel to its
    /// title bar (and the strip of its only group). On an accordion entry, closes it.
    #[non_exhaustive]
    SetMinimized {
        /// The panel the action applies to.
        panel: P,
        /// Only the title bar or tab strip shows.
        minimized: bool,
    },
    /// Collapse the panel's dock column to icons, or expand it.
    #[non_exhaustive]
    SetIconic {
        /// The panel the action applies to.
        panel: P,
        /// Collapses the column to an icon strip.
        iconic: bool,
    },
    /// Set the expanded width in points of the panel's dock column.
    #[non_exhaustive]
    ResizeColumn {
        /// The panel the action applies to.
        panel: P,
        /// Width in points.
        width: f32,
    },
    /// Preferred height of a dock group or of an accordion entry.
    #[non_exhaustive]
    ResizeGroup {
        /// The panel the action applies to.
        panel: P,
        /// Preferred height in points; `None` shares the free height.
        height: Option<f32>,
    },
    /// Set the width in points of the flyout of the dock group holding the panel; `None` takes the column's width.
    #[non_exhaustive]
    ResizeFlyoutWidth {
        /// The panel the action applies to.
        panel: P,
        /// Width in points.
        width: Option<f32>,
    },
    /// Set the width in points of the panel's dock column while iconic; `None` clears it.
    #[non_exhaustive]
    ResizeIconic {
        /// The panel the action applies to.
        panel: P,
        /// Width in points.
        width: Option<f32>,
    },
    /// Put the floating window in its own OS window at `placement`, or back in the app window.
    #[non_exhaustive]
    SetNative {
        /// The panel the action applies to.
        panel: P,
        /// Where the OS window is on the desktop.
        #[serde(default)]
        placement: Option<Placement>,
    },
    /// Move one panel, without the rest of its group. `Target::At` is [`Action::Move`];
    /// `NewDock` and `Float` give the panel a group of its own.
    #[non_exhaustive]
    MovePanel {
        /// The panel the action applies to.
        panel: P,
        /// The destination.
        to: Target<P>,
    },
}

impl<P> Location<P> {
    /// A location relative to `anchor` by `placement`, with no recorded sizes or siblings.
    pub fn new(anchor: Option<P>, placement: DockPlacement<P>) -> Self {
        Self { floating: None, anchor, placement, size: None, stack: None, sibling: Vec::new(), group: None }
    }
}

impl<P> Action<P> {
    /// Builds [`Action::Move`].
    pub fn move_beside(panel: P, anchor: P, placement: DockPlacement<P>) -> Self {
        Self::Move { panel, anchor, placement }
    }

    /// Builds [`Action::Float`].
    pub fn float(panel: P, rect: Rect) -> Self {
        Self::Float { panel, rect }
    }

    /// Builds [`Action::Close`].
    pub fn close(panel: P) -> Self {
        Self::Close { panel }
    }

    /// Builds [`Action::Activate`].
    pub fn activate(panel: P) -> Self {
        Self::Activate { panel }
    }

    /// Builds [`Action::Open`].
    pub fn open(panel: P, anchor: Option<P>) -> Self {
        Self::Open { panel, anchor }
    }

    /// Builds [`Action::OpenAt`].
    pub fn open_at(panel: P, anchor: P, placement: DockPlacement<P>) -> Self {
        Self::OpenAt { panel, anchor, placement }
    }

    /// Builds [`Action::MoveFloating`].
    pub fn move_floating(panel: P, rect: Rect) -> Self {
        Self::MoveFloating { panel, rect }
    }

    /// Builds [`Action::ResizeSplit`].
    pub fn resize_split(path: Vec<bool>, size: SplitSize) -> Self {
        Self::ResizeSplit { path, size }
    }

    /// Builds [`Action::ResizeWindowSplit`].
    pub fn resize_window_split(panel: P, path: Vec<bool>, size: SplitSize) -> Self {
        Self::ResizeWindowSplit { panel, path, size }
    }

    /// Builds [`Action::SetStackOpen`].
    pub fn set_stack_open(panel: P, open: bool) -> Self {
        Self::SetStackOpen { panel, open }
    }

    /// Builds [`Action::ResizeStack`].
    pub fn resize_stack(panel: P, height: Option<f32>) -> Self {
        Self::ResizeStack { panel, height }
    }

    /// Builds [`Action::MoveGroup`].
    pub fn move_group(panel: P, to: Target<P>) -> Self {
        Self::MoveGroup { panel, to }
    }

    /// Builds [`Action::StepGroup`].
    pub fn step_group(panel: P, up: bool) -> Self {
        Self::StepGroup { panel, up }
    }

    /// Builds [`Action::MoveColumn`].
    pub fn move_column(panel: P, to: Target<P>) -> Self {
        Self::MoveColumn { panel, to }
    }

    /// Builds [`Action::MoveWindow`].
    pub fn move_window(panel: P, to: Target<P>) -> Self {
        Self::MoveWindow { panel, to }
    }

    /// Builds [`Action::CloseGroup`].
    pub fn close_group(panel: P) -> Self {
        Self::CloseGroup { panel }
    }

    /// Builds [`Action::CloseWindow`].
    pub fn close_window(panel: P) -> Self {
        Self::CloseWindow { panel }
    }

    /// Builds [`Action::CloseOthers`].
    pub fn close_others(panel: P) -> Self {
        Self::CloseOthers { panel }
    }

    /// Builds [`Action::SetMinimized`].
    pub fn set_minimized(panel: P, minimized: bool) -> Self {
        Self::SetMinimized { panel, minimized }
    }

    /// Builds [`Action::SetIconic`].
    pub fn set_iconic(panel: P, iconic: bool) -> Self {
        Self::SetIconic { panel, iconic }
    }

    /// Builds [`Action::ResizeColumn`].
    pub fn resize_column(panel: P, width: f32) -> Self {
        Self::ResizeColumn { panel, width }
    }

    /// Builds [`Action::ResizeGroup`].
    pub fn resize_group(panel: P, height: Option<f32>) -> Self {
        Self::ResizeGroup { panel, height }
    }

    /// Builds [`Action::ResizeFlyoutWidth`].
    pub fn resize_flyout_width(panel: P, width: Option<f32>) -> Self {
        Self::ResizeFlyoutWidth { panel, width }
    }

    /// Builds [`Action::ResizeIconic`].
    pub fn resize_iconic(panel: P, width: Option<f32>) -> Self {
        Self::ResizeIconic { panel, width }
    }

    /// Builds [`Action::SetNative`].
    pub fn set_native(panel: P, placement: Option<Placement>) -> Self {
        Self::SetNative { panel, placement }
    }

    /// Builds [`Action::MovePanel`].
    pub fn move_panel(panel: P, to: Target<P>) -> Self {
        Self::MovePanel { panel, to }
    }
}

impl<P> DockPlacement<P> {
    /// Builds [`DockPlacement::Tab`].
    pub fn tab(before: Option<P>) -> Self {
        Self::Tab { before }
    }

    /// Builds [`DockPlacement::Group`].
    pub fn group(below: bool, height: Option<f32>) -> Self {
        Self::Group { below, height }
    }

    /// Builds [`DockPlacement::Column`].
    pub fn column(after: bool) -> Self {
        Self::Column { after }
    }
}

impl<P> DockPlacement<P> {
    /// Whether the anchor's permissions take this placement: tabs need `accepts_tabs`, side
    /// splits `accepts_splits`, and so does a group placed at an anchor in a floating window
    /// (`floating`), which splits the anchor's frame.
    pub fn allowed_by(&self, anchor: Permissions, floating: bool) -> bool {
        match self {
            Self::Tab { .. } | Self::Split(DockSide::Center) => anchor.accepts_tabs,
            Self::Split(_) | Self::SplitSized { .. } => anchor.accepts_splits,
            Self::Group { .. } if floating => anchor.accepts_splits,
            _ => true,
        }
    }
}
