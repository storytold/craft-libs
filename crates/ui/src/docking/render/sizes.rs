//! Split-tree geometry: where each split draws its children, how far a divider can travel,
//! and the sizes that move a divider.
//!
//! A dragged divider grows the groups touching it on one side, up to their maximums, and
//! shrinks the groups on the other side in turn: the group touching the divider gives way
//! down to its minimum, then the next group along the axis, and so on, until every group on
//! that side is at its minimum. Groups side by side across the axis shrink together, and the
//! line goes on past them once one is at its minimum. The line follows splits of the drag's
//! axis up the tree and ends at a split across it or beside a dock. Every other group keeps
//! its size.

use egui::Rect;

use super::{dock::dock_rects, node_limits, split_bounds};
use crate::docking::{DockContent, DockStyle, Node, PanelLimits};
use crate::layout::{SplitAxis, SplitLayout, SplitSize, split_rect};

/// A split's two children with the rects they are drawn in.
type Children<'n, P> = [(&'n Node<P>, Rect); 2];

/// How far each divider beside a group that a drop makes smaller can still travel both ways:
/// the group keeps this much above its minimum, or the groups beyond it give the room.
pub(in crate::docking) const DROP_MARGIN: f32 = 1.0;

pub(in crate::docking) struct Sizer<'a, P> {
    pub(in crate::docking) style: &'a DockStyle,
    pub(in crate::docking) content: &'a dyn DockContent<P>,
}

/// One group as drawn: its node, rect and limits.
pub(in crate::docking) struct Drawn<'a, P> {
    pub(in crate::docking) node: &'a Node<P>,
    pub(in crate::docking) rect: Rect,
    pub(in crate::docking) limits: PanelLimits,
}

fn extent(rect: Rect, axis: SplitAxis) -> f32 {
    match axis {
        SplitAxis::Horizontal => rect.width(),
        SplitAxis::Vertical => rect.height(),
    }
}

fn along(v: egui::Vec2, axis: SplitAxis) -> f32 {
    match axis {
        SplitAxis::Horizontal => v.x,
        SplitAxis::Vertical => v.y,
    }
}

