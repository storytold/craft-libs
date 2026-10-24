use std::{collections::HashSet, fmt, hash::Hash};

use super::dock_model::{
    Column, ColumnLocation, Group, GroupLocation, Moving, PanelState, Place, Side, Target, Unit, node_at, split_frame, tree_panels, valid_column,
};
use crate::layout::{SplitAxis, SplitSize};

/// Limits are checked before cloning or recursively modifying an application layout.
pub const MAX_DEPTH: usize = 64;
pub const MAX_PANELS: usize = 4096;
pub(super) const MAX_NODES: usize = MAX_PANELS * 2;
const MAX_EXTENT: f32 = 1_000_000.0;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum Zone {
    Center,
    Left,
    Right,
    Top,
    Bottom,
}

#[derive(Clone, Debug, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[non_exhaustive]
pub enum Node<P> {
    Split {
        axis: SplitAxis,
        size: SplitSize,
        first: Box<Node<P>>,
        second: Box<Node<P>>,
    },
    Tabs {
        panels: Vec<P>,
        active: usize,
    },
    /// An accordion keeps its open state and preferred heights when other panels move.
    Stack {
        entries: Vec<StackEntry<P>>,
    },
    /// One window edge's dock, columns from the window edge inwards. A dock is the root or,
    /// following only same-side children of horizontal splits, at its window edge. At most one
    /// per side. An empty dock (no columns) is valid and accepts [`Target::NewDock`].
    Dock {
        side: Side,
        columns: Vec<Column<P>>,
    },
}

#[derive(Clone, Debug, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct StackEntry<P> {
    pub panel: P,
    pub open: bool,
    pub height: Option<f32>,
}

/// A floating window: a tree of groups like the docked layout's, without docks.
///
/// Serde reads the v1 shape `{panels, active, rect}` as one group, and a stack of groups
/// `{groups, rect}` as a vertical split of them, sized by their heights. A window with one
/// group (not minimized, no native rect) writes the v1 shape, so older builds can read files
/// that use no newer feature. A native window also writes its `id`; any other window gets a
/// fresh one on load.
///
/// Equality ignores `id`: two windows with the same contents and place are the same layout.
#[derive(Clone, Debug)]
pub struct Floating<P> {
    /// Names the window across panel moves, and its OS window with feature `native-windows`.
    /// Unique within a layout: [`Self::new`] and loading draw fresh ids, [`Self::with_id`]
    /// sets one, and merging windows keeps the receiving window's.
    pub id: u64,
    /// `Split`, `Tabs` and `Stack` nodes; a `Dock` is invalid here.
    pub root: Node<P>,
    /// `[x, y, width, height]` in logical points, in the application's egui viewport.
    pub rect: [f32; 4],
    /// Desktop rect while the window is in its own OS window.
    pub native: Option<[f32; 4]>,
    /// The monitor holding the center of `native`, from [`super::DockArea::monitors`].
    pub monitor: Option<Monitor>,
    /// Without a monitor list: the size of the monitor egui reported for the native window.
    pub monitor_size: Option<[f32; 2]>,
    /// Only the title bar shows.
    pub minimized: bool,
}

impl<P: PartialEq> PartialEq for Floating<P> {
    fn eq(&self, other: &Self) -> bool {
        self.root == other.root
            && self.rect == other.rect
            && self.native == other.native
            && self.monitor == other.monitor
            && self.monitor_size == other.monitor_size
            && self.minimized == other.minimized
    }
}

/// A display, in desktop logical points. Applications on an egui with monitor reports fill
/// [`super::DockArea::monitors`] with these; native floating windows store the one they are on.
#[derive(Clone, Debug, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Monitor {
    pub name: Option<String>,
    /// `[x, y, width, height]` on the desktop.
    pub rect: [f32; 4],
    /// Physical pixels per logical point.
    pub scale: f32,
}

/// A window id no other window of this process has drawn, from a random start so ids saved
/// by earlier runs are unlikely to repeat.
pub(super) fn fresh_window_id() -> u64 {
    use std::hash::{BuildHasher, Hasher};
    use std::sync::{
        OnceLock,
        atomic::{AtomicU64, Ordering},
    };
    static NEXT: OnceLock<AtomicU64> = OnceLock::new();
    let next = NEXT.get_or_init(|| AtomicU64::new(std::collections::hash_map::RandomState::new().build_hasher().finish() >> 1));
    next.fetch_add(1, Ordering::Relaxed)
}

impl<P> Floating<P> {
    /// A window holding `root`. Each call makes a new window, with a fresh [`Self::id`]: build
    /// a layout once and keep it, or give its windows ids with [`Self::with_id`]. A layout
    /// built anew each frame without them opens new OS windows each frame.
    pub fn new(root: Node<P>, rect: [f32; 4]) -> Self {
        Self { id: fresh_window_id(), root, rect, native: None, monitor: None, monitor_size: None, minimized: false }
    }

    /// A window with one group. Each call makes a new window, as [`Self::new`] does.
    pub fn single(panels: Vec<P>, active: usize, rect: [f32; 4]) -> Self {
        Self::new(Node::Tabs { panels, active }, rect)
    }

    /// The window with `id`, for a layout the app builds in code, such as a default layout:
    /// built with the same id, it is the same window, and the same OS window, on every frame.
    /// Ids must be unique within a layout.
    pub fn with_id(mut self, id: u64) -> Self {
        self.id = id;
        self
    }

    pub fn panels(&self) -> impl Iterator<Item = &P> {
        tree_panels(&self.root).into_iter()
    }

    /// Groups stacked top to bottom, as a v1 layout stored them: each group over a vertical
    /// split of the ones below it, sized by their heights. `None` without groups. Each call
    /// makes a new window, as [`Self::new`] does.
    pub fn stack(groups: Vec<Group<P>>, rect: [f32; 4]) -> Option<Self> {
        super::dock_model::stack_tree(groups).map(|root| Self::new(root, rect))
    }

    /// The window's groups, first to last.
    #[cfg(test)]
    pub(crate) fn groups(&self) -> Vec<Group<P>>
    where
        P: Clone,
    {
        super::dock_model::tree_groups(&self.root)
    }

    /// True when a group of this window holds `panel`. Takes any borrowed form of `P`, such
    /// as `&str` for `String` panels.
    pub fn contains<Q: ?Sized + PartialEq>(&self, panel: &Q) -> bool
    where
        P: std::borrow::Borrow<Q>,
    {
        self.panels().any(|p| p.borrow() == panel)
    }
}

#[cfg(feature = "serde")]
mod floating_serde {
    use serde::{Deserialize, Deserializer, Serialize, Serializer};

    use super::{Floating, Group, Monitor, Node, fresh_window_id};
    use crate::docking::dock_model::stack_tree;

