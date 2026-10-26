//! Docks, columns, groups and floating windows: their types and the structural edits behind
//! group, column and window moves.
//!
//! A group, column or floating window is named by a panel it contains. Edits run on a proposed
//! copy of the layout, so a helper that fails part-way leaves nothing behind.

use std::{collections::HashMap, hash::Hash};

use super::model::{DockError, Floating, Layout, MAX_DEPTH, MAX_NODES, MAX_PANELS, Node, Placement, Zone, valid_height, valid_rect, valid_width};
use crate::layout::{SplitAxis, SplitSize};

/// Width of a new column when nothing better is known.
pub(super) const DEFAULT_COLUMN_WIDTH: f32 = 240.0;
/// Rect of a new floating window when the caller gives none and the source has no size.
const DEFAULT_FLOAT: [f32; 4] = [48.0, 48.0, 260.0, 320.0];

/// The window edge a [`Node::Dock`] sits on, in reading order so right-to-left interfaces
/// mirror. `Start` is the edge of a horizontal split's first child (left in a left-to-right
/// layout); the renderer maps it to a physical edge from the layout direction.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[non_exhaustive]
pub enum Side {
    Start,
    End,
}

/// Tabs that move, minimize and size together in a dock column.
#[derive(Clone, Debug, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(bound(deserialize = "P: serde::Deserialize<'de>")))]
pub struct Group<P> {
    pub panels: Vec<P>,
    pub active: usize,
    /// Only the tab strip shows. `height` is kept for when the group expands again.
    #[cfg_attr(feature = "serde", serde(default))]
    pub minimized: bool,
    /// Preferred height in points, set when the user resizes the group; `None` shares the free
    /// height of the column or stack, or in a dock column takes
    /// [`DockContent::dock_heights`](crate::docking::DockContent::dock_heights).
    #[cfg_attr(feature = "serde", serde(default))]
    pub height: Option<f32>,
}

impl<P> Group<P> {
    pub fn new(panels: Vec<P>, active: usize) -> Self {
        Self { panels, active, minimized: false, height: None }
    }

    pub fn active_panel(&self) -> Option<&P> {
        self.panels.get(self.active)
    }
}

/// One column of a dock: groups from top to bottom.
#[derive(Clone, Debug, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(bound(deserialize = "P: serde::Deserialize<'de>")))]
pub struct Column<P> {
    pub groups: Vec<Group<P>>,
    /// Expanded width in points.
    pub width: f32,
    /// Collapsed to one icon per panel.
    #[cfg_attr(feature = "serde", serde(default))]
    pub iconic: bool,
    /// Width while iconic; labels show beside the icons when it is wide enough.
    #[cfg_attr(feature = "serde", serde(default))]
    pub iconic_width: Option<f32>,
}

impl<P> Column<P> {
    pub fn new(groups: Vec<Group<P>>, width: f32) -> Self {
        Self { groups, width, iconic: false, iconic_width: None }
    }
}

/// Where a group, column or floating window goes.
#[derive(Clone, Debug, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[non_exhaustive]
pub enum Target<P> {
    /// Relative to the panel `anchor`, which must not be part of what moves.
    At { anchor: P, placement: Placement<P> },
    /// A new dock on an edge that has none (or an empty one). The root becomes the other side
    /// of a horizontal split.
    NewDock { side: Side },
    /// A new floating window. `None` keeps the source's size at a default position.
    Float { rect: Option<[f32; 4]> },
}

impl<P: Clone + Eq + Hash> Placement<P> {
    /// The placement for a drop zone on `anchor`'s group, in the vocabulary its container
    /// accepts. `zone` is physical: `Left` is the left half on screen in either direction.
    ///
    /// | Anchor in | `Center` | `Top`, `Bottom` | `Left`, `Right` |
    /// |---|---|---|---|
    /// | the split tree or a floating window | [`Placement::Split`] | `Split` | `Split` |
    /// | a dock column | `Split(Center)` (join as tabs) | [`Placement::Group`] above or below | [`Placement::Column`] on that side |
    ///
    /// A dock's `Column { after }` counts from its window edge, which depends on the dock's
    /// side and `right_to_left` (see [`Side`]); this picks `after` so the new column lands on
    /// the zone's side of the anchor's column.
    pub fn for_zone(layout: &Layout<P>, anchor: &P, zone: Zone, right_to_left: bool) -> Result<Self, DockError> {
        let place = layout.place(anchor).ok_or(DockError::MissingTarget)?;
        Ok(match (place, zone) {
            (Place::Node(_) | Place::Floating { .. }, zone) | (_, zone @ Zone::Center) => Placement::Split(zone),
            (Place::Dock { .. }, Zone::Top | Zone::Bottom) => Placement::Group { below: zone == Zone::Bottom, height: None },
            (Place::Dock { path, .. }, _) => {
                let side = match layout.root.as_ref().and_then(|root| node_at(root, &path)) {
                    Some(Node::Dock { side, .. }) => *side,
                    _ => return Err(DockError::MissingTarget),
                };
                // The physical edge of the dock: Start is left in a left-to-right layout.
                let left_edge = (side == Side::Start) != right_to_left;
                // `after` is away from the window edge: rightwards for a left dock.
                Placement::Column { after: (zone == Zone::Right) == left_edge }
            }
        })
    }
}