impl<P> Sizer<'_, P> {
    fn limits(&self, node: &Node<P>) -> PanelLimits {
        node_limits(node, self.style, self.content)
    }

    /// The split as drawn: its stored size, clamped so both children fit their limits.
    pub(in crate::docking) fn split(&self, axis: SplitAxis, size: SplitSize, first: &Node<P>, second: &Node<P>, rect: Rect) -> Option<SplitLayout> {
        let wanted = split_rect(rect, axis, size, self.style.gap, 0.0)?;
        let (lo, hi) = split_bounds(wanted.available(), axis, self.limits(first), self.limits(second));
        split_rect(rect, axis, SplitSize::FixedFirst(wanted.first_size().clamp(lo, hi)), self.style.gap, 0.0)
    }

    /// The children of a split drawn in `rect`, with their rects; a dock beside its neighbour
    /// is drawn by the dock renderer.
    pub(in crate::docking) fn children<'n>(&self, node: &'n Node<P>, rect: Rect) -> Option<[(&'n Node<P>, Rect); 2]> {
        let Node::Split { axis, size, first, second } = node else { return None };
        match (&**first, &**second) {
            (Node::Dock { side, columns }, other) if *axis == SplitAxis::Horizontal => {
                let (dock, rest, _) = dock_rects(*side, columns, other, rect, self.style, self.content);
                Some([(first, dock), (second, rest)])
            }
            (other, Node::Dock { side, columns }) if *axis == SplitAxis::Horizontal => {
                let (dock, rest, _) = dock_rects(*side, columns, other, rect, self.style, self.content);
                Some([(first, rest), (second, dock)])
            }
            _ => {
                let split = self.split(*axis, *size, first, second, rect)?;
                Some([(first, split.first), (second, split.second)])
            }
        }
    }

    /// Every group of `node` drawn in `rect`; a dock counts as one.
    pub(in crate::docking) fn groups<'n>(&self, node: &'n Node<P>, rect: Rect, out: &mut Vec<Drawn<'n, P>>) {
        match self.children(node, rect) {
            Some(children) => children.into_iter().for_each(|(child, rect)| self.groups(child, rect, out)),
            None => out.push(Drawn { node, rect, limits: self.limits(node) }),
        }
    }

    /// How far the end edge (`end`) or start edge of `node` drawn in `rect` can move along
    /// `axis`: inward until every group along the axis from that edge is at its minimum, and
    /// outward while the groups touching it stay below their maximums. Groups side by side
    /// across the axis move together, so the first of them to reach its limit stops them.
    pub(in crate::docking) fn travel(&self, node: &Node<P>, rect: Rect, axis: SplitAxis, end: bool) -> (f32, f32) {
        let docked =
            matches!(node, Node::Split { first, second, .. } if matches!(**first, Node::Dock { .. }) || matches!(**second, Node::Dock { .. }));
        match (node, self.children(node, rect)) {
            (Node::Split { first, .. }, Some([a, b])) if docked => {
                let (other, rect) = if matches!(**first, Node::Dock { .. }) { b } else { a };
                self.travel(other, rect, axis, end)
            }
            (Node::Split { axis: split_axis, .. }, Some([a, b])) if *split_axis == axis => {
                let (near, far) = if end { (b, a) } else { (a, b) };
                let (near, far) = (self.travel(near.0, near.1, axis, end), self.travel(far.0, far.1, axis, end));
                (near.0 + far.0, near.1)
            }
            (_, Some([a, b])) => {
                let (a, b) = (self.travel(a.0, a.1, axis, end), self.travel(b.0, b.1, axis, end));
                (a.0.min(b.0), a.1.min(b.1))
            }
            (_, None) => {
                let limits = self.limits(node);
                let size = extent(rect, axis);
                ((size - along(limits.min, axis)).max(0.0), (along(limits.max, axis) - size).max(0.0))
            }
        }
    }

    /// The sizes that move the end (`end`) or start edge of `node` drawn in `rect` outward by
    /// `grow` points. Growing, only the groups touching that edge change. Shrinking, the group
    /// at the edge gives way down to its minimum, then the next one along the axis, and so on.
    /// Paths are relative to `path`.
    pub(in crate::docking) fn follow(
        &self,
        (node, rect): (&Node<P>, Rect),
        axis: SplitAxis,
        end: bool,
        grow: f32,
        path: &mut Vec<bool>,
        out: &mut Vec<(Vec<bool>, SplitSize)>,
    ) {
        let Some([a, b]) = self.children(node, rect) else { return };
        let Node::Split { axis: split_axis, size, first, second } = node else { return };
        let docked = matches!(**first, Node::Dock { .. }) || matches!(**second, Node::Dock { .. });
        let visit = |child: (&Node<P>, Rect), second_child: bool, grow: f32, path: &mut Vec<bool>, out: &mut Vec<(Vec<bool>, SplitSize)>| {
            path.push(second_child);
            self.follow(child, axis, end, grow, path, out);
            path.pop();
        };
        if docked {
            let dock_first = matches!(**first, Node::Dock { .. });
            visit(if dock_first { b } else { a }, dock_first, grow, path, out);
        } else if *split_axis == axis {
            let (near, far) = if end { (b, a) } else { (a, b) };
            let near_grow = if grow < 0.0 { grow.max(-self.travel(near.0, near.1, axis, end).0) } else { grow };
            let far_grow = grow - near_grow;
            let (near_size, far_size) = (extent(near.1, axis) + near_grow, extent(far.1, axis) + far_grow);
            let first_size = if end { far_size } else { near_size };
            push_size(path, *size, first_size, near_size + far_size, out);
            visit(near, end, near_grow, path, out);
            if far_grow != 0.0 {
                visit(far, !end, far_grow, path, out);
            }
        } else {
            visit(a, false, grow, path, out);
            visit(b, true, grow, path, out);
        }
    }

    /// The drawn splits from `root` down to the one at `path`, each with its rect.
    fn chain<'n>(&self, root: (&'n Node<P>, Rect), path: &[bool]) -> Option<Vec<(&'n Node<P>, Rect)>> {
        let mut chain = vec![root];
        for &second in path {
            let (node, rect) = *chain.last()?;
            let [a, b] = self.children(node, rect)?;
            chain.push(if second { b } else { a });
        }
        Some(chain)
    }

    /// The ancestors that continue the line of the split at the end of `chain` along `axis`:
    /// each with its children and whether the line runs through its second child, nearest first.
    /// The line ends at a split across the axis or beside a dock.
    fn line<'n>(&self, chain: &[(&'n Node<P>, Rect)], path: &[bool], axis: SplitAxis) -> Vec<(usize, Children<'n, P>, bool)> {
        let mut line = Vec::new();
        for depth in (0..path.len()).rev() {
            let Some(&(node, rect)) = chain.get(depth) else { break };
            let Node::Split { axis: split_axis, first, second, .. } = node else { break };
            if *split_axis != axis || matches!(**first, Node::Dock { .. }) || matches!(**second, Node::Dock { .. }) {
                break;
            }
            let (Some(children), Some(&second_child)) = (self.children(node, rect), path.get(depth)) else { break };
            line.push((depth, children, second_child));
        }
        line
    }

    /// The divider of the split at `path` under `root` (a node and the rect it is drawn in)
    /// moved by `delta` points, clamped to its travel. The groups touching the divider on the
    /// side it moves toward give way, then the groups beyond them along the axis, one after
    /// another, as far as the line of same-axis splits reaches; the groups on the other side
    /// that touch the divider take the room. Paths are absolute, like `path`.
    pub(in crate::docking) fn drag(&self, root: (&Node<P>, Rect), path: &[bool], delta: f32) -> Vec<(Vec<bool>, SplitSize)> {
        let Some(chain) = self.chain(root, path) else { return Vec::new() };
        let Some(&(node, rect)) = chain.last() else { return Vec::new() };
        let Node::Split { axis, size, first, second } = node else { return Vec::new() };
        let axis = *axis;
        let Some(split) = self.split(axis, *size, first, second, rect) else { return Vec::new() };
        let (lo, hi) = self.range(root, path);
        let delta = delta.clamp(lo, hi);
        if delta == 0.0 || !delta.is_finite() || split.available() <= 0.0 {
            return Vec::new();
        }
        // Toward the end edge the second child gives way, toward the start the first.
        let forward = delta > 0.0;
        let moved = delta.abs();
        let given = if forward {
            moved.min(self.travel(second, split.second, axis, false).0)
        } else {
            moved.min(self.travel(first, split.first, axis, true).0)
        };
        let (first_grow, second_grow) = if forward { (moved, -given) } else { (-given, moved) };
        let mut out = Vec::new();
        let mut path = path.to_vec();
        let first_size = split.first_size() + first_grow;
        push_size(&path, *size, first_size, first_size + extent(split.second, axis) + second_grow, &mut out);
        path.push(false);
        self.follow((first, split.first), axis, true, first_grow, &mut path, &mut out);
        path.pop();
        path.push(true);
        self.follow((second, split.second), axis, false, second_grow, &mut path, &mut out);
        path.pop();
        // What the split's own child could not give, the ancestors' children beyond it give:
        // the split grows by `rest` toward them, and so does each ancestor until one absorbs it.
        let mut rest = moved - given;
        for (depth, [a, b], second_child) in self.line(&chain, &path, axis) {
            if rest <= 0.0 {
                break;
            }
            path.truncate(depth);
            let (a_size, b_size) = (extent(a.1, axis), extent(b.1, axis));
            let (first_size, second_size) = match (forward, second_child) {
                (true, false) => {
                    let taken = rest.min(self.travel(b.0, b.1, axis, false).0);
                    path.push(true);
                    self.follow(b, axis, false, -taken, &mut path, &mut out);
                    path.pop();
                    let sizes = (a_size + rest, b_size - taken);
                    rest -= taken;
                    sizes
                }
                (false, true) => {
                    let taken = rest.min(self.travel(a.0, a.1, axis, true).0);
                    path.push(false);
                    self.follow(a, axis, true, -taken, &mut path, &mut out);
                    path.pop();
                    let sizes = (a_size - taken, b_size + rest);
                    rest -= taken;
                    sizes
                }
                (true, true) => (a_size, b_size + rest),
                (false, false) => (a_size + rest, b_size),
            };
            if let Some((Node::Split { size, .. }, _)) = chain.get(depth) {
                push_size(&path, *size, first_size, first_size + second_size, &mut out);
            }
        }
        out
    }

    /// How far the divider of the split at `path` under `root` can move: `(lo, hi)`, with
    /// `lo <= 0 <= hi`. Each way it is the lesser of the room the groups touching the divider
    /// have to grow and the room every group beyond the divider, along the line of same-axis
    /// splits, has to shrink.
    pub(in crate::docking) fn range(&self, root: (&Node<P>, Rect), path: &[bool]) -> (f32, f32) {
        let Some(chain) = self.chain(root, path) else { return (0.0, 0.0) };
        let Some(&(node, rect)) = chain.last() else { return (0.0, 0.0) };
        let Node::Split { axis, size, first, second } = node else { return (0.0, 0.0) };
        let Some(split) = self.split(*axis, *size, first, second, rect) else { return (0.0, 0.0) };
        let (mut first_in, first_out) = self.travel(first, split.first, *axis, true);
        let (mut second_in, second_out) = self.travel(second, split.second, *axis, false);
        for (_, [a, b], second_child) in self.line(&chain, path, *axis) {
            if second_child {
                first_in += self.travel(a.0, a.1, *axis, true).0;
            } else {
                second_in += self.travel(b.0, b.1, *axis, false).0;
            }
        }
        (-first_in.min(second_out), first_out.min(second_in))
    }

    /// Whether every divider of `root` across `axis` that runs along a side of the group drawn
    /// at `group` can travel [`DROP_MARGIN`] both ways, by [`Self::range`]: a group at its
    /// minimum holds no divider still while the groups beyond it can give way.
    pub(in crate::docking) fn frees(&self, root: (&Node<P>, Rect), group: Rect, axis: SplitAxis) -> bool {
        let edges = |rect: Rect| match axis {
            SplitAxis::Horizontal => (rect.left(), rect.right()),
            SplitAxis::Vertical => (rect.top(), rect.bottom()),
        };
        let (start, end) = edges(group);
        let beside = |child: Rect, edge: f32, own: f32| child.contains(group.center()) && (edge - own).abs() < 0.5;
        let mut nodes = vec![(root.0, root.1, Vec::new())];
        while let Some((node, rect, path)) = nodes.pop() {
            let (Node::Split { axis: split_axis, .. }, Some([a, b])) = (node, self.children(node, rect)) else { continue };
            if *split_axis == axis && (beside(a.1, edges(a.1).1, end) || beside(b.1, edges(b.1).0, start)) {
                let (lo, hi) = self.range(root, &path);
                if -lo < DROP_MARGIN || hi < DROP_MARGIN {
                    return false;
                }
            }
            for (second, (child, rect)) in [(false, a), (true, b)] {
                let mut path = path.clone();
                path.push(second);
                nodes.push((child, rect, path));
            }
        }
        true
    }
}