    #[derive(Serialize)]
    struct V1<'a, P> {
        panels: &'a [P],
        active: usize,
        rect: [f32; 4],
    }

    #[derive(Serialize)]
    struct Tree<'a, P> {
        #[serde(skip_serializing_if = "Option::is_none")]
        id: Option<u64>,
        root: &'a Node<P>,
        rect: [f32; 4],
        #[serde(skip_serializing_if = "Option::is_none")]
        native: Option<[f32; 4]>,
        #[serde(skip_serializing_if = "Option::is_none")]
        monitor: Option<&'a Monitor>,
        #[serde(skip_serializing_if = "Option::is_none")]
        monitor_size: Option<[f32; 2]>,
        #[serde(skip_serializing_if = "std::ops::Not::not")]
        minimized: bool,
    }

    impl<P: Serialize> Serialize for Floating<P> {
        fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
            match &self.root {
                Node::Tabs { panels, active } if !self.minimized && self.native.is_none() => {
                    V1 { panels, active: *active, rect: self.rect }.serialize(serializer)
                }
                root => Tree {
                    id: self.native.map(|_| self.id),
                    root,
                    rect: self.rect,
                    native: self.native,
                    monitor: self.monitor.as_ref(),
                    monitor_size: self.monitor_size,
                    minimized: self.minimized,
                }
                .serialize(serializer),
            }
        }
    }

    #[derive(Deserialize)]
    #[serde(bound(deserialize = "P: Deserialize<'de>"))]
    struct Repr<P> {
        #[serde(default)]
        id: Option<u64>,
        root: Option<Node<P>>,
        groups: Option<Vec<Group<P>>>,
        panels: Option<Vec<P>>,
        #[serde(default)]
        active: usize,
        rect: [f32; 4],
        #[serde(default)]
        native: Option<[f32; 4]>,
        #[serde(default)]
        monitor: Option<Monitor>,
        #[serde(default)]
        monitor_size: Option<[f32; 2]>,
        #[serde(default)]
        minimized: bool,
    }

    /// Floating windows with a fresh id for each one whose id an earlier window took.
    pub(super) fn unique_ids<'de, D: Deserializer<'de>, P: Deserialize<'de>>(deserializer: D) -> Result<Vec<Floating<P>>, D::Error> {
        let mut windows = Vec::<Floating<P>>::deserialize(deserializer)?;
        let mut seen = std::collections::HashSet::new();
        for window in &mut windows {
            while !seen.insert(window.id) {
                window.id = fresh_window_id();
            }
        }
        Ok(windows)
    }

    impl<'de, P: Deserialize<'de>> Deserialize<'de> for Floating<P> {
        fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
            let repr = Repr::<P>::deserialize(deserializer)?;
            let mut minimized = repr.minimized;
            let root = match (repr.root, repr.groups, repr.panels) {
                (Some(root), None, None) => root,
                (None, Some(groups), None) => {
                    minimized |= !groups.is_empty() && groups.iter().all(|g| g.minimized);
                    stack_tree(groups).ok_or_else(|| serde::de::Error::invalid_length(0, &"at least one group"))?
                }
                (None, None, Some(panels)) => Node::Tabs { panels, active: repr.active },
                (None, None, None) => return Err(serde::de::Error::missing_field("root")),
                _ => return Err(serde::de::Error::duplicate_field("root")),
            };
            Ok(Floating {
                id: repr.id.unwrap_or_else(fresh_window_id),
                root,
                rect: repr.rect,
                native: repr.native,
                monitor: repr.monitor,
                monitor_size: repr.monitor_size,
                minimized,
            })
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Layout<P> {
    pub root: Option<Node<P>>,
    #[cfg_attr(feature = "serde", serde(deserialize_with = "floating_serde::unique_ids"))]
    pub floating: Vec<Floating<P>>,
}

impl<P> Default for Layout<P> {
    fn default() -> Self {
        Self { root: None, floating: Vec::new() }
    }
}

#[derive(Clone, Debug, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[non_exhaustive]
pub enum Placement<P> {
    /// Insert before this existing destination member, or append. Also reorders a group.
    Tab { before: Option<P> },
    /// Center inserts after the anchor; sides split the anchor's frame into two halves, and
    /// every other frame keeps its size. Sides apply to `Tabs` and `Stack` nodes only.
    Split(Zone),
    /// A new group above or below the anchor's group in a dock column. On an anchor in a
    /// floating window, a split of the anchor's group above or below.
    /// `height` is the inserted group's preferred height ([`super::Group::height`]); `None`
    /// keeps a moved group's own height, or shares the free height for a new group.
    Group {
        below: bool,
        #[cfg_attr(feature = "serde", serde(default, skip_serializing_if = "Option::is_none"))]
        height: Option<f32>,
    },
    /// A new column before (towards the window edge) or after the anchor's column.
    Column { after: bool },
    /// A new frame along one edge of the docked layout, a quarter of its width or height: the
    /// root is split, or in a layout with docks, the part between them. The anchor names any
    /// docked panel. `Center` is invalid.
    Edge(Zone),
}

/// A closed panel's former placement. Applications may retain this beside visibility flags.
/// Anchors are stable panel identities rather than mutable tree indices.
#[derive(Clone, Debug, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(bound(deserialize = "P: serde::Deserialize<'de>")))]
pub struct Location<P> {
    pub floating: Option<[f32; 4]>,
    pub anchor: Option<P>,
    pub placement: Placement<P>,
    pub size: Option<SplitSize>,
    /// Accordion state to restore if the destination is still an accordion.
    pub stack: Option<(bool, Option<f32>)>,
    /// A split's complete sibling subtree. This retains placement around a nested workspace,
    /// rather than accidentally inserting beside only its first leaf when restoring.
    #[cfg_attr(feature = "serde", serde(default))]
    pub sibling: Vec<P>,
    /// Group and column state when the panel was in a dock. Applies when
    /// restoring creates a new group, column or dock for the panel.
    #[cfg_attr(feature = "serde", serde(default))]
    pub group: Option<GroupLocation>,
}

/// Groups, columns and floating windows are named by a panel they contain.
#[derive(Clone, Debug, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[non_exhaustive]
pub enum Action<P> {
    Move {
        panel: P,
        anchor: P,
        placement: Placement<P>,
    },
    Float {
        panel: P,
        rect: [f32; 4],
    },
    Close {
        panel: P,
    },
    Activate {
        panel: P,
    },
    /// Reveal a registered, currently absent panel. Applications validate their panel registry.
    Open {
        panel: P,
        anchor: Option<P>,
    },
    /// Reveal an absent panel beside a protected canvas or at another explicit destination.
    OpenAt {
        panel: P,
        anchor: P,
        placement: Placement<P>,
    },
    /// Reposition/resize a floating window, identified by one of its panels.
    MoveFloating {
        panel: P,
        rect: [f32; 4],
    },
    /// Path from the root: false is the first child, true the second.
    ResizeSplit {
        path: Vec<bool>,
        size: SplitSize,
    },
    /// [`Action::ResizeSplit`] in the floating window holding `panel`, with a path from its
    /// root.
    ResizeWindowSplit {
        panel: P,
        path: Vec<bool>,
        size: SplitSize,
    },
    SetStackOpen {
        panel: P,
        open: bool,
    },
    ResizeStack {
        panel: P,
        height: Option<f32>,
    },
    /// Move the group containing `panel` with its order, active tab and minimized state.
    /// `Target::At` with `Tab` or `Split(Center)` merges every member. An accordion entry is a
    /// group of one. A target inside the group is a no-op.
    MoveGroup {
        panel: P,
        to: Target<P>,
    },
    /// Move the dock column containing `panel`. `Float` makes a window of its groups.
    MoveColumn {
        panel: P,
        to: Target<P>,
    },
    /// Dock or merge the floating window containing `panel`, every group included.
    MoveWindow {
        panel: P,
        to: Target<P>,
    },
    /// Close Tab Group.
    CloseGroup {
        panel: P,
    },
    /// × on a floating window: closes all its panels.
    CloseWindow {
        panel: P,
    },
    /// Close Other Panels in Group.
    CloseOthers {
        panel: P,
    },
    /// Minimize a dock group to its tab strip, or the floating window holding the panel to its
    /// title bar (and the strip of its only group). On an accordion entry, closes it.
    SetMinimized {
        panel: P,
        minimized: bool,
    },
    SetIconic {
        panel: P,
        iconic: bool,
    },
    ResizeColumn {
        panel: P,
        width: f32,
    },
    /// Preferred height of a dock group or of an accordion entry.
    ResizeGroup {
        panel: P,
        height: Option<f32>,
    },
    ResizeIconic {
        panel: P,
        width: Option<f32>,
    },
    /// Put the floating window in its own OS window at `rect`, or back in the app window.
    SetNative {
        panel: P,
        rect: Option<[f32; 4]>,
    },
    /// Record the monitor of the native window holding `panel`: from the application's
    /// monitor list, or else only the size egui reported.
    SetMonitor {
        panel: P,
        monitor: Option<Monitor>,
        monitor_size: Option<[f32; 2]>,
    },
    /// Move one panel, without the rest of its group. `Target::At` is [`Action::Move`];
    /// `NewDock` and `Float` give the panel a group of its own.
    MovePanel {
        panel: P,
        to: Target<P>,
    },
}