/// What the Window menu shows for a panel. The model does not track the z-order of floating
/// windows, so `Front` means "showing": the active tab of an expanded group in an expanded column.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PanelState {
    Hidden,
    Open,
    Front,
}

/// The group a closed panel was in, when that group was in a dock.
#[derive(Clone, Debug, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct GroupLocation {
    pub minimized: bool,
    pub height: Option<f32>,
    /// The column, when the group was docked.
    pub column: Option<ColumnLocation>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct ColumnLocation {
    pub side: Side,
    pub width: f32,
    pub iconic: bool,
    pub iconic_width: Option<f32>,
}

/// Where a panel is. Paths lead from the root: false is a split's first child, true its second.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum Place {
    /// A `Tabs` or `Stack` node.
    Node(Vec<bool>),
    Dock {
        path: Vec<bool>,
        column: usize,
        group: usize,
    },
    /// A `Tabs` or `Stack` node of a floating window's tree.
    Floating {
        window: usize,
        path: Vec<bool>,
    },
}

impl Place {
    /// The tree (`None` for the docked layout, else a floating window) and the path of a
    /// `Tabs` or `Stack` node.
    pub(super) fn tree_path(&self) -> Option<(Option<usize>, &[bool])> {
        match self {
            Place::Node(path) => Some((None, path)),
            Place::Floating { window, path } => Some((Some(*window), path)),
            Place::Dock { .. } => None,
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
    pub rect: Option<[f32; 4]>,
    pub native: Option<[f32; 4]>,
    /// The id of a whole floating window in transit, which a new window keeps.
    pub id: Option<u64>,
}

impl<P> Moving<P> {
    pub fn group(group: Group<P>) -> Self {
        Self { groups: vec![group], node: None, minimized: false, width: None, iconic: false, iconic_width: None, rect: None, native: None, id: None }
    }

    fn into_column(self, width: f32) -> Column<P> {
        Column { groups: self.groups, width: self.width.unwrap_or(width), iconic: self.iconic, iconic_width: self.iconic_width }
    }
}

pub(super) fn node_at<'a, P>(mut node: &'a Node<P>, path: &[bool]) -> Option<&'a Node<P>> {
    for second_child in path {
        let Node::Split { first, second, .. } = node else { return None };
        node = if *second_child { second } else { first };
    }
    Some(node)
}

pub(super) fn node_at_mut<'a, P>(mut node: &'a mut Node<P>, path: &[bool]) -> Option<&'a mut Node<P>> {
    for second_child in path {
        let Node::Split { first, second, .. } = node else { return None };
        node = if *second_child { second } else { first };
    }
    Some(node)
}

/// Every `Tabs`, `Stack` and `Dock` node of a tree with its path, first to last. Stops at
/// [`MAX_NODES`] for an invalid tree.
pub(super) fn leaves<P>(root: &Node<P>) -> Vec<(&Node<P>, Vec<bool>)> {
    let mut out = Vec::new();
    let mut visit = vec![(root, Vec::new())];
    let mut visited = 0usize;
    while let Some((node, path)) = visit.pop() {
        visited = visited.saturating_add(1);
        if visited > MAX_NODES {
            break;
        }
        match node {
            Node::Split { first, second, .. } => {
                let mut second_path = path.clone();
                second_path.push(true);
                let mut first_path = path;
                first_path.push(false);
                visit.push((second, second_path));
                visit.push((first, first_path));
            }
            leaf => out.push((leaf, path)),
        }
    }
    out
}

/// The panels of a tree in order, without recursion; at most [`MAX_PANELS`].
pub(super) fn tree_panels<P>(root: &Node<P>) -> Vec<&P> {
    let mut panels = Vec::new();
    for (leaf, _) in leaves(root) {
        let room = MAX_PANELS.saturating_sub(panels.len());
        match leaf {
            Node::Tabs { panels: group, .. } => panels.extend(group.iter().take(room)),
            Node::Stack { entries } => panels.extend(entries.iter().map(|e| &e.panel).take(room)),
            Node::Dock { columns, .. } => panels.extend(columns.iter().flat_map(|c| &c.groups).flat_map(|g| &g.panels).take(room)),
            Node::Split { .. } => {}
        }
    }
    panels
}

/// The shown panel of a tree's first group.
#[cfg(feature = "native-windows")]
pub(super) fn first_active<P>(root: &Node<P>) -> Option<&P> {
    match leaves(root).into_iter().next()?.0 {
        Node::Tabs { panels, active } => panels.get(*active),
        Node::Stack { entries } => entries.first().map(|e| &e.panel),
        Node::Split { .. } | Node::Dock { .. } => None,
    }
}

/// The groups of a tree, first to last: what a dock column takes from a floating window.
pub(super) fn tree_groups<P: Clone>(root: &Node<P>) -> Vec<Group<P>> {
    let mut groups = Vec::new();
    for (leaf, _) in leaves(root) {
        match leaf {
            Node::Tabs { panels, active } => groups.push(Group::new(panels.clone(), *active)),
            Node::Stack { entries } => groups.push(Group::new(entries.iter().map(|e| e.panel.clone()).collect(), 0)),
            Node::Dock { columns, .. } => groups.extend(columns.iter().flat_map(|c| c.groups.iter().cloned())),
            Node::Split { .. } => {}
        }
    }
    groups
}