/// The size of a split at `path` whose first child is `first` of `available` points, in the
/// split's own mode `size`: a fixed side keeps its points, a ratio stays a ratio.
fn push_size(path: &[bool], size: SplitSize, first: f32, available: f32, out: &mut Vec<(Vec<bool>, SplitSize)>) {
    if let Some(size) = resized(size, first, available) {
        out.push((path.to_vec(), size));
    }
}

/// `size` in its own mode for a first child of `first` out of `available` points.
pub(in crate::docking) fn resized(size: SplitSize, first: f32, available: f32) -> Option<SplitSize> {
    if available <= 0.0 || !available.is_finite() || !first.is_finite() {
        return None;
    }
    let first = first.clamp(0.0, available);
    Some(match size {
        SplitSize::Ratio(_) => SplitSize::Ratio((first / available).clamp(0.0, 1.0)),
        SplitSize::FixedFirst(_) => SplitSize::FixedFirst(first),
        SplitSize::FixedSecond(_) => SplitSize::FixedSecond(available - first),
    })
}

/// One group of a flat row, such as the panels of a stack or the groups of a dock column:
/// its size along the row and its limits.
#[derive(Clone, Copy)]
pub(in crate::docking) struct RowItem {
    pub(in crate::docking) size: f32,
    pub(in crate::docking) min: f32,
    pub(in crate::docking) max: f32,
}