/// What may happen to a panel. [`Layout::apply_with_permissions`] enforces these for every
/// action, whoever issues it. They also decide which gestures and menu items the renderer
/// offers; [`super::DockStyle::close_menu_items`] and [`super::DockStyle::close_button`] hide
/// the built-in close controls of a panel that stays closable by command.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Permissions {
    pub movable: bool,
    pub floatable: bool,
    pub closable: bool,
    /// Other panels may join this panel's group as tabs.
    pub accepts_tabs: bool,
    /// Other panels may split this panel's frame (`Placement::Split` with a side zone).
    /// Outer-edge frames ([`Placement::Edge`]) are not splits of the anchor.
    pub accepts_splits: bool,
}

impl Default for Permissions {
    fn default() -> Self {
        Self { movable: true, floatable: true, closable: true, accepts_tabs: true, accepts_splits: true }
    }
}

impl Permissions {
    /// A document canvas: it stays docked and open, and takes neither tabs nor side splits.
    /// A group dropped on it floats there. An application that opens panels beside its canvas
    /// uses `Permissions { accepts_splits: true, ..Permissions::PROTECTED }`.
    pub const PROTECTED: Self = Self { movable: false, floatable: false, closable: false, accepts_tabs: false, accepts_splits: false };

    /// A canvas: it can neither move, close nor take tabs. `accepts_splits` and `floatable`
    /// do not matter, so a canvas that opens panels beside it still counts.
    pub fn is_protected(self) -> bool {
        !self.movable && !self.closable && !self.accepts_tabs
    }

    /// Lock Workspace: nothing moves, floats or closes; tab and split drops keep their policy.
    /// [`super::Workspace::apply`] applies this when the workspace is locked; an application
    /// with a global lock preference applies it in its own permissions callback.
    pub fn locked(self) -> Self {
        Self { movable: false, floatable: false, closable: false, ..self }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum DockError {
    LimitExceeded,
    DuplicatePanel,
    MissingPanel,
    MissingTarget,
    InvalidSelection,
    InvalidGeometry,
    InvalidPlacement,
    NotAllowed,
    /// Saved by a newer version of the docking format.
    UnsupportedVersion,
}

impl fmt::Display for DockError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::LimitExceeded => "docking layout exceeds its size or depth limit",
            Self::DuplicatePanel => "panel occurs in more than one docking location",
            Self::MissingPanel => "source panel is not present in the docking layout",
            Self::MissingTarget => "docking destination is no longer present",
            Self::InvalidSelection => "active docking tab is out of bounds",
            Self::InvalidGeometry => "docking geometry is not finite and bounded",
            Self::InvalidPlacement => "docking destination does not support this placement",
            Self::NotAllowed => "this panel does not allow the requested docking operation",
            Self::UnsupportedVersion => "docking data was saved by a newer version",
        })
    }
}

impl std::error::Error for DockError {}

pub(super) fn valid_rect(rect: [f32; 4]) -> bool {
    rect.iter().all(|v| v.is_finite() && v.abs() <= MAX_EXTENT) && rect.get(2).is_some_and(|v| *v > 0.0) && rect.get(3).is_some_and(|v| *v > 0.0)
}

/// A floating window's saved monitor and monitor size, each where given.
fn valid_monitor(monitor: Option<&Monitor>, monitor_size: Option<[f32; 2]>) -> bool {
    monitor.is_none_or(|m| valid_rect(m.rect) && m.scale.is_finite() && m.scale > 0.0)
        && monitor_size.is_none_or(|[w, h]| valid_rect([0.0, 0.0, w, h]))
}

pub(super) fn valid_width(width: f32) -> bool {
    width.is_finite() && width > 0.0 && width <= MAX_EXTENT
}

fn valid_size(size: SplitSize) -> bool {
    match size {
        SplitSize::Ratio(r) => r.is_finite() && (0.0..=1.0).contains(&r),
        SplitSize::FixedFirst(v) | SplitSize::FixedSecond(v) => v.is_finite() && (0.0..=MAX_EXTENT).contains(&v),
    }
}

pub(super) fn valid_height(height: Option<f32>) -> bool {
    height.is_none_or(|v| v.is_finite() && (0.0..=MAX_EXTENT).contains(&v))
}

fn valid_active(len: usize, active: usize) -> bool {
    active < len || (len == 0 && active == 0)
}

impl<P: Clone + Eq + Hash> Layout<P> {
    /// Check depth, allocation bounds, geometry, selection, globally unique panel ownership and
    /// that no frame, group, column or window is empty.
    /// Traversal is iterative, including for an invalid, excessively deep supplied tree.
    pub fn validate(&self) -> Result<(), DockError> {
        if self.floating.len() > MAX_PANELS {
            return Err(DockError::LimitExceeded);
        }
        let mut seen = HashSet::new();
        let mut count = 0usize;
        let mut docks = (false, false);
        // Each node carries whether it touches the window's start and end edges.
        let mut visit = self.root.as_ref().map(|n| vec![(n, 1usize, true, true)]).unwrap_or_default();
        while let Some((node, depth, start, end)) = visit.pop() {
            count = count.saturating_add(1);
            if depth > MAX_DEPTH || count > MAX_NODES {
                return Err(DockError::LimitExceeded);
            }
            match node {
                Node::Split { axis, size, first, second } => {
                    if !valid_size(*size) {
                        return Err(DockError::InvalidGeometry);
                    }
                    let horizontal = *axis == SplitAxis::Horizontal;
                    visit.push((second, depth + 1, false, end && horizontal));
                    visit.push((first, depth + 1, start && horizontal, false));
                }
                Node::Dock { side, columns } => {
                    let (at_edge, taken) = match side {
                        Side::Start => (start, &mut docks.0),
                        Side::End => (end, &mut docks.1),
                    };
                    if !at_edge || *taken {
                        return Err(DockError::InvalidPlacement);
                    }
                    *taken = true;
                    if columns.len() > MAX_PANELS {
                        return Err(DockError::LimitExceeded);
                    }
                    for column in columns {
                        if !valid_column(column) || column.groups.is_empty() {
                            return Err(DockError::InvalidGeometry);
                        }
                        count = count.saturating_add(column.groups.len().saturating_add(1));
                        if count > MAX_NODES {
                            return Err(DockError::LimitExceeded);
                        }
                        for group in &column.groups {
                            check_group(group, &mut seen)?;
                        }
                    }
                }
                Node::Tabs { .. } | Node::Stack { .. } => check_leaf(node, &mut seen)?,
            }
        }
        for floating in &self.floating {
            if !valid_rect(floating.rect)
                || floating.native.is_some_and(|r| !valid_rect(r))
                || !valid_monitor(floating.monitor.as_ref(), floating.monitor_size)
            {
                return Err(DockError::InvalidGeometry);
            }
            let mut visit = vec![(&floating.root, 1usize)];
            while let Some((node, depth)) = visit.pop() {
                count = count.saturating_add(1);
                if depth > MAX_DEPTH || count > MAX_NODES {
                    return Err(DockError::LimitExceeded);
                }
                match node {
                    Node::Split { size, first, second, .. } => {
                        if !valid_size(*size) {
                            return Err(DockError::InvalidGeometry);
                        }
                        visit.push((second, depth + 1));
                        visit.push((first, depth + 1));
                    }
                    Node::Dock { .. } => return Err(DockError::InvalidPlacement),
                    Node::Tabs { .. } | Node::Stack { .. } => check_leaf(node, &mut seen)?,
                }
            }
        }
        Ok(())
    }