/// Groups stacked top to bottom as a tree: each group over a vertical split of the ones below
/// it, sized by their heights, with an equal share where a group has none.
pub(super) fn stack_tree<P>(groups: Vec<Group<P>>) -> Option<Node<P>> {
    let height = |g: &Group<P>| g.height.filter(|h| h.is_finite() && *h > 0.0);
    let known: Vec<f32> = groups.iter().filter_map(height).collect();
    let share = if known.is_empty() { 1.0 } else { known.iter().sum::<f32>() / known.len() as f32 };
    let weights: Vec<f32> = groups.iter().map(|g| height(g).unwrap_or(share)).collect();
    let mut node = None;
    let mut below = 0.0;
    for (group, weight) in groups.into_iter().zip(weights).rev() {
        let tabs = Node::Tabs { panels: group.panels, active: group.active };
        node = Some(match node {
            None => tabs,
            Some(rest) => {
                let ratio = (weight / (weight + below)).clamp(0.0, 1.0);
                Node::Split { axis: SplitAxis::Vertical, size: SplitSize::Ratio(ratio), first: Box::new(tabs), second: Box::new(rest) }
            }
        });
        below += weight;
    }
    node
}

/// Put `new` on the `zone` side of the frame at `path`: the frame splits into two halves and
/// every other frame keeps its size.
pub(super) fn split_frame<P>(root: &mut Node<P>, path: &[bool], new: Node<P>, zone: Zone) -> bool {
    let axis = if matches!(zone, Zone::Top | Zone::Bottom) { SplitAxis::Vertical } else { SplitAxis::Horizontal };
    let new_first = matches!(zone, Zone::Top | Zone::Left);
    let Some(node) = node_at_mut(root, path) else { return false };
    let old = std::mem::replace(node, Node::Tabs { panels: Vec::new(), active: 0 });
    let (first, second) = if new_first { (new, old) } else { (old, new) };
    *node = Node::Split { axis, size: SplitSize::Ratio(0.5), first: Box::new(first), second: Box::new(second) };
    true
}

/// The depth along the frame `path` of the split whose size a [`Placement::SplitSized`] on
/// `zone` replaces: the nearest enclosing split along the zone's axis (`on_axis` at a depth)
/// whose divider runs along the frame's `zone` side, else, for a frame at the edge of its
/// area on that side, the nearest one along its opposite side.
pub(super) fn sized_split_depth(path: &[bool], zone: Zone, on_axis: impl Fn(usize) -> bool) -> Option<usize> {
    // A divider on the frame's start side (left or top) belongs to a split holding the frame
    // in its second child.
    let start = matches!(zone, Zone::Left | Zone::Top);
    let nearest = |second: bool| (0..path.len()).rev().find(|&depth| path.get(depth) == Some(&second) && on_axis(depth));
    nearest(start).or_else(|| nearest(!start))
}

/// [`split_frame`] with the sizes of [`Placement::SplitSized`]: the new split takes `size`, and
/// the split [`sized_split_depth`] finds takes `outer`. False when there is no such split, or a
/// dock is beyond it.
pub(super) fn split_frame_sized<P>(root: &mut Node<P>, path: &[bool], new: Node<P>, zone: Zone, size: SplitSize, outer: SplitSize) -> bool {
    let axis = if matches!(zone, Zone::Top | Zone::Bottom) { SplitAxis::Vertical } else { SplitAxis::Horizontal };
    let enclosing =
        sized_split_depth(path, zone, |depth| matches!(node_at(root, &path[..depth]), Some(Node::Split { axis: own, .. }) if *own == axis));
    let Some(depth) = enclosing else { return false };
    let Some(Node::Split { size: own, first, second, .. }) = node_at_mut(root, &path[..depth]) else { return false };
    if zone == Zone::Center || matches!(**first, Node::Dock { .. }) || matches!(**second, Node::Dock { .. }) {
        return false;
    }
    *own = outer;
    if !split_frame(root, path, new, zone) {
        return false;
    }
    match node_at_mut(root, path) {
        Some(Node::Split { size: own, .. }) => {
            *own = size;
            true
        }
        _ => false,
    }
}

/// Insert `new` at `index` and select the member at `index + selected`.
fn splice<P>(panels: &mut Vec<P>, active: &mut usize, index: usize, new: Vec<P>, selected: usize) {
    let index = index.min(panels.len());
    let count = new.len();
    panels.splice(index..index, new);
    *active = index + selected.min(count.saturating_sub(1));
}