/// How far the divider after `row[index]` can move: `(lo, hi)`, with `lo <= 0 <= hi`, by the
/// rule of [`row_drag`].
pub(in crate::docking) fn row_range(row: &[RowItem], index: usize) -> (f32, f32) {
    let room = |items: &[RowItem]| items.iter().map(|item| (item.size - item.min).max(0.0)).sum::<f32>();
    let grow = |item: Option<&RowItem>| item.map_or(0.0, |item| (item.max - item.size).max(0.0));
    let (before, after) = row.split_at((index + 1).min(row.len()));
    (-room(before).min(grow(after.first())), room(after).min(grow(before.last())))
}

/// The sizes of `row` after the divider after `row[index]` moves by `delta`, clamped to its
/// travel: the item touching the divider on the side it moves toward gives way down to its
/// minimum, then the next one, and so on; the item on the other side takes the room.
pub(in crate::docking) fn row_drag(row: &[RowItem], index: usize, delta: f32) -> Vec<f32> {
    let (lo, hi) = row_range(row, index);
    let delta = if delta.is_finite() { delta.clamp(lo, hi) } else { 0.0 };
    let mut sizes: Vec<f32> = row.iter().map(|item| item.size).collect();
    let (grows, givers): (usize, Vec<usize>) =
        if delta > 0.0 { (index, (index + 1..row.len()).collect()) } else { (index + 1, (0..=index).rev().collect()) };
    let mut rest = delta.abs();
    for giver in givers {
        let (Some(size), Some(item)) = (sizes.get_mut(giver), row.get(giver)) else { continue };
        let taken = rest.min((item.size - item.min).max(0.0));
        *size -= taken;
        rest -= taken;
    }
    if let Some(size) = sizes.get_mut(grows) {
        *size += delta.abs() - rest;
    }
    sizes
}