    /// Iterate identities without recursion. On excessive input returns no more than the limit.
    pub fn panels(&self) -> Vec<&P> {
        let mut panels = Vec::new();
        let mut visit: Vec<&Node<P>> = self.root.iter().collect();
        let mut visited = 0usize;
        while let Some(node) = visit.pop() {
            visited = visited.saturating_add(1);
            if visited > MAX_NODES || panels.len() >= MAX_PANELS {
                break;
            }
            match node {
                Node::Split { first, second, .. } => {
                    visit.push(second);
                    visit.push(first);
                }
                Node::Tabs { panels: group, .. } => panels.extend(group.iter().take(MAX_PANELS - panels.len())),
                Node::Stack { entries } => panels.extend(entries.iter().map(|e| &e.panel).take(MAX_PANELS - panels.len())),
                Node::Dock { columns, .. } => {
                    let members = columns.iter().flat_map(|c| c.groups.iter()).flat_map(|g| g.panels.iter());
                    panels.extend(members.take(MAX_PANELS - panels.len()));
                }
            }
        }
        for window in self.floating.iter().take(MAX_PANELS) {
            panels.extend(window.panels().take(MAX_PANELS - panels.len()));
        }
        panels
    }

    pub fn contains(&self, panel: &P) -> bool {
        self.panels().contains(&panel)
    }

    /// The panels `panel` can join as a tab: every panel in another group whose permissions
    /// accept tabs. A "Move Panel to Group" menu with an empty answer has nothing to offer and
    /// stays hidden.
    pub fn move_targets(&self, panel: &P, permissions: impl Fn(&P) -> Permissions) -> Vec<&P> {
        let own = self.place(panel);
        self.panels()
            .into_iter()
            .filter(|target| *target != panel && permissions(target).accepts_tabs && (own.is_none() || self.place(target) != own))
            .collect()
    }

    /// Capture a panel's nearest surviving group/split neighbor before hiding it.
    pub fn location(&self, panel: &P) -> Result<Location<P>, DockError> {
        self.validate()?;
        let place = self.place(panel).ok_or(DockError::MissingPanel)?;
        let (groups, group, column) = match &place {
            Place::Node(_) => return self.root.as_ref().and_then(|node| node_location(node, panel)).ok_or(DockError::MissingPanel),
            Place::Floating { window, .. } => {
                let window = self.floating.get(*window).ok_or(DockError::MissingPanel)?;
                let mut location = node_location(&window.root, panel).ok_or(DockError::MissingPanel)?;
                location.floating = Some(window.rect);
                return Ok(location);
            }
            Place::Dock { path, group, .. } => {
                let side = match self.root.as_ref().and_then(|root| node_at(root, path)) {
                    Some(Node::Dock { side, .. }) => *side,
                    _ => return Err(DockError::MissingPanel),
                };
                let column = self.column(&place).ok_or(DockError::MissingPanel)?;
                let state = ColumnLocation { side, width: column.width, iconic: column.iconic, iconic_width: column.iconic_width };
                (&column.groups, *group, Some(state))
            }
        };
        let own = groups.get(group).ok_or(DockError::MissingPanel)?;
        let mut location = group_location(&own.panels, panel).ok_or(DockError::MissingPanel)?;
        location.group = Some(GroupLocation { minimized: own.minimized, height: own.height, column });
        if location.anchor.is_some() {
            return Ok(location);
        }
        // Alone in its group: anchor to the neighboring group, then the neighboring column.
        let first = |g: Option<&Group<P>>| g.and_then(|g| g.panels.first()).cloned();
        if let Some(anchor) = first(groups.get(group + 1)) {
            location.anchor = Some(anchor);
            location.placement = Placement::Group { below: false, height: None };
        } else if let Some(anchor) = first(group.checked_sub(1).and_then(|i| groups.get(i))) {
            location.anchor = Some(anchor);
            location.placement = Placement::Group { below: true, height: None };
        } else if groups.len() == 1
            && let Place::Dock { path, column, .. } = &place
            && let Some(Node::Dock { columns, .. }) = self.root.as_ref().and_then(|root| node_at(root, path))
        {
            let first_of = |c: Option<&Column<P>>| first(c.and_then(|c| c.groups.first()));
            if let Some(anchor) = first_of(columns.get(column + 1)) {
                location.anchor = Some(anchor);
                location.placement = Placement::Column { after: false };
            } else if let Some(anchor) = first_of(column.checked_sub(1).and_then(|i| columns.get(i))) {
                location.anchor = Some(anchor);
                location.placement = Placement::Column { after: true };
            }
        }
        Ok(location)
    }

    /// Hidden, open behind another tab or in a collapsed container, or showing.
    pub fn panel_state(&self, panel: &P) -> PanelState {
        let showing = match self.place(panel) {
            None => return PanelState::Hidden,
            Some(place @ (Place::Node(_) | Place::Floating { .. })) => {
                let minimized = matches!(place, Place::Floating { window, .. } if self.floating.get(window).is_some_and(|w| w.minimized));
                !minimized
                    && match self.node_of(&place) {
                        Some(Node::Tabs { panels, active }) => panels.get(*active) == Some(panel),
                        Some(Node::Stack { entries }) => entries.iter().any(|e| &e.panel == panel && e.open),
                        _ => false,
                    }
            }
            Some(place) => {
                let iconic = self.column(&place).is_some_and(|c| c.iconic);
                self.group(&place).is_some_and(|g| !g.minimized && g.active_panel() == Some(panel)) && !iconic
            }
        };
        if showing { PanelState::Front } else { PanelState::Open }
    }

    /// Restore an absent panel at a captured location. Missing anchors are reported without
    /// changes, allowing the app to choose an explicit default; a floating panel whose
    /// window is gone floats at its last rect instead. A tab whose next tab is not in the
    /// anchor's group goes last. Unrelated tabs retain selection.
    pub fn restore(&mut self, panel: P, location: &Location<P>) -> Result<(), DockError> {
        self.validate()?;
        if self.contains(&panel) {
            return Err(DockError::DuplicatePanel);
        }
        if location.size.is_some_and(|s| !valid_size(s)) || location.stack.is_some_and(|(_, h)| !valid_height(h)) {
            return Err(DockError::InvalidGeometry);
        }
        let state = location.group.as_ref();
        if state.is_some_and(|s| {
            !valid_height(s.height) || s.column.is_some_and(|c| !valid_width(c.width) || c.iconic_width.is_some_and(|w| !valid_width(w)))
        }) {
            return Err(DockError::InvalidGeometry);
        }
        // A restore that creates a group, column or dock gives it the saved state.
        let moving = || {
            let group =
                Group { panels: vec![panel.clone()], active: 0, minimized: state.is_some_and(|s| s.minimized), height: state.and_then(|s| s.height) };
            let column = state.and_then(|s| s.column);
            Moving {
                width: column.map(|c| c.width),
                iconic: column.is_some_and(|c| c.iconic),
                iconic_width: column.and_then(|c| c.iconic_width),
                ..Moving::group(group)
            }
        };
        let mut proposed = self.clone();
        if location.floating.is_some_and(|rect| !valid_rect(rect)) {
            return Err(DockError::InvalidGeometry);
        }
        let window_open = location.anchor.as_ref().is_some_and(|anchor| proposed.floating.iter().any(|w| w.contains(anchor)));
        if let Some(rect) = location.floating.filter(|_| !window_open) {
            // A window that closed as a whole reopens at its last rect.
            proposed.insert_moving(moving(), Target::Float { rect: Some(rect) })?;
        } else if let Some(anchor) = &location.anchor
            && matches!(location.placement, Placement::Group { .. } | Placement::Column { .. })
        {
            if !proposed.contains(anchor) {
                return Err(DockError::MissingTarget);
            }
            proposed.insert_moving(moving(), Target::At { anchor: anchor.clone(), placement: location.placement.clone() })?;
        } else if let Some(column) = state.and_then(|s| s.column).filter(|_| location.anchor.is_none()) {
            // Alone in its dock: back at that edge, as the outermost column if a dock exists.
            match proposed.dock_path(column.side) {
                Some(path) => {
                    let columns = proposed.columns_mut(&path).ok_or(DockError::MissingTarget)?;
                    let Moving { groups, .. } = moving();
                    columns.insert(0, Column { groups, width: column.width, iconic: column.iconic, iconic_width: column.iconic_width });
                }
                None => proposed.insert_moving(moving(), Target::NewDock { side: column.side })?,
            }
        } else if let Some(anchor) = &location.anchor {
            if !proposed.contains(anchor) {
                return Err(DockError::MissingTarget);
            }
            let mut around = false;
            if let Placement::Split(zone @ (Zone::Left | Zone::Right | Zone::Top | Zone::Bottom)) = location.placement
                && !location.sibling.is_empty()
            {
                if location.sibling.len() > MAX_PANELS {
                    return Err(DockError::LimitExceeded);
                }
                let members: HashSet<&P> = location.sibling.iter().collect();
                if members.len() != location.sibling.len() || !members.contains(anchor) {
                    return Err(DockError::InvalidPlacement);
                }
                let size = location.size.unwrap_or(SplitSize::Ratio(0.5));
                around = proposed.trees_mut().any(|root| restore_around_subtree(root, &panel, &members, zone, size));
            }
            // A subtree that changed since (a migrated or reconciled layout) splits the anchor.
            if !around {
                // The tab that followed may have closed or moved since; the panel then goes last.
                let placement = match &location.placement {
                    Placement::Tab { before: Some(before) } if !proposed.same_group(before, anchor) => Placement::Tab { before: None },
                    placement => placement.clone(),
                };
                proposed.apply_inner(Action::OpenAt { panel: panel.clone(), anchor: anchor.clone(), placement })?;
                if let Some(size) = location.size {
                    proposed.trees_mut().any(|root| restore_parent_size(root, &panel, size));
                }
            }
        } else if proposed.root.is_none() {
            proposed.root = Some(Node::Tabs { panels: vec![panel.clone()], active: 0 });
        } else {
            return Err(DockError::MissingTarget);
        }
        if let Some((open, height)) = location.stack {
            for root in proposed.trees_mut() {
                restore_stack_kind(root, &panel);
                if let Some(entry) = stack_entry_mut(root, &panel) {
                    entry.open = open;
                    entry.height = height;
                }
            }
        }
        proposed.validate()?;
        *self = proposed;
        Ok(())
    }