impl<P: Clone + Eq + Hash> Layout<P> {
    /// The layout without its sizes and rectangles: which panels are where, and their state.
    /// A run of splits along one axis nests one way (`a | (b | c)`), since nesting it the
    /// other way only moves sizes between its members.
    pub(super) fn shape(&self) -> Layout<P> {
        fn run<'a, P>(node: &'a Node<P>, axis: SplitAxis, out: &mut Vec<&'a Node<P>>) {
            match node {
                Node::Split { axis: own, first, second, .. } if *own == axis => {
                    run(first, axis, out);
                    run(second, axis, out);
                }
                node => out.push(node),
            }
        }
        fn node<P: Clone>(n: &Node<P>) -> Node<P> {
            match n {
                Node::Split { axis, .. } => {
                    let mut members = Vec::new();
                    run(n, *axis, &mut members);
                    let mut shaped = members.into_iter().rev().map(node);
                    let last = shaped.next().unwrap_or(Node::Tabs { panels: Vec::new(), active: 0 });
                    shaped.fold(last, |second, first| Node::Split {
                        axis: *axis,
                        size: SplitSize::Ratio(0.5),
                        first: Box::new(first),
                        second: Box::new(second),
                    })
                }
                Node::Tabs { panels, active } => Node::Tabs { panels: panels.clone(), active: *active },
                Node::Stack { entries } => Node::Stack { entries: entries.iter().map(|e| super::StackEntry { height: None, ..e.clone() }).collect() },
                Node::Dock { side, columns } => Node::Dock {
                    side: *side,
                    columns: columns
                        .iter()
                        .map(|c| Column {
                            groups: c.groups.iter().map(|g| Group { height: None, ..g.clone() }).collect(),
                            width: 0.0,
                            iconic: c.iconic,
                            iconic_width: None,
                        })
                        .collect(),
                },
            }
        }
        Layout {
            root: self.root.as_ref().map(node),
            floating: self
                .floating
                .iter()
                .map(|w| Floating { root: node(&w.root), rect: [0.0; 4], native: None, monitor: None, monitor_size: None, ..w.clone() })
                .collect(),
        }
    }

    /// Whether `other` arranges the panels as this layout does: equal apart from split sizes,
    /// column widths, group heights and window rectangles. A drop that leads from one to the
    /// other changes nothing the user would see as a move.
    pub(super) fn same_arrangement(&self, other: &Self) -> bool {
        self.shape() == other.shape()
    }

    pub(super) fn place(&self, panel: &P) -> Option<Place> {
        for (w, window) in self.floating.iter().enumerate() {
            for (leaf, path) in leaves(&window.root) {
                if leaf_holds(leaf, panel) {
                    return Some(Place::Floating { window: w, path });
                }
            }
        }
        for (leaf, path) in self.root.as_ref().map(leaves).unwrap_or_default() {
            match leaf {
                Node::Dock { columns, .. } => {
                    for (c, column) in columns.iter().enumerate() {
                        if let Some(g) = column.groups.iter().position(|group| group.panels.contains(panel)) {
                            return Some(Place::Dock { path, column: c, group: g });
                        }
                    }
                }
                leaf if leaf_holds(leaf, panel) => return Some(Place::Node(path)),
                _ => {}
            }
        }
        None
    }

    /// [`Self::place`] of every panel, from one traversal: the renderer looks up many panels a
    /// frame.
    pub(super) fn places(&self) -> HashMap<P, Place> {
        fn note<P: Clone + Eq + Hash>(places: &mut HashMap<P, Place>, leaf: &Node<P>, place: &dyn Fn() -> Place) {
            match leaf {
                Node::Tabs { panels, .. } => {
                    for panel in panels {
                        places.entry(panel.clone()).or_insert_with(place);
                    }
                }
                Node::Stack { entries } => {
                    for entry in entries {
                        places.entry(entry.panel.clone()).or_insert_with(place);
                    }
                }
                _ => {}
            }
        }
        let mut places = HashMap::new();
        for (w, window) in self.floating.iter().enumerate() {
            for (leaf, path) in leaves(&window.root) {
                note(&mut places, leaf, &|| Place::Floating { window: w, path: path.clone() });
            }
        }
        for (leaf, path) in self.root.as_ref().map(leaves).unwrap_or_default() {
            if let Node::Dock { columns, .. } = leaf {
                for (c, column) in columns.iter().enumerate() {
                    for (g, group) in column.groups.iter().enumerate() {
                        for panel in &group.panels {
                            places.entry(panel.clone()).or_insert_with(|| Place::Dock { path: path.clone(), column: c, group: g });
                        }
                    }
                }
            } else {
                note(&mut places, leaf, &|| Place::Node(path.clone()));
            }
        }
        places
    }

    /// The docked layout (`None`) or a floating window's tree.
    pub(super) fn tree(&self, window: Option<usize>) -> Option<&Node<P>> {
        match window {
            None => self.root.as_ref(),
            Some(window) => self.floating.get(window).map(|w| &w.root),
        }
    }

    pub(super) fn tree_mut(&mut self, window: Option<usize>) -> Option<&mut Node<P>> {
        match window {
            None => self.root.as_mut(),
            Some(window) => self.floating.get_mut(window).map(|w| &mut w.root),
        }
    }

    /// The docked layout's tree, then each floating window's.
    pub(super) fn trees_mut(&mut self) -> impl Iterator<Item = &mut Node<P>> {
        self.root.iter_mut().chain(self.floating.iter_mut().map(|w| &mut w.root))
    }

    /// The `Tabs` or `Stack` node at a tree place.
    pub(super) fn node_of(&self, place: &Place) -> Option<&Node<P>> {
        let (window, path) = place.tree_path()?;
        node_at(self.tree(window)?, path)
    }

    fn node_of_mut(&mut self, place: &Place) -> Option<&mut Node<P>> {
        let (window, path) = place.tree_path()?;
        node_at_mut(self.tree_mut(window)?, path)
    }

    pub(super) fn dock_path(&self, side: Side) -> Option<Vec<bool>> {
        let mut visit: Vec<(&Node<P>, Vec<bool>)> = self.root.iter().map(|node| (node, Vec::new())).collect();
        let mut visited = 0usize;
        while let Some((node, path)) = visit.pop() {
            visited = visited.saturating_add(1);
            if visited > MAX_NODES {
                return None;
            }
            match node {
                Node::Split { first, second, .. } => {
                    let mut second_path = path.clone();
                    second_path.push(true);
                    let mut first_path = path;
                    first_path.push(false);
                    visit.push((second, second_path));
                    visit.push((first, first_path));
                }
                Node::Dock { side: s, .. } if *s == side => return Some(path),
                _ => {}
            }
        }
        None
    }

    /// The node an edge split at `zone` wraps: the root, or the part between the docks. Where
    /// neither side of a side-by-side split is a dock and only one holds a `canvas` panel,
    /// that side: the canvas column, without the side frames. A top (bottom) edge also
    /// descends a split of rows into its first (last) row when only that row holds a `canvas`
    /// panel, so a full-width row below (above) the canvas keeps its edge.
    pub(in crate::docking) fn edge_path(&self, zone: Zone, canvas: &dyn Fn(&P) -> bool) -> Vec<bool> {
        let holds = |node: &Node<P>| tree_panels(node).into_iter().any(canvas);
        let mut path = Vec::new();
        let mut node = self.root.as_ref();
        while let Some(Node::Split { axis, first, second, .. }) = node {
            if path.len() > MAX_DEPTH {
                break;
            }
            let second_side = match (axis, &**first, &**second) {
                (SplitAxis::Horizontal, Node::Dock { .. }, _) => true,
                (SplitAxis::Horizontal, _, Node::Dock { .. }) => false,
                (SplitAxis::Vertical, ..) => match (zone, holds(first), holds(second)) {
                    (Zone::Top, true, false) => false,
                    (Zone::Bottom, false, true) => true,
                    _ => break,
                },
                _ => match (holds(first), holds(second)) {
                    (true, false) => false,
                    (false, true) => true,
                    _ => break,
                },
            };
            path.push(second_side);
            node = Some(if second_side { second } else { first });
        }
        path
    }

    /// Whether an edge drop at `zone` that narrows by `canvas` wraps less than one that does
    /// not: the edge area is narrower than the part between the docks.
    pub(in crate::docking) fn edge_narrows(&self, zone: Zone, canvas: &dyn Fn(&P) -> bool) -> bool {
        self.edge_path(zone, canvas) != self.edge_path(zone, &|_| false)
    }

    /// The docked panel, other than `skip`, that a [`Placement::Edge`] anchors on so the drop
    /// wraps the edge area it shows: a `protected` one where that area is narrowed to the
    /// canvas column (`narrowed`), else one that is not protected, since a protected anchor
    /// narrows the model's edge. Any docked panel when every one is protected.
    pub(in crate::docking) fn edge_anchor(&self, narrowed: bool, protected: &dyn Fn(&P) -> bool, skip: &dyn Fn(&P) -> bool) -> Option<P> {
        let docked: Vec<&P> =
            self.panels().into_iter().filter(|p| !skip(p) && matches!(self.place(p), Some(Place::Dock { .. } | Place::Node(_)))).collect();
        docked.iter().find(|p| protected(p) == narrowed).or(docked.first()).map(|p| (*p).clone())
    }

    pub(super) fn columns_mut(&mut self, path: &[bool]) -> Option<&mut Vec<Column<P>>> {
        match self.root.as_mut().and_then(|root| node_at_mut(root, path)) {
            Some(Node::Dock { columns, .. }) => Some(columns),
            _ => None,
        }
    }

    pub(super) fn column(&self, place: &Place) -> Option<&Column<P>> {
        let Place::Dock { path, column, .. } = place else { return None };
        match self.root.as_ref().and_then(|root| node_at(root, path)) {
            Some(Node::Dock { columns, .. }) => columns.get(*column),
            _ => None,
        }
    }

    pub(super) fn column_mut(&mut self, place: &Place) -> Option<&mut Column<P>> {
        let Place::Dock { path, column, .. } = place else { return None };
        self.columns_mut(path)?.get_mut(*column)
    }

    /// The dock group at `place`; `None` for a `Tabs` or `Stack` node.
    pub(super) fn group(&self, place: &Place) -> Option<&Group<P>> {
        match place {
            Place::Dock { group, .. } => self.column(place)?.groups.get(*group),
            Place::Node(_) | Place::Floating { .. } => None,
        }
    }

    pub(super) fn group_mut(&mut self, place: &Place) -> Option<&mut Group<P>> {
        match place {
            Place::Dock { group, .. } => self.column_mut(place)?.groups.get_mut(*group),
            Place::Node(_) | Place::Floating { .. } => None,
        }
    }

    /// Remove the node at `path` of the docked layout; its parent split collapses into the
    /// sibling.
    pub(super) fn take_node(&mut self, path: &[bool]) -> Option<Node<P>> {
        self.take_node_in(None, path)
    }

    /// [`Self::take_node`] in the docked layout (`None`) or a floating window, which closes
    /// when its root goes.
    fn take_node_in(&mut self, window: Option<usize>, path: &[bool]) -> Option<Node<P>> {
        let Some((second_child, parent_path)) = path.split_last() else {
            return match window {
                None => self.root.take(),
                Some(window) => (window < self.floating.len()).then(|| self.floating.remove(window).root),
            };
        };
        let parent = self.tree_mut(window).and_then(|root| node_at_mut(root, parent_path))?;
        if !matches!(parent, Node::Split { .. }) {
            return None;
        }
        match std::mem::replace(parent, Node::Tabs { panels: Vec::new(), active: 0 }) {
            Node::Split { first, second, .. } => {
                let (taken, kept) = if *second_child { (second, first) } else { (first, second) };
                *parent = *kept;
                Some(*taken)
            }
            other => {
                *parent = other;
                None
            }
        }
    }

    /// The members of the group, column or floating window containing `panel`, in order.
    /// An accordion entry is a group of one.
    pub(super) fn members(&self, panel: &P, unit: Unit) -> Result<Vec<P>, DockError> {
        let place = self.place(panel).ok_or(DockError::MissingPanel)?;
        match (unit, &place) {
            (Unit::Group, Place::Node(_) | Place::Floating { .. }) => match self.node_of(&place) {
                Some(Node::Tabs { panels, .. }) => Ok(panels.clone()),
                Some(Node::Stack { .. }) => Ok(vec![panel.clone()]),
                _ => Err(DockError::MissingPanel),
            },
            (Unit::Group, _) => self.group(&place).map(|g| g.panels.clone()).ok_or(DockError::MissingPanel),
            (Unit::Column, Place::Dock { .. }) => {
                let column = self.column(&place).ok_or(DockError::MissingPanel)?;
                Ok(column.groups.iter().flat_map(|g| g.panels.iter().cloned()).collect())
            }
            (Unit::Window, Place::Floating { window, .. }) => {
                let window = self.floating.get(*window).ok_or(DockError::MissingPanel)?;
                Ok(window.panels().cloned().collect())
            }
            (Unit::Column | Unit::Window, _) => Err(DockError::InvalidPlacement),
        }
    }

    pub(super) fn take_unit(&mut self, panel: &P, unit: Unit) -> Result<Moving<P>, DockError> {
        match unit {
            Unit::Group => self.take_group(panel),
            Unit::Column => self.take_column(panel),
            Unit::Window => self.take_window(panel),
        }
    }

    fn take_group(&mut self, panel: &P) -> Result<Moving<P>, DockError> {
        let place = self.place(panel).ok_or(DockError::MissingPanel)?;
        match place {
            Place::Node(_) | Place::Floating { .. } => {
                let (window, path) = place.tree_path().map(|(w, p)| (w, p.to_vec())).ok_or(DockError::MissingPanel)?;
                // A group leaving a floating window keeps its width; the window's only group
                // takes the window's rect along.
                let source = window.and_then(|w| self.floating.get(w)).map(|w| (w.rect, w.native, w.minimized, w.id));
                let whole = |moving: Moving<P>| match source {
                    Some((rect, native, minimized, id)) if path.is_empty() => {
                        Moving { width: rect.get(2).copied(), rect: Some(rect), native, minimized, id: Some(id), ..moving }
                    }
                    Some((rect, ..)) => Moving { width: rect.get(2).copied(), ..moving },
                    None => moving,
                };
                if let Some(Node::Stack { entries }) = self.node_of_mut(&place)
                    && entries.len() > 1
                    && let Some(index) = entries.iter().position(|e| &e.panel == panel)
                {
                    let entry = entries.remove(index);
                    let group = Group { panels: vec![entry.panel], active: 0, minimized: !entry.open, height: entry.height };
                    let moving = whole(Moving::group(group));
                    return Ok(Moving { rect: None, native: None, id: None, ..moving });
                }
                match self.take_node_in(window, &path) {
                    Some(Node::Tabs { panels, active }) => Ok(whole(Moving::group(Group::new(panels, active)))),
                    Some(Node::Stack { mut entries }) if entries.len() == 1 => {
                        let entry = entries.remove(0);
                        Ok(whole(Moving::group(Group { panels: vec![entry.panel], active: 0, minimized: !entry.open, height: entry.height })))
                    }
                    _ => Err(DockError::MissingPanel),
                }
            }
            Place::Dock { path, column, group } => {
                let columns = self.columns_mut(&path).ok_or(DockError::MissingPanel)?;
                let source = columns.get_mut(column).ok_or(DockError::MissingPanel)?;
                if group >= source.groups.len() {
                    return Err(DockError::MissingPanel);
                }
                let taken = source.groups.remove(group);
                let width = source.width;
                if source.groups.is_empty() {
                    columns.remove(column);
                }
                if columns.is_empty() {
                    self.take_node(&path);
                }
                Ok(Moving { width: Some(width), ..Moving::group(taken) })
            }
        }
    }

    fn take_column(&mut self, panel: &P) -> Result<Moving<P>, DockError> {
        let Some(Place::Dock { path, column, .. }) = self.place(panel) else { return Err(DockError::InvalidPlacement) };
        let columns = self.columns_mut(&path).ok_or(DockError::MissingPanel)?;
        if column >= columns.len() {
            return Err(DockError::MissingPanel);
        }
        let taken = columns.remove(column);
        if columns.is_empty() {
            self.take_node(&path);
        }
        Ok(Moving {
            groups: taken.groups,
            node: None,
            minimized: false,
            width: Some(taken.width),
            iconic: taken.iconic,
            iconic_width: taken.iconic_width,
            rect: None,
            native: None,
            id: None,
        })
    }

    fn take_window(&mut self, panel: &P) -> Result<Moving<P>, DockError> {
        let Some(Place::Floating { window, .. }) = self.place(panel) else { return Err(DockError::InvalidPlacement) };
        if window >= self.floating.len() {
            return Err(DockError::MissingPanel);
        }
        let taken = self.floating.remove(window);
        Ok(Moving {
            groups: tree_groups(&taken.root),
            node: Some(taken.root),
            minimized: taken.minimized,
            width: taken.rect.get(2).copied(),
            iconic: false,
            iconic_width: None,
            rect: Some(taken.rect),
            native: taken.native,
            id: Some(taken.id),
        })
    }

    /// Insert groups taken by [`Self::take_unit`] (or a new single-panel group).
    /// `canvas` is the protected-panel test an outer edge drop narrows by ([`Self::edge_path`]).
    pub(super) fn insert_moving(&mut self, moving: Moving<P>, to: Target<P>, canvas: &dyn Fn(&P) -> bool) -> Result<(), DockError> {
        if moving.groups.is_empty() || moving.groups.iter().any(|g| g.panels.is_empty()) {
            return Err(DockError::MissingPanel);
        }
        match to {
            Target::Float { rect } => {
                let native = if rect.is_none() { moving.native } else { None };
                let rect = rect.or(moving.rect).unwrap_or_else(|| {
                    let [x, y, width, height] = DEFAULT_FLOAT;
                    [x, y, moving.width.unwrap_or(width), height]
                });
                if !valid_rect(rect) {
                    return Err(DockError::InvalidGeometry);
                }
                let root = match moving.node {
                    Some(node) => node,
                    None => stack_tree(moving.groups).ok_or(DockError::MissingPanel)?,
                };
                let window = Floating { native, minimized: moving.minimized, ..Floating::new(root, rect) };
                self.floating.push(Floating { id: moving.id.unwrap_or(window.id), ..window });
                Ok(())
            }
            Target::NewDock { side } => {
                if let Some(path) = self.dock_path(side) {
                    let columns = self.columns_mut(&path).ok_or(DockError::MissingTarget)?;
                    if !columns.is_empty() {
                        return Err(DockError::InvalidPlacement);
                    }
                    columns.push(moving.into_column(DEFAULT_COLUMN_WIDTH));
                    return Ok(());
                }
                let column = moving.into_column(DEFAULT_COLUMN_WIDTH);
                self.add_dock(side, column);
                Ok(())
            }
            Target::At { anchor, placement } => {
                let place = self.place(&anchor).ok_or(DockError::MissingTarget)?;
                match placement {
                    Placement::Tab { before } => {
                        let index = match &before {
                            Some(before) => {
                                if self.place(before).as_ref() != Some(&place) {
                                    return Err(DockError::MissingTarget);
                                }
                                self.index_in(&place, before)
                            }
                            None => None,
                        };
                        self.merge(&place, index, moving)
                    }
                    Placement::Split(Zone::Center) => {
                        let index = self.index_in(&place, &anchor).map(|i| i + 1);
                        self.merge(&place, index, moving)
                    }
                    Placement::Split(zone) => {
                        let (window, path) = place.tree_path().ok_or(DockError::InvalidPlacement)?;
                        let new = match moving.node {
                            Some(node) => node,
                            None => {
                                let (panels, active) = flatten(moving.groups);
                                Node::Tabs { panels, active }
                            }
                        };
                        let root = self.tree_mut(window).ok_or(DockError::MissingTarget)?;
                        if split_frame(root, path, new, zone) { Ok(()) } else { Err(DockError::MissingTarget) }
                    }
                    Placement::SplitSized { zone, size, outer } => {
                        let (window, path) = place.tree_path().ok_or(DockError::InvalidPlacement)?;
                        let new = match moving.node {
                            Some(node) => node,
                            None => {
                                let (panels, active) = flatten(moving.groups);
                                Node::Tabs { panels, active }
                            }
                        };
                        let root = self.tree_mut(window).ok_or(DockError::MissingTarget)?;
                        if split_frame_sized(root, path, new, zone, size, outer) { Ok(()) } else { Err(DockError::InvalidPlacement) }
                    }
                    // A floating window is a tree: a group above or below splits the anchor's group.
                    Placement::Group { below, .. } if matches!(place, Place::Floating { .. }) => {
                        let zone = if below { Zone::Bottom } else { Zone::Top };
                        self.insert_moving(moving, Target::At { anchor, placement: Placement::Split(zone) }, canvas)
                    }
                    Placement::Group { below, height } => {
                        if !valid_height(height) {
                            return Err(DockError::InvalidGeometry);
                        }
                        let mut moving = moving;
                        if height.is_some() {
                            moving.groups.iter_mut().for_each(|group| group.height = height);
                        }
                        let (groups, index) = match &place {
                            Place::Dock { group, .. } => (&mut self.column_mut(&place).ok_or(DockError::MissingTarget)?.groups, *group),
                            Place::Node(_) | Place::Floating { .. } => return Err(DockError::InvalidPlacement),
                        };
                        let index = (index + usize::from(below)).min(groups.len());
                        groups.splice(index..index, moving.groups);
                        Ok(())
                    }
                    Placement::Edge(zone) => {
                        if zone == Zone::Center || matches!(place, Place::Floating { .. }) {
                            return Err(DockError::InvalidPlacement);
                        }
                        // A protected anchor narrows the edge to the canvas column.
                        let path = if canvas(&anchor) { self.edge_path(zone, canvas) } else { self.edge_path(zone, &|_| false) };
                        let node = self.root.as_mut().and_then(|root| node_at_mut(root, &path)).ok_or(DockError::MissingTarget)?;
                        if matches!(node, Node::Dock { .. }) {
                            return Err(DockError::InvalidPlacement);
                        }
                        let (panels, active) = flatten(moving.groups);
                        let new = Node::Tabs { panels, active };
                        let old = std::mem::replace(node, Node::Tabs { panels: Vec::new(), active: 0 });
                        let axis = if matches!(zone, Zone::Top | Zone::Bottom) { SplitAxis::Vertical } else { SplitAxis::Horizontal };
                        let (first, second, size) = if matches!(zone, Zone::Top | Zone::Left) { (new, old, 0.25) } else { (old, new, 0.75) };
                        *node = Node::Split { axis, size: SplitSize::Ratio(size), first: Box::new(first), second: Box::new(second) };
                        Ok(())
                    }
                    Placement::Column { after } => {
                        let Place::Dock { path, column, .. } = &place else { return Err(DockError::InvalidPlacement) };
                        let columns = self.columns_mut(path).ok_or(DockError::MissingTarget)?;
                        let width = columns.get(*column).map_or(DEFAULT_COLUMN_WIDTH, |c| c.width);
                        let index = (column + usize::from(after)).min(columns.len());
                        columns.insert(index, moving.into_column(width));
                        Ok(())
                    }
                }
            }
        }
    }

    /// Wrap the root in a horizontal split with a new dock on `side`. The split's fixed size is
    /// the column width.
    pub(super) fn add_dock(&mut self, side: Side, column: Column<P>) {
        let width = column.width;
        let dock = Node::Dock { side, columns: vec![column] };
        self.root = Some(match self.root.take() {
            None => dock,
            Some(root) => {
                let (first, second, size) = match side {
                    Side::Start => (dock, root, SplitSize::FixedFirst(width)),
                    Side::End => (root, dock, SplitSize::FixedSecond(width)),
                };
                Node::Split { axis: SplitAxis::Horizontal, size, first: Box::new(first), second: Box::new(second) }
            }
        });
    }

    fn index_in(&self, place: &Place, panel: &P) -> Option<usize> {
        match place {
            Place::Node(_) | Place::Floating { .. } => match self.node_of(place)? {
                Node::Tabs { panels, .. } => panels.iter().position(|p| p == panel),
                Node::Stack { entries } => entries.iter().position(|e| &e.panel == panel),
                _ => None,
            },
            _ => self.group(place)?.panels.iter().position(|p| p == panel),
        }
    }

    /// Join every moving panel to the group at `place`, before `index` or at the end. The first
    /// moving group's active panel becomes active.
    fn merge(&mut self, place: &Place, index: Option<usize>, moving: Moving<P>) -> Result<(), DockError> {
        let (panels, selected) = flatten(moving.groups);
        match place {
            Place::Node(_) | Place::Floating { .. } => match self.node_of_mut(place) {
                Some(Node::Tabs { panels: target, active }) => {
                    splice(target, active, index.unwrap_or(target.len()), panels, selected);
                    Ok(())
                }
                Some(Node::Stack { entries }) => {
                    let index = index.unwrap_or(entries.len()).min(entries.len());
                    entries.splice(index..index, panels.into_iter().map(|panel| super::StackEntry { panel, open: true, height: None }));
                    Ok(())
                }
                _ => Err(DockError::MissingTarget),
            },
            _ => {
                let group = self.group_mut(place).ok_or(DockError::MissingTarget)?;
                let end = group.panels.len();
                splice(&mut group.panels, &mut group.active, index.unwrap_or(end), panels, selected);
                Ok(())
            }
        }
    }
}

fn leaf_holds<P: PartialEq>(leaf: &Node<P>, panel: &P) -> bool {
    match leaf {
        Node::Tabs { panels, .. } => panels.contains(panel),
        Node::Stack { entries } => entries.iter().any(|e| &e.panel == panel),
        Node::Split { .. } | Node::Dock { .. } => false,
    }
}

/// All panels in order, and the index of the first group's active panel among them.
fn flatten<P>(groups: Vec<Group<P>>) -> (Vec<P>, usize) {
    let selected = groups.first().map_or(0, |g| g.active.min(g.panels.len().saturating_sub(1)));
    (groups.into_iter().flat_map(|g| g.panels).collect(), selected)
}

pub(super) fn valid_column<P>(column: &Column<P>) -> bool {
    valid_width(column.width) && column.iconic_width.is_none_or(valid_width)
}
