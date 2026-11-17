//! Structural edits behind group, column and window moves, and the queries they share.
//!
//! A group, column or floating window is named by a panel it contains. Edits run on a proposed
//! copy of the layout, so a helper that fails part-way leaves nothing behind.

use std::hash::Hash;

use emath::Rect;

use crate::flow::Axis;

use super::{
    DockSide,
    action::DockPlacement,
    groups::{Column, DockEdge, Group, Moving, PanelPlace, Target, Unit},
    layout::Layout,
    limits::{MAX_DEPTH, MAX_NODES, MAX_PANELS, valid_height, valid_rect, valid_width},
    node::{Floating, Node, StackEntry},
    policy::DockError,
    split::SplitSize,
};

/// The node at `path` from `root`: false is a split's first child, true its second.
pub fn node_at<'a, P>(mut node: &'a Node<P>, path: &[bool]) -> Option<&'a Node<P>> {
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
pub(crate) fn leaves<P>(root: &Node<P>) -> Vec<(&Node<P>, Vec<bool>)> {
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
pub fn tree_panels<P>(root: &Node<P>) -> Vec<&P> {
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
pub fn first_active<P>(root: &Node<P>) -> Option<&P> {
    match leaves(root).into_iter().next()?.0 {
        Node::Tabs { panels, active } => panels.get(*active),
        Node::Stack { entries } => entries.first().map(|e| &e.panel),
        Node::Split { .. } | Node::Dock { .. } => None,
    }
}

/// Put `new` on the `zone` side of the frame at `path`: the frame splits into two halves and
/// every other frame keeps its size.
pub(super) fn split_frame<P>(root: &mut Node<P>, path: &[bool], new: Node<P>, zone: DockSide) -> bool {
    let axis = if matches!(zone, DockSide::Top | DockSide::Bottom) { Axis::Y } else { Axis::X };
    let new_first = matches!(zone, DockSide::Top | DockSide::Start);
    let Some(node) = node_at_mut(root, path) else { return false };
    let old = std::mem::replace(node, Node::Tabs { panels: Vec::new(), active: 0 });
    let (first, second) = if new_first { (new, old) } else { (old, new) };
    *node = Node::Split { axis, size: SplitSize::Ratio(0.5), first: Box::new(first), second: Box::new(second) };
    true
}

/// The depth along the frame `path` of the split whose size a [`DockPlacement::SplitSized`] on
/// `zone` replaces: the nearest enclosing split along the zone's axis (`on_axis` at a depth)
/// whose divider runs along the frame's `zone` side, else, for a frame at the edge of its
/// area on that side, the nearest one along its opposite side.
pub(super) fn sized_split_depth(path: &[bool], zone: DockSide, on_axis: impl Fn(usize) -> bool) -> Option<usize> {
    // A divider on the frame's start side (start or top) belongs to a split holding the frame
    // in its second child.
    let start = matches!(zone, DockSide::Start | DockSide::Top);
    let nearest = |second: bool| (0..path.len()).rev().find(|&depth| path.get(depth) == Some(&second) && on_axis(depth));
    nearest(start).or_else(|| nearest(!start))
}

/// [`split_frame`] with the sizes of [`DockPlacement::SplitSized`]: the new split takes `size`, and
/// the split [`sized_split_depth`] finds takes `outer`. False when there is no such split, or a
/// dock is beyond it.
pub(super) fn split_frame_sized<P>(root: &mut Node<P>, path: &[bool], new: Node<P>, zone: DockSide, size: SplitSize, outer: SplitSize) -> bool {
    let axis = if matches!(zone, DockSide::Top | DockSide::Bottom) { Axis::Y } else { Axis::X };
    let enclosing =
        sized_split_depth(path, zone, |depth| matches!(node_at(root, &path[..depth]), Some(Node::Split { axis: own, .. }) if *own == axis));
    let Some(depth) = enclosing else { return false };
    let Some(Node::Split { size: own, first, second, .. }) = node_at_mut(root, &path[..depth]) else { return false };
    if zone == DockSide::Center || matches!(**first, Node::Dock { .. }) || matches!(**second, Node::Dock { .. }) {
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
pub(super) fn splice<P>(panels: &mut Vec<P>, active: &mut usize, index: usize, new: Vec<P>, selected: usize) {
    let index = index.min(panels.len());
    let count = new.len();
    panels.splice(index..index, new);
    *active = index + selected.min(count.saturating_sub(1));
}

/// Queries the renderer reads: where a panel is, the node, column or group at a place, the dock and edge
/// geometry a drop needs, and whether two layouts arrange the panels alike.
impl<P: Clone + Eq + Hash> Layout<P> {
    /// Whether `other` arranges the panels as this layout does: equal apart from split sizes,
    /// column widths, group heights and window rectangles. A drop that leads from one to the
    /// other changes nothing the user would see as a move.
    pub fn same_arrangement(&self, other: &Self) -> bool {
        self.shape() == other.shape()
    }

    /// The place of `panel`, or `None` when it is absent.
    pub fn place(&self, panel: &P) -> Option<PanelPlace> {
        for (w, window) in self.floating.iter().enumerate() {
            if let Some(g) = window.column.groups.iter().position(|group| group.panels.contains(panel)) {
                return Some(PanelPlace::Floating { window: w, group: g });
            }
        }
        for (leaf, path) in self.root.as_ref().map(leaves).unwrap_or_default() {
            match leaf {
                Node::Dock { columns, .. } => {
                    for (c, column) in columns.iter().enumerate() {
                        if let Some(g) = column.groups.iter().position(|group| group.panels.contains(panel)) {
                            return Some(PanelPlace::Dock { path, column: c, group: g });
                        }
                    }
                }
                leaf if leaf_holds(leaf, panel) => return Some(PanelPlace::Node(path)),
                _ => {}
            }
        }
        None
    }

    /// The `Tabs` or `Stack` node at a place of the docked tree.
    pub fn node_of(&self, place: &PanelPlace) -> Option<&Node<P>> {
        node_at(self.root.as_ref()?, place.tree_path()?)
    }

    /// The path of the `Dock` node on `side`, when the layout has one.
    pub fn dock_path(&self, side: DockEdge) -> Option<Vec<bool>> {
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
    pub fn edge_path(&self, zone: DockSide, canvas: &dyn Fn(&P) -> bool) -> Vec<bool> {
        let holds = |node: &Node<P>| tree_panels(node).into_iter().any(canvas);
        let mut path = Vec::new();
        let mut node = self.root.as_ref();
        while let Some(Node::Split { axis, first, second, .. }) = node {
            if path.len() > MAX_DEPTH {
                break;
            }
            let second_side = match (axis, &**first, &**second) {
                (Axis::X, Node::Dock { .. }, _) => true,
                (Axis::X, _, Node::Dock { .. }) => false,
                (Axis::Y, ..) => match (zone, holds(first), holds(second)) {
                    (DockSide::Top, true, false) => false,
                    (DockSide::Bottom, false, true) => true,
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
    pub fn edge_narrows(&self, zone: DockSide, canvas: &dyn Fn(&P) -> bool) -> bool {
        self.edge_path(zone, canvas) != self.edge_path(zone, &|_| false)
    }

    /// The docked panel, other than `skip`, that a [`DockPlacement::Edge`] anchors on so the drop
    /// wraps the edge area it shows: a `protected` one where that area is narrowed to the
    /// canvas column (`narrowed`), else one that is not protected, since a protected anchor
    /// narrows the model's edge. Any docked panel when every one is protected.
    pub fn edge_anchor(&self, narrowed: bool, protected: &dyn Fn(&P) -> bool, skip: &dyn Fn(&P) -> bool) -> Option<P> {
        let docked: Vec<&P> =
            self.panels().into_iter().filter(|p| !skip(p) && matches!(self.place(p), Some(PanelPlace::Dock { .. } | PanelPlace::Node(_)))).collect();
        docked.iter().find(|p| protected(p) == narrowed).or(docked.first()).map(|p| (*p).clone())
    }

    /// The column at `place`: a dock column or a floating window's column; `None` for a node of the docked tree.
    pub fn column(&self, place: &PanelPlace) -> Option<&Column<P>> {
        match place {
            PanelPlace::Dock { path, column, .. } => match self.root.as_ref().and_then(|root| node_at(root, path)) {
                Some(Node::Dock { columns, .. }) => columns.get(*column),
                _ => None,
            },
            PanelPlace::Floating { window, .. } => self.floating.get(*window).map(|w| &w.column),
            PanelPlace::Node(_) => None,
        }
    }

    /// The group at `place` of a dock column or a floating window; `None` for a `Tabs` or `Stack` node.
    pub fn group(&self, place: &PanelPlace) -> Option<&Group<P>> {
        self.column(place)?.groups.get(place.group_index()?)
    }

    /// Whether opening `panel` shows its column's flyout instead of the column: its column is collapsed to icons.
    pub fn opens_in_flyout(&self, panel: &P) -> bool {
        self.place(panel).is_some_and(|place| self.column(&place).is_some_and(|column| column.iconic))
    }
}

/// A column without its sizes: its groups and their state, and whether it is collapsed.
fn column_shape<P: Clone>(column: &Column<P>) -> Column<P> {
    Column {
        groups: column.groups.iter().map(|g| Group { height: None, flyout_width: None, flyout_offset: None, ..g.clone() }).collect(),
        width: None,
        iconic: column.iconic,
        iconic_width: None,
        steps: None,
    }
}

impl<P: Clone + Eq + Hash> Layout<P> {
    /// The layout without its sizes and rectangles: which panels are where, and their state.
    /// A run of splits along one axis nests one way (`a | (b | c)`), since nesting it the
    /// other way only moves sizes between its members.
    fn shape(&self) -> Layout<P> {
        fn run<'a, P>(node: &'a Node<P>, axis: Axis, out: &mut Vec<&'a Node<P>>) {
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
                Node::Stack { entries } => Node::Stack { entries: entries.iter().map(|e| StackEntry { height: None, ..e.clone() }).collect() },
                Node::Dock { side, columns } => Node::Dock { side: *side, columns: columns.iter().map(column_shape).collect() },
            }
        }
        Layout {
            root: self.root.as_ref().map(node),
            floating: self
                .floating
                .iter()
                .map(|w| Floating { column: column_shape(&w.column), rect: Rect::ZERO, native: None, edge: None, ..w.clone() })
                .collect(),
        }
    }

    pub(super) fn node_of_mut(&mut self, place: &PanelPlace) -> Option<&mut Node<P>> {
        node_at_mut(self.root.as_mut()?, place.tree_path()?)
    }

    pub(super) fn columns_mut(&mut self, path: &[bool]) -> Option<&mut Vec<Column<P>>> {
        match self.root.as_mut().and_then(|root| node_at_mut(root, path)) {
            Some(Node::Dock { columns, .. }) => Some(columns),
            _ => None,
        }
    }

    pub(super) fn column_mut(&mut self, place: &PanelPlace) -> Option<&mut Column<P>> {
        match place {
            PanelPlace::Dock { path, column, .. } => self.columns_mut(path)?.get_mut(*column),
            PanelPlace::Floating { window, .. } => self.floating.get_mut(*window).map(|w| &mut w.column),
            PanelPlace::Node(_) => None,
        }
    }

    pub(super) fn group_mut(&mut self, place: &PanelPlace) -> Option<&mut Group<P>> {
        let group = place.group_index()?;
        self.column_mut(place)?.groups.get_mut(group)
    }

    /// The state a new column made of groups taken from the column at `place` keeps ([`Column::emptied`]), with a
    /// floating window's width; no state for a place outside a column.
    pub(super) fn source_column(&self, place: &PanelPlace) -> Column<P> {
        let mut state = self.column(place).map_or_else(|| Column::new(Vec::new()), Column::emptied);
        if let PanelPlace::Floating { window, .. } = place
            && let Some(window) = self.floating.get(*window)
        {
            state.width = Some(window.rect.width());
        }
        state
    }

    /// Removes the column at `place` when it holds no group: a dock column (and its dock when that empties too), or
    /// a floating window. Returns the id of a window it removed.
    fn drop_if_empty(&mut self, place: &PanelPlace) -> Option<u64> {
        if !self.column(place).is_some_and(|column| column.groups.is_empty()) {
            return None;
        }
        match place {
            PanelPlace::Dock { path, column, .. } => {
                let columns = self.columns_mut(path)?;
                columns.remove(*column);
                if columns.is_empty() {
                    self.take_node(path);
                }
                None
            }
            PanelPlace::Floating { window, .. } => Some(self.floating.remove(*window).id),
            PanelPlace::Node(_) => None,
        }
    }

    /// Remove the node at `path` of the docked layout; its parent split collapses into the
    /// sibling.
    pub(super) fn take_node(&mut self, path: &[bool]) -> Option<Node<P>> {
        let Some((second_child, parent_path)) = path.split_last() else { return self.root.take() };
        let parent = self.root.as_mut().and_then(|root| node_at_mut(root, parent_path))?;
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
            (Unit::Group, PanelPlace::Node(_)) => match self.node_of(&place) {
                Some(Node::Tabs { panels, .. }) => Ok(panels.clone()),
                Some(Node::Stack { .. }) => Ok(vec![panel.clone()]),
                _ => Err(DockError::MissingPanel),
            },
            (Unit::Group, _) => self.group(&place).map(|g| g.panels.clone()).ok_or(DockError::MissingPanel),
            (Unit::Column, PanelPlace::Dock { .. } | PanelPlace::Floating { .. }) | (Unit::Window, PanelPlace::Floating { .. }) => {
                Ok(self.column(&place).ok_or(DockError::MissingPanel)?.panels().cloned().collect())
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

    /// Detaches the panel's group with the state of the column it leaves; a floating window's last group carries
    /// the window's id. A stack entry becomes a single-panel group with its saved height.
    pub(super) fn take_group(&mut self, panel: &P) -> Result<Moving<P>, DockError> {
        let place = self.place(panel).ok_or(DockError::MissingPanel)?;
        let PanelPlace::Node(path) = &place else {
            let source = self.source_column(&place);
            let group = place.group_index().ok_or(DockError::MissingPanel)?;
            let column = self.column_mut(&place).ok_or(DockError::MissingPanel)?;
            if group >= column.groups.len() {
                return Err(DockError::MissingPanel);
            }
            let taken = column.groups.remove(group);
            let id = self.drop_if_empty(&place);
            return Ok(Moving { id, ..Moving::taken(taken, source) });
        };
        if let Some(Node::Stack { entries }) = self.node_of_mut(&place)
            && entries.len() > 1
            && let Some(index) = entries.iter().position(|e| &e.panel == panel)
        {
            return Ok(Moving::group(entries.remove(index).into_group()));
        }
        match self.take_node(path) {
            Some(Node::Tabs { panels, active }) => Ok(Moving::group(Group::new(panels, active))),
            Some(Node::Stack { mut entries }) if entries.len() == 1 => Ok(Moving::group(entries.remove(0).into_group())),
            _ => Err(DockError::MissingPanel),
        }
    }

    /// Detaches the panel's column: a dock column, or a floating window's, which takes the window's width and id
    /// along.
    pub(super) fn take_column(&mut self, panel: &P) -> Result<Moving<P>, DockError> {
        match self.place(panel) {
            Some(PanelPlace::Dock { path, column, .. }) => {
                let columns = self.columns_mut(&path).ok_or(DockError::MissingPanel)?;
                if column >= columns.len() {
                    return Err(DockError::MissingPanel);
                }
                let taken = columns.remove(column);
                if columns.is_empty() {
                    self.take_node(&path);
                }
                Ok(Moving { column: taken, id: None })
            }
            Some(PanelPlace::Floating { window, .. }) => {
                if window >= self.floating.len() {
                    return Err(DockError::MissingPanel);
                }
                let taken = self.floating.remove(window);
                Ok(Moving { column: Column { width: Some(taken.rect.width()), ..taken.column }, id: Some(taken.id) })
            }
            Some(PanelPlace::Node(_)) | None => Err(DockError::InvalidPlacement),
        }
    }

    /// Detaches the floating window holding the panel: its column.
    pub(super) fn take_window(&mut self, panel: &P) -> Result<Moving<P>, DockError> {
        match self.place(panel) {
            Some(PanelPlace::Floating { .. }) => self.take_column(panel),
            _ => Err(DockError::InvalidPlacement),
        }
    }

    /// Insert groups taken by [`Self::take_unit`] (or a new single-panel group). A new column, docked or floating,
    /// keeps the moving column's state; groups that join a column take that column's.
    /// `canvas` is the protected-panel test an outer edge drop narrows by ([`Self::edge_path`]).
    pub(super) fn insert_moving(&mut self, moving: Moving<P>, to: Target<P>, canvas: &dyn Fn(&P) -> bool) -> Result<(), DockError> {
        let mut moving = moving;
        if moving.column.groups.is_empty() || moving.column.groups.iter().any(|g| g.panels.is_empty()) {
            return Err(DockError::MissingPanel);
        }
        match to {
            Target::Float { rect, edge } => {
                if !valid_rect(rect) {
                    return Err(DockError::InvalidGeometry);
                }
                // A window's width is its rect's; only a collapsed column keeps to an edge of its own.
                let window = Floating::new(Column { width: None, ..moving.column }, rect);
                let edge = edge.filter(|_| window.column.iconic);
                self.floating.push(Floating { id: moving.id.unwrap_or(window.id), edge, ..window });
                Ok(())
            }
            Target::NewDock { side } => {
                if let Some(path) = self.dock_path(side) {
                    let columns = self.columns_mut(&path).ok_or(DockError::MissingTarget)?;
                    if !columns.is_empty() {
                        return Err(DockError::InvalidPlacement);
                    }
                    columns.push(moving.into_column(None));
                    return Ok(());
                }
                let column = moving.into_column(None);
                self.add_dock(side, column);
                Ok(())
            }
            Target::At { anchor, placement } => {
                let place = self.place(&anchor).ok_or(DockError::MissingTarget)?;
                match placement {
                    DockPlacement::Tab { before } => {
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
                    DockPlacement::Split(DockSide::Center) => {
                        let index = self.index_in(&place, &anchor).map(|i| i + 1);
                        self.merge(&place, index, moving)
                    }
                    DockPlacement::Split(zone) => {
                        let path = place.tree_path().ok_or(DockError::InvalidPlacement)?;
                        let (panels, active) = flatten(moving.column.groups);
                        let root = self.root.as_mut().ok_or(DockError::MissingTarget)?;
                        if split_frame(root, path, Node::Tabs { panels, active }, zone) { Ok(()) } else { Err(DockError::MissingTarget) }
                    }
                    DockPlacement::SplitSized { zone, size, outer } => {
                        let path = place.tree_path().ok_or(DockError::InvalidPlacement)?;
                        let (panels, active) = flatten(moving.column.groups);
                        let root = self.root.as_mut().ok_or(DockError::MissingTarget)?;
                        if split_frame_sized(root, path, Node::Tabs { panels, active }, zone, size, outer) {
                            Ok(())
                        } else {
                            Err(DockError::InvalidPlacement)
                        }
                    }
                    DockPlacement::Group { below, height } => {
                        if !valid_height(height) {
                            return Err(DockError::InvalidGeometry);
                        }
                        if height.is_some() {
                            moving.column.groups.iter_mut().for_each(|group| group.height = height);
                        }
                        let index = place.group_index().ok_or(DockError::InvalidPlacement)?;
                        let groups = &mut self.column_mut(&place).ok_or(DockError::MissingTarget)?.groups;
                        let index = (index + usize::from(below)).min(groups.len());
                        groups.splice(index..index, moving.column.groups);
                        Ok(())
                    }
                    DockPlacement::Edge(zone) => {
                        if zone == DockSide::Center || matches!(place, PanelPlace::Floating { .. }) {
                            return Err(DockError::InvalidPlacement);
                        }
                        // A protected anchor narrows the edge to the canvas column.
                        let path = if canvas(&anchor) { self.edge_path(zone, canvas) } else { self.edge_path(zone, &|_| false) };
                        let node = self.root.as_mut().and_then(|root| node_at_mut(root, &path)).ok_or(DockError::MissingTarget)?;
                        if matches!(node, Node::Dock { .. }) {
                            return Err(DockError::InvalidPlacement);
                        }
                        let (panels, active) = flatten(moving.column.groups);
                        let new = Node::Tabs { panels, active };
                        let old = std::mem::replace(node, Node::Tabs { panels: Vec::new(), active: 0 });
                        let axis = if matches!(zone, DockSide::Top | DockSide::Bottom) { Axis::Y } else { Axis::X };
                        let (first, second, size) = if matches!(zone, DockSide::Top | DockSide::Start) { (new, old, 0.25) } else { (old, new, 0.75) };
                        *node = Node::Split { axis, size: SplitSize::Ratio(size), first: Box::new(first), second: Box::new(second) };
                        Ok(())
                    }
                    DockPlacement::Column { after } => {
                        let PanelPlace::Dock { path, column, .. } = &place else { return Err(DockError::InvalidPlacement) };
                        let columns = self.columns_mut(path).ok_or(DockError::MissingTarget)?;
                        let width = columns.get(*column).and_then(|c| c.width);
                        let index = (column + usize::from(after)).min(columns.len());
                        columns.insert(index, moving.into_column(width));
                        Ok(())
                    }
                }
            }
        }
    }

    /// Wrap the root in a horizontal split with a new dock on `side`. A dock is laid out from its
    /// columns' widths, so the split's size is a placeholder.
    pub(super) fn add_dock(&mut self, side: DockEdge, column: Column<P>) {
        let dock = Node::Dock { side, columns: vec![column] };
        self.root = Some(match self.root.take() {
            None => dock,
            Some(root) => {
                let (first, second, size) = match side {
                    DockEdge::Start => (dock, root, SplitSize::Ratio(0.25)),
                    DockEdge::End => (root, dock, SplitSize::Ratio(0.75)),
                };
                Node::Split { axis: Axis::X, size, first: Box::new(first), second: Box::new(second) }
            }
        });
    }

    pub(super) fn index_in(&self, place: &PanelPlace, panel: &P) -> Option<usize> {
        match place {
            PanelPlace::Node(_) => match self.node_of(place)? {
                Node::Tabs { panels, .. } => panels.iter().position(|p| p == panel),
                Node::Stack { entries } => entries.iter().position(|e| &e.panel == panel),
                _ => None,
            },
            _ => self.group(place)?.panels.iter().position(|p| p == panel),
        }
    }

    /// Join every moving panel to the group at `place`, before `index` or at the end. The first
    /// moving group's active panel becomes active.
    pub(super) fn merge(&mut self, place: &PanelPlace, index: Option<usize>, moving: Moving<P>) -> Result<(), DockError> {
        let (panels, selected) = flatten(moving.column.groups);
        match place {
            PanelPlace::Node(_) => match self.node_of_mut(place) {
                Some(Node::Tabs { panels: target, active }) => {
                    splice(target, active, index.unwrap_or(target.len()), panels, selected);
                    Ok(())
                }
                Some(Node::Stack { entries }) => {
                    let index = index.unwrap_or(entries.len()).min(entries.len());
                    entries.splice(index..index, panels.into_iter().map(|panel| StackEntry { panel, open: true, height: None }));
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

pub(super) fn leaf_holds<P: PartialEq>(leaf: &Node<P>, panel: &P) -> bool {
    match leaf {
        Node::Tabs { panels, .. } => panels.contains(panel),
        Node::Stack { entries } => entries.iter().any(|e| &e.panel == panel),
        Node::Split { .. } | Node::Dock { .. } => false,
    }
}

/// All panels in order, and the index of the first group's active panel among them.
pub(super) fn flatten<P>(groups: Vec<Group<P>>) -> (Vec<P>, usize) {
    let selected = groups.first().map_or(0, |g| g.active.min(g.panels.len().saturating_sub(1)));
    (groups.into_iter().flat_map(|g| g.panels).collect(), selected)
}

pub(super) fn valid_column<P>(column: &Column<P>) -> bool {
    column.width.is_none_or(valid_width) && column.iconic_width.is_none_or(valid_width) && column.steps != Some(0)
}