    /// Open an absent panel as a frame at the outer edge of the docked area, taking most of it:
    /// the last resort for a panel that must stay shown, such as a protected canvas.
    pub(super) fn open_at_edge(&mut self, panel: P) -> Result<(), DockError> {
        let mut proposed = self.clone();
        if self.root.is_none() {
            proposed.root = Some(Node::Tabs { panels: vec![panel], active: 0 });
        } else {
            let anchor = self
                .panels()
                .into_iter()
                .find(|p| matches!(self.place(p), Some(Place::Dock { .. } | Place::Node(_))))
                .cloned()
                .ok_or(DockError::MissingTarget)?;
            proposed.apply_inner(Action::OpenAt { panel: panel.clone(), anchor, placement: Placement::Edge(Zone::Right) })?;
            if let Some(root) = proposed.root.as_mut() {
                restore_parent_size(root, &panel, SplitSize::Ratio(0.25));
            }
        }
        proposed.validate()?;
        *self = proposed;
        Ok(())
    }

    /// [`Self::apply_with_permissions`] with default permissions.
    pub fn apply(&mut self, action: Action<P>) -> Result<Vec<P>, DockError> {
        self.apply_with_permissions(action, |_| Permissions::default())
    }

    /// Check application policy before making an atomic change, and return the panels the
    /// action closed (see [`Self::closed_by`]) so the application can release their state.
    /// The callback must be stable throughout one call. Registry validation remains with the
    /// application, particularly Open.
    pub fn apply_with_permissions(&mut self, action: Action<P>, permissions: impl Fn(&P) -> Permissions) -> Result<Vec<P>, DockError> {
        self.validate()?;
        match &action {
            Action::Move { panel, anchor, placement } | Action::OpenAt { panel, anchor, placement } => {
                // Opening an absent panel moves nothing, so it works in a locked workspace too.
                let opens = matches!(action, Action::OpenAt { .. }) && !self.contains(panel);
                if !opens && !permissions(panel).movable {
                    return Err(DockError::NotAllowed);
                }
                if !placement_allowed(placement, permissions(anchor)) {
                    return Err(DockError::NotAllowed);
                }
            }
            Action::Float { panel, .. } if !permissions(panel).floatable || !permissions(panel).movable => return Err(DockError::NotAllowed),
            Action::Close { panel } if !permissions(panel).closable => return Err(DockError::NotAllowed),
            // The rectangle belongs to the whole floating window, so every panel in it must allow it.
            Action::MoveFloating { panel, .. }
                if !permissions(panel).movable
                    || self.floating.iter().find(|w| w.contains(panel)).is_some_and(|w| w.panels().any(|p| !permissions(p).movable)) =>
            {
                return Err(DockError::NotAllowed);
            }
            Action::Open { anchor: Some(anchor), .. } if !permissions(anchor).accepts_tabs => return Err(DockError::NotAllowed),
            Action::MovePanel { panel, to } => {
                let policy = permissions(panel);
                let allowed = policy.movable
                    && match to {
                        Target::Float { .. } => policy.floatable,
                        Target::At { anchor, placement } => placement_allowed(placement, permissions(anchor)),
                        Target::NewDock { .. } => true,
                    };
                if !allowed {
                    return Err(DockError::NotAllowed);
                }
            }
            Action::MoveGroup { panel, to } | Action::MoveColumn { panel, to } | Action::MoveWindow { panel, to } => {
                let members = self.members(panel, unit_of(&action))?;
                if members.iter().any(|p| !permissions(p).movable) {
                    return Err(DockError::NotAllowed);
                }
                match to {
                    Target::Float { .. } if members.iter().any(|p| !permissions(p).floatable) => return Err(DockError::NotAllowed),
                    Target::At { anchor, placement } if !members.contains(anchor) && !placement_allowed(placement, permissions(anchor)) => {
                        return Err(DockError::NotAllowed);
                    }
                    _ => {}
                }
            }
            Action::CloseGroup { panel } | Action::CloseWindow { panel } | Action::CloseOthers { panel } => {
                let others = matches!(action, Action::CloseOthers { .. });
                let members = self.members(panel, unit_of(&action))?;
                if members.iter().filter(|p| !others || *p != panel).any(|p| !permissions(p).closable) {
                    return Err(DockError::NotAllowed);
                }
            }
            Action::SetNative { panel, rect: Some(_) } if self.members(panel, Unit::Window)?.iter().any(|p| !permissions(p).floatable) => {
                return Err(DockError::NotAllowed);
            }
            _ => {}
        }
        let closed = self.closed_by(&action)?;
        let mut proposed = self.clone();
        proposed.apply_inner(action)?;
        proposed.validate()?;
        *self = proposed;
        Ok(closed)
    }

    /// Apply every action in order, or none: the first rejection leaves the layout unchanged.
    /// Returns every panel the actions closed, in order.
    pub fn apply_all(&mut self, actions: impl IntoIterator<Item = Action<P>>, permissions: impl Fn(&P) -> Permissions) -> Result<Vec<P>, DockError> {
        let mut proposed = self.clone();
        let mut closed = Vec::new();
        for action in actions {
            closed.extend(proposed.apply_with_permissions(action, &permissions)?);
        }
        *self = proposed;
        Ok(closed)
    }

    /// The panels a close action (`Close`, `CloseGroup`, `CloseWindow`, `CloseOthers`) would
    /// remove, in group order. Empty for other actions. The apply methods return the same
    /// list; this one answers before applying.
    pub fn closed_by(&self, action: &Action<P>) -> Result<Vec<P>, DockError> {
        Ok(match action {
            Action::Close { panel } => vec![panel.clone()],
            Action::CloseGroup { panel } | Action::CloseWindow { panel } => self.members(panel, unit_of(action))?,
            Action::CloseOthers { panel } => self.members(panel, Unit::Group)?.into_iter().filter(|p| p != panel).collect(),
            _ => Vec::new(),
        })
    }

    fn apply_inner(&mut self, action: Action<P>) -> Result<(), DockError> {
        match action {
            Action::Move { panel, anchor, placement } => {
                if panel == anchor {
                    return Err(DockError::InvalidPlacement);
                }
                if !self.contains(&anchor) {
                    return Err(DockError::MissingTarget);
                }
                if let Placement::Tab { before: Some(before) } = &placement {
                    if before == &panel {
                        // A tab dropped immediately before itself has not moved.
                        return if self.same_group(&panel, &anchor) { Ok(()) } else { Err(DockError::InvalidPlacement) };
                    }
                    if !self.same_group(before, &anchor) {
                        return Err(DockError::MissingTarget);
                    }
                }
                let entry = self.take(&panel).ok_or(DockError::MissingPanel)?;
                self.insert(entry, &anchor, placement)
            }
            Action::Float { panel, rect } => {
                if !valid_rect(rect) {
                    return Err(DockError::InvalidGeometry);
                }
                // A window's only panel keeps the window's id.
                let lone = self.floating.iter().find(|w| w.contains(&panel)).filter(|w| w.panels().count() == 1).map(|w| w.id);
                let entry = self.take(&panel).ok_or(DockError::MissingPanel)?;
                let window = Floating::single(vec![entry.panel], 0, rect);
                self.floating.push(Floating { id: lone.unwrap_or(window.id), ..window });
                Ok(())
            }
            Action::Close { panel } => self.take(&panel).map(|_| ()).ok_or(DockError::MissingPanel),
            Action::Activate { panel } => {
                let place = self.place(&panel).ok_or(DockError::MissingPanel)?;
                if let Some(group) = self.group_mut(&place) {
                    // Selecting a panel shows it: a minimized group expands.
                    group.active = group.panels.iter().position(|p| p == &panel).ok_or(DockError::MissingPanel)?;
                    group.minimized = false;
                    return Ok(());
                }
                if let Place::Floating { window, .. } = place
                    && let Some(window) = self.floating.get_mut(window)
                {
                    window.minimized = false;
                }
                if self.trees_mut().any(|node| activate(node, &panel)) {
                    return Ok(());
                }
                Err(DockError::MissingPanel)
            }
            Action::Open { panel, anchor } => {
                if self.contains(&panel) {
                    return self.apply_inner(Action::Activate { panel });
                }
                let entry = StackEntry { panel, open: true, height: None };
                if let Some(anchor) = anchor {
                    self.insert(entry, &anchor, Placement::Tab { before: None })
                } else if self.root.is_none() {
                    self.root = Some(Node::Tabs { panels: vec![entry.panel], active: 0 });
                    Ok(())
                } else {
                    Err(DockError::MissingTarget)
                }
            }
            Action::OpenAt { panel, anchor, placement } => {
                if self.contains(&panel) {
                    return self.apply_inner(Action::Move { panel, anchor, placement });
                }
                if let Placement::Tab { before: Some(before) } = &placement
                    && !self.same_group(before, &anchor)
                {
                    return Err(DockError::MissingTarget);
                }
                self.insert(StackEntry { panel, open: true, height: None }, &anchor, placement)
            }
            Action::MoveFloating { panel, rect } => {
                if !valid_rect(rect) {
                    return Err(DockError::InvalidGeometry);
                }
                let window = self.floating.iter_mut().find(|g| g.contains(&panel)).ok_or(DockError::MissingPanel)?;
                window.rect = rect;
                Ok(())
            }
            Action::ResizeSplit { path, size } => {
                if path.len() >= MAX_DEPTH {
                    return Err(DockError::LimitExceeded);
                }
                if !valid_size(size) {
                    return Err(DockError::InvalidGeometry);
                }
                resize_split(self.root.as_mut().ok_or(DockError::MissingTarget)?, &path, size)
            }
            Action::ResizeWindowSplit { panel, path, size } => {
                if path.len() >= MAX_DEPTH {
                    return Err(DockError::LimitExceeded);
                }
                if !valid_size(size) {
                    return Err(DockError::InvalidGeometry);
                }
                let window = self.floating.iter_mut().find(|w| w.contains(&panel)).ok_or(DockError::MissingPanel)?;
                resize_split(&mut window.root, &path, size)
            }
            Action::SetStackOpen { panel, open } => {
                let entry = self.trees_mut().find_map(|n| stack_entry_mut(n, &panel)).ok_or(DockError::MissingPanel)?;
                entry.open = open;
                Ok(())
            }
            Action::ResizeStack { panel, height } => {
                if !valid_height(height) {
                    return Err(DockError::InvalidGeometry);
                }
                let entry = self.trees_mut().find_map(|n| stack_entry_mut(n, &panel)).ok_or(DockError::MissingPanel)?;
                entry.height = height;
                Ok(())
            }
            Action::MoveGroup { ref panel, ref to } | Action::MoveColumn { ref panel, ref to } | Action::MoveWindow { ref panel, ref to } => {
                let unit = unit_of(&action);
                if let Target::At { anchor, .. } = to
                    && self.members(panel, unit)?.contains(anchor)
                {
                    // Dropped on itself.
                    return Ok(());
                }
                let moving = self.take_unit(panel, unit)?;
                self.insert_moving(moving, to.clone())
            }
            Action::MovePanel { panel, to } => match to {
                Target::At { anchor, placement } => self.apply_inner(Action::Move { panel, anchor, placement }),
                to => {
                    let entry = self.take(&panel).ok_or(DockError::MissingPanel)?;
                    self.insert_moving(Moving::group(Group::new(vec![entry.panel], 0)), to)
                }
            },
            Action::CloseGroup { ref panel } | Action::CloseWindow { ref panel } => self.take_unit(panel, unit_of(&action)).map(|_| ()),
            Action::CloseOthers { panel } => {
                for other in self.members(&panel, Unit::Group)? {
                    if other != panel {
                        self.take(&other).ok_or(DockError::MissingPanel)?;
                    }
                }
                Ok(())
            }
            Action::SetMinimized { panel, minimized } => {
                let place = self.place(&panel).ok_or(DockError::MissingPanel)?;
                if let Some(group) = self.group_mut(&place) {
                    group.minimized = minimized;
                    return Ok(());
                }
                if let Place::Floating { window, .. } = place {
                    self.floating.get_mut(window).ok_or(DockError::MissingPanel)?.minimized = minimized;
                    return Ok(());
                }
                let entry = self.trees_mut().find_map(|n| stack_entry_mut(n, &panel)).ok_or(DockError::InvalidPlacement)?;
                entry.open = !minimized;
                Ok(())
            }
            Action::ResizeGroup { panel, height } => {
                if !valid_height(height) {
                    return Err(DockError::InvalidGeometry);
                }
                let place = self.place(&panel).ok_or(DockError::MissingPanel)?;
                if let Some(group) = self.group_mut(&place) {
                    group.height = height;
                    return Ok(());
                }
                let entry = self.trees_mut().find_map(|n| stack_entry_mut(n, &panel)).ok_or(DockError::InvalidPlacement)?;
                entry.height = height;
                Ok(())
            }
            Action::SetIconic { panel, iconic } => {
                self.column_of(&panel)?.iconic = iconic;
                Ok(())
            }
            Action::ResizeColumn { panel, width } => {
                if !valid_width(width) {
                    return Err(DockError::InvalidGeometry);
                }
                self.column_of(&panel)?.width = width;
                Ok(())
            }
            Action::ResizeIconic { panel, width } => {
                if width.is_some_and(|w| !valid_width(w)) {
                    return Err(DockError::InvalidGeometry);
                }
                self.column_of(&panel)?.iconic_width = width;
                Ok(())
            }
            Action::SetNative { panel, rect } => {
                if rect.is_some_and(|r| !valid_rect(r)) {
                    return Err(DockError::InvalidGeometry);
                }
                let window = self.floating.iter_mut().find(|g| g.contains(&panel)).ok_or(DockError::InvalidPlacement)?;
                window.native = rect;
                if rect.is_none() {
                    window.monitor = None;
                    window.monitor_size = None;
                }
                Ok(())
            }
            Action::SetMonitor { panel, monitor, monitor_size } => {
                if !valid_monitor(monitor.as_ref(), monitor_size) {
                    return Err(DockError::InvalidGeometry);
                }
                let window = self.floating.iter_mut().find(|g| g.contains(&panel)).ok_or(DockError::InvalidPlacement)?;
                window.monitor = monitor;
                window.monitor_size = monitor_size;
                Ok(())
            }
        }
    }

    fn column_of(&mut self, panel: &P) -> Result<&mut Column<P>, DockError> {
        let place = self.place(panel).ok_or(DockError::MissingPanel)?;
        self.column_mut(&place).ok_or(DockError::InvalidPlacement)
    }

    fn same_group(&self, panel: &P, anchor: &P) -> bool {
        let place = self.place(panel);
        place.is_some() && place == self.place(anchor)
    }

    pub(super) fn take(&mut self, panel: &P) -> Option<StackEntry<P>> {
        let mut removed = None;
        self.root = self.root.take().and_then(|n| remove_node(n, panel, &mut removed));
        if removed.is_some() {
            return removed;
        }
        if let Some(index) = self.floating.iter().position(|w| w.contains(panel))
            && let Some(window) = self.floating.get_mut(index)
        {
            let root = std::mem::replace(&mut window.root, Node::Tabs { panels: Vec::new(), active: 0 });
            match remove_node(root, panel, &mut removed) {
                Some(root) => window.root = root,
                None => {
                    self.floating.remove(index);
                }
            }
        }
        removed
    }

    fn insert(&mut self, entry: StackEntry<P>, anchor: &P, placement: Placement<P>) -> Result<(), DockError> {
        let place = self.place(anchor).ok_or(DockError::MissingTarget)?;
        let tree = place.tree_path().map(|(window, path)| (window, path.to_vec()));
        let Some((window, path)) = tree.filter(|_| !matches!(placement, Placement::Group { .. } | Placement::Column { .. } | Placement::Edge(_)))
        else {
            let group = Group::new(vec![entry.panel], 0);
            return self.insert_moving(Moving::group(group), Target::At { anchor: anchor.clone(), placement });
        };
        let root = self.tree_mut(window).ok_or(DockError::MissingTarget)?;
        if let Placement::Split(zone) = placement
            && zone != Zone::Center
        {
            let new = Node::Tabs { panels: vec![entry.panel], active: 0 };
            return if split_frame(root, &path, new, zone) { Ok(()) } else { Err(DockError::MissingTarget) };
        }
        if insert_node(root, &entry, anchor, &placement) { Ok(()) } else { Err(DockError::MissingTarget) }
    }
}

/// Whether the anchor's permissions take this placement: tabs need `accepts_tabs`, side
/// splits `accepts_splits`.
fn placement_allowed<P>(placement: &Placement<P>, anchor: Permissions) -> bool {
    match placement {
        Placement::Tab { .. } | Placement::Split(Zone::Center) => anchor.accepts_tabs,
        Placement::Split(_) => anchor.accepts_splits,
        _ => true,
    }
}

fn unit_of<P>(action: &Action<P>) -> Unit {
    match action {
        Action::MoveColumn { .. } => Unit::Column,
        Action::MoveWindow { .. } | Action::CloseWindow { .. } => Unit::Window,
        _ => Unit::Group,
    }
}

fn check_panel<'a, P: Eq + Hash>(panel: &'a P, seen: &mut HashSet<&'a P>) -> Result<(), DockError> {
    if seen.len() >= MAX_PANELS {
        return Err(DockError::LimitExceeded);
    }
    if !seen.insert(panel) {
        return Err(DockError::DuplicatePanel);
    }
    Ok(())
}

/// A `Tabs` or `Stack` node: not empty, a valid selection and heights, panels not seen before.
fn check_leaf<'a, P: Eq + Hash>(node: &'a Node<P>, seen: &mut HashSet<&'a P>) -> Result<(), DockError> {
    match node {
        Node::Tabs { panels, active } => {
            if panels.is_empty() {
                return Err(DockError::InvalidGeometry);
            }
            check_tabs(panels, *active, seen)
        }
        Node::Stack { entries } => {
            if entries.is_empty() {
                return Err(DockError::InvalidGeometry);
            }
            if entries.len() > MAX_PANELS {
                return Err(DockError::LimitExceeded);
            }
            for entry in entries {
                if !valid_height(entry.height) {
                    return Err(DockError::InvalidGeometry);
                }
                check_panel(&entry.panel, seen)?;
            }
            Ok(())
        }
        Node::Split { .. } | Node::Dock { .. } => Ok(()),
    }
}

fn check_group<'a, P: Eq + Hash>(group: &'a Group<P>, seen: &mut HashSet<&'a P>) -> Result<(), DockError> {
    if group.panels.is_empty() || !valid_height(group.height) {
        return Err(DockError::InvalidGeometry);
    }
    check_tabs(&group.panels, group.active, seen)
}

fn check_tabs<'a, P: Eq + Hash>(panels: &'a [P], active: usize, seen: &mut HashSet<&'a P>) -> Result<(), DockError> {
    if panels.len() > MAX_PANELS {
        return Err(DockError::LimitExceeded);
    }
    if !valid_active(panels.len(), active) {
        return Err(DockError::InvalidSelection);
    }
    for panel in panels {
        check_panel(panel, seen)?;
    }
    Ok(())
}

fn remove_tab<P: Clone + Eq>(panels: &mut Vec<P>, active: &mut usize, panel: &P) -> Option<P> {
    let index = panels.iter().position(|p| p == panel)?;
    let selected = panels.get(*active).cloned();
    let removed = panels.remove(index);
    *active = selected.as_ref().and_then(|p| panels.iter().position(|q| q == p)).unwrap_or(index.min(panels.len().saturating_sub(1)));
    Some(removed)
}

fn remove_node<P: Clone + Eq>(node: Node<P>, panel: &P, removed: &mut Option<StackEntry<P>>) -> Option<Node<P>> {
    match node {
        Node::Tabs { mut panels, mut active } => {
            if let Some(panel) = remove_tab(&mut panels, &mut active, panel) {
                *removed = Some(StackEntry { panel, open: true, height: None });
            }
            (!panels.is_empty()).then_some(Node::Tabs { panels, active })
        }
        Node::Stack { mut entries } => {
            if let Some(i) = entries.iter().position(|e| &e.panel == panel) {
                *removed = Some(entries.remove(i));
            }
            (!entries.is_empty()).then_some(Node::Stack { entries })
        }
        Node::Split { axis, size, first, second } => {
            let first = remove_node(*first, panel, removed);
            let second = remove_node(*second, panel, removed);
            match (first, second) {
                (Some(first), Some(second)) => Some(Node::Split { axis, size, first: Box::new(first), second: Box::new(second) }),
                (node, None) | (None, node) => node,
            }
        }
        Node::Dock { side, mut columns } => {
            // A dock that was already empty stays; one emptied by this removal collapses.
            let had_columns = !columns.is_empty();
            for column in &mut columns {
                for group in &mut column.groups {
                    if let Some(panel) = remove_tab(&mut group.panels, &mut group.active, panel) {
                        *removed = Some(StackEntry { panel, open: true, height: None });
                    }
                }
                column.groups.retain(|g| !g.panels.is_empty());
            }
            columns.retain(|c| !c.groups.is_empty());
            (!had_columns || !columns.is_empty()).then_some(Node::Dock { side, columns })
        }
    }
}

fn resize_split<P>(mut node: &mut Node<P>, path: &[bool], size: SplitSize) -> Result<(), DockError> {
    for second_child in path {
        let Node::Split { first, second, .. } = node else { return Err(DockError::MissingTarget) };
        node = if *second_child { second } else { first };
    }
    let Node::Split { size: old, .. } = node else { return Err(DockError::MissingTarget) };
    *old = size;
    Ok(())
}

fn activate<P: Eq>(node: &mut Node<P>, panel: &P) -> bool {
    match node {
        Node::Tabs { panels, active } => {
            if let Some(i) = panels.iter().position(|p| p == panel) {
                *active = i;
                true
            } else {
                false
            }
        }
        Node::Stack { entries } => {
            if let Some(entry) = entries.iter_mut().find(|e| &e.panel == panel) {
                entry.open = true;
                true
            } else {
                false
            }
        }
        Node::Split { first, second, .. } => activate(first, panel) || activate(second, panel),
        Node::Dock { .. } => false,
    }
}

fn stack_entry_mut<'a, P: Eq>(node: &'a mut Node<P>, panel: &P) -> Option<&'a mut StackEntry<P>> {
    match node {
        Node::Stack { entries } => entries.iter_mut().find(|e| &e.panel == panel),
        Node::Split { first, second, .. } => stack_entry_mut(first, panel).or_else(|| stack_entry_mut(second, panel)),
        Node::Tabs { .. } | Node::Dock { .. } => None,
    }
}

fn insert_node<P: Clone + Eq>(node: &mut Node<P>, entry: &StackEntry<P>, anchor: &P, placement: &Placement<P>) -> bool {
    let here = match node {
        Node::Split { first, second, .. } => return insert_node(first, entry, anchor, placement) || insert_node(second, entry, anchor, placement),
        Node::Tabs { panels, .. } => panels.contains(anchor),
        Node::Stack { entries } => entries.iter().any(|e| &e.panel == anchor),
        Node::Dock { .. } => false,
    };
    if !here {
        return false;
    }
    match node {
        Node::Tabs { panels, active } => {
            let index = match placement {
                Placement::Tab { before } => before.as_ref().and_then(|p| panels.iter().position(|q| q == p)).unwrap_or(panels.len()),
                _ => panels.iter().position(|p| p == anchor).map_or(panels.len(), |i| i + 1),
            };
            panels.insert(index, entry.panel.clone());
            *active = index;
        }
        Node::Stack { entries } => {
            let index = match placement {
                Placement::Tab { before } => before.as_ref().and_then(|p| entries.iter().position(|e| &e.panel == p)).unwrap_or(entries.len()),
                _ => entries.iter().position(|e| &e.panel == anchor).map_or(entries.len(), |i| i + 1),
            };
            entries.insert(index, entry.clone());
        }
        Node::Split { .. } | Node::Dock { .. } => return false,
    }
    true
}

fn group_location<P: Clone + Eq>(panels: &[P], panel: &P) -> Option<Location<P>> {
    let index = panels.iter().position(|p| p == panel)?;
    let anchor = panels.iter().find(|p| *p != panel).cloned();
    let before = panels.get(index + 1).cloned();
    Some(Location { floating: None, anchor, placement: Placement::Tab { before }, size: None, stack: None, sibling: Vec::new(), group: None })
}

fn first_panel<P>(node: &Node<P>) -> Option<&P> {
    match node {
        Node::Tabs { panels, .. } => panels.first(),
        Node::Stack { entries } => entries.first().map(|e| &e.panel),
        Node::Split { first, second, .. } => first_panel(first).or_else(|| first_panel(second)),
        Node::Dock { columns, .. } => columns.iter().flat_map(|c| c.groups.iter()).find_map(|g| g.panels.first()),
    }
}

fn node_location<P: Clone + Eq>(node: &Node<P>, panel: &P) -> Option<Location<P>> {
    match node {
        Node::Tabs { panels, .. } => group_location(panels, panel),
        Node::Stack { entries } => {
            let entry = entries.iter().find(|e| &e.panel == panel)?;
            let panels: Vec<P> = entries.iter().map(|e| e.panel.clone()).collect();
            let mut location = group_location(&panels, panel)?;
            location.stack = Some((entry.open, entry.height));
            Some(location)
        }
        Node::Split { axis, size, first, second } => {
            let (mut location, sibling, is_first) = if let Some(location) = node_location(first, panel) {
                (location, second, true)
            } else {
                (node_location(second, panel)?, first, false)
            };
            if location.anchor.is_none()
                && let Some(anchor) = first_panel(sibling)
            {
                location.anchor = Some(anchor.clone());
                location.placement = Placement::Split(match (*axis, is_first) {
                    (SplitAxis::Horizontal, true) => Zone::Left,
                    (SplitAxis::Horizontal, false) => Zone::Right,
                    (SplitAxis::Vertical, true) => Zone::Top,
                    (SplitAxis::Vertical, false) => Zone::Bottom,
                });
                location.size = Some(*size);
                let mut members = Vec::new();
                node_members(sibling, &mut members);
                location.sibling = members.into_iter().cloned().collect();
            }
            Some(location)
        }
        Node::Dock { .. } => None,
    }
}

fn restore_parent_size<P: Eq>(node: &mut Node<P>, panel: &P, size: SplitSize) -> bool {
    fn alone<P: Eq>(node: &Node<P>, panel: &P) -> bool {
        matches!(node, Node::Tabs { panels, .. } if panels.len() == 1 && panels.first() == Some(panel))
    }
    if let Node::Split { size: current, first, second, .. } = node {
        if alone(first, panel) || alone(second, panel) {
            *current = size;
            true
        } else {
            restore_parent_size(first, panel, size) || restore_parent_size(second, panel, size)
        }
    } else {
        false
    }
}

fn restore_stack_kind<P: Clone + Eq>(node: &mut Node<P>, panel: &P) {
    match node {
        Node::Tabs { panels, .. } if panels.len() == 1 && panels.first() == Some(panel) => {
            *node = Node::Stack { entries: vec![StackEntry { panel: panel.clone(), open: true, height: None }] };
        }
        Node::Split { first, second, .. } => {
            restore_stack_kind(first, panel);
            restore_stack_kind(second, panel);
        }
        _ => {}
    }
}

fn node_members<'a, P>(node: &'a Node<P>, members: &mut Vec<&'a P>) {
    match node {
        Node::Tabs { panels, .. } => members.extend(panels),
        Node::Stack { entries } => members.extend(entries.iter().map(|e| &e.panel)),
        Node::Split { first, second, .. } => {
            node_members(first, members);
            node_members(second, members);
        }
        Node::Dock { columns, .. } => members.extend(columns.iter().flat_map(|c| c.groups.iter()).flat_map(|g| g.panels.iter())),
    }
}

fn restore_around_subtree<P: Clone + Eq + Hash>(node: &mut Node<P>, panel: &P, members: &HashSet<&P>, zone: Zone, size: SplitSize) -> bool {
    let mut actual = Vec::new();
    node_members(node, &mut actual);
    if actual.len() == members.len() && actual.iter().all(|p| members.contains(p)) {
        let new = Node::Tabs { panels: vec![panel.clone()], active: 0 };
        let old = std::mem::replace(node, Node::Tabs { panels: Vec::new(), active: 0 });
        let (first, second) = if matches!(zone, Zone::Left | Zone::Top) { (new, old) } else { (old, new) };
        *node = Node::Split {
            axis: if matches!(zone, Zone::Top | Zone::Bottom) { SplitAxis::Vertical } else { SplitAxis::Horizontal },
            size,
            first: Box::new(first),
            second: Box::new(second),
        };
        true
    } else if let Node::Split { first, second, .. } = node {
        restore_around_subtree(first, panel, members, zone, size) || restore_around_subtree(second, panel, members, zone, size)
    } else {
        false
    }
}
