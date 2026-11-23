//! Layout to frames: where each group of a docked tree sits, how far each divider can travel and what a drag of a
//! divider writes back.
//!
//! A dragged divider grows the groups touching it on one side, up to their maximums, and shrinks the groups on the
//! other side in turn: the group touching the divider gives way down to its minimum, then the next group along the
//! axis, and so on, until every group on that side is at its minimum. Groups side by side across the axis shrink
//! together, and the line goes on past them once one is at its minimum. The line follows splits of the drag's axis
//! up the tree and ends at a split across it or beside a dock. Every other group keeps its size.
//!
//! The maths runs in reading order: a dock on `DockEdge::Start` is at the left of its rect. `frames` mirrors the
//! rects it returns for right-to-left.

use super::targets::DockSizes;
use crate::contract::SizeLimits;
use craft_behavior::dock::{Column, DockEdge, Node, SplitSize};
use craft_behavior::{Axis, Flow};
use egui::{Rect, Vec2, pos2, vec2};

/// How far each divider beside a group that a drop makes smaller can still travel both ways: the group keeps this
/// much above its minimum, or the groups beyond it give the room.
pub const DROP_MARGIN: f32 = 1.0;

type DockWidthFn<'a, P> = dyn Fn(&[Column<P>]) -> f32 + 'a;

/// What the maths asks of the app: each panel's limits, the width of a dock's columns and which panels have no
/// strip.
pub struct FrameRules<'a, P> {
    limits: &'a dyn Fn(&P) -> SizeLimits,
    dock_width: Option<&'a DockWidthFn<'a, P>>,
    stripless: Option<&'a dyn Fn(&P) -> bool>,
}

impl<P> Clone for FrameRules<'_, P> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<P> Copy for FrameRules<'_, P> {}

impl<'a, P> FrameRules<'a, P> {
    /// Rules with the panels' `limits`. A dock is as wide as its columns' stored widths (the dock column width for
    /// a column without one) until a `dock_width` says otherwise, and every panel has a strip.
    pub fn new(limits: &'a dyn Fn(&P) -> SizeLimits) -> Self {
        FrameRules { limits, dock_width: None, stripless: None }
    }

    /// The width of a dock's columns, as dock columns are drawn.
    #[must_use]
    pub fn dock_width(mut self, width: &'a dyn Fn(&[Column<P>]) -> f32) -> Self {
        self.dock_width = Some(width);
        self
    }

    /// The panels that draw no strip when alone in a group.
    #[must_use]
    pub fn stripless(mut self, stripless: &'a dyn Fn(&P) -> bool) -> Self {
        self.stripless = Some(stripless);
        self
    }

    fn dock_width_of(&self, columns: &[Column<P>], default: f32) -> f32 {
        self.dock_width.map_or_else(|| columns.iter().map(|column| column.width.unwrap_or(default)).sum(), |width| width(columns))
    }
}

/// One leaf of the tree as drawn: a tab group, an accordion or a dock.
#[derive(Debug)]
#[non_exhaustive]
pub struct GroupFrame<'a, P> {
    /// The leaf node.
    pub node: &'a Node<P>,
    /// The path to the node from the root: false is a split's first child.
    pub path: Vec<bool>,
    /// Where the leaf sits, mirrored for right-to-left.
    pub rect: Rect,
    /// What the leaf's panels allow.
    pub limits: SizeLimits,
}

impl<P> GroupFrame<'_, P> {
    /// The panel that names the group: the first of its tabs or sections. A dock has none.
    pub fn anchor(&self) -> Option<&P> {
        match self.node {
            Node::Tabs { panels, .. } => panels.first(),
            Node::Stack { entries, .. } => entries.first().map(|entry| &entry.panel),
            _ => None,
        }
    }

    /// The leaf is a dock, which dock columns draw.
    pub fn is_dock(&self) -> bool {
        matches!(self.node, Node::Dock { .. })
    }
}

/// The divider between the two children of a split that neither side owns.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub struct SplitHandle {
    /// The path of the split from the root.
    pub path: Vec<bool>,
    /// The axis the split lays its children along.
    pub axis: Axis,
    /// The divider's rect, mirrored for right-to-left. It has no thickness when the gap is zero.
    pub divider: Rect,
    /// The rect of the whole split.
    pub split: Rect,
}

/// The frames of a tree in a rect.
#[derive(Debug)]
#[non_exhaustive]
pub struct Frames<'a, P> {
    /// The leaves, first to last.
    pub groups: Vec<GroupFrame<'a, P>>,
    /// The dividers that resize groups. A dock's boundary is the dock's own.
    pub handles: Vec<SplitHandle>,
}

impl<P> Default for Frames<'_, P> {
    fn default() -> Self {
        Frames { groups: Vec::new(), handles: Vec::new() }
    }
}

/// Where every group of `root` sits in `rect`, and the dividers between them.
pub fn frames<'a, P>(root: &'a Node<P>, rect: Rect, sizes: &DockSizes, rules: &FrameRules<'_, P>, rtl: bool) -> Frames<'a, P> {
    let sizer = Sizer { rules, sizes };
    let flow = Flow::new(rect, rtl);
    let mut out = Frames::default();
    sizer.collect(root, rect, &mut Vec::new(), &mut out);
    for group in &mut out.groups {
        group.rect = flow.to_physical(group.rect);
    }
    for handle in &mut out.handles {
        handle.divider = flow.to_physical(handle.divider);
        handle.split = flow.to_physical(handle.split);
    }
    out
}

/// The sizes that move the divider of the split at `path` in `root` by `delta` points along its axis (in reading
/// order), clamped to its travel. Each is a split's path and its new size in its own mode.
pub fn drag_sizes<P>(
    root: &Node<P>,
    rect: Rect,
    sizes: &DockSizes,
    rules: &FrameRules<'_, P>,
    path: &[bool],
    delta: f32,
) -> Vec<(Vec<bool>, SplitSize)> {
    Sizer { rules, sizes }.drag((root, rect), path, delta)
}

/// How far the divider of the split at `path` can move: `(lo, hi)` with `lo <= 0 <= hi`.
pub fn drag_range<P>(root: &Node<P>, rect: Rect, sizes: &DockSizes, rules: &FrameRules<'_, P>, path: &[bool]) -> (f32, f32) {
    Sizer { rules, sizes }.range((root, rect), path)
}

/// Whether every divider across `axis` along a side of the group at `group` can travel `DROP_MARGIN` both ways.
pub fn frees<P>(root: &Node<P>, rect: Rect, sizes: &DockSizes, rules: &FrameRules<'_, P>, group: Rect, axis: Axis) -> bool {
    Sizer { rules, sizes }.frees((root, rect), group, axis)
}

/// Writes `sizes` (from `drag_sizes`) into the splits of `root`.
pub fn apply_sizes<P>(root: &mut Node<P>, sizes: &[(Vec<bool>, SplitSize)]) {
    for (path, size) in sizes {
        let mut node = &mut *root;
        for second_child in path {
            let Node::Split { first, second, .. } = node else { break };
            node = if *second_child { &mut **second } else { &mut **first };
        }
        if let Node::Split { size: held, .. } = node {
            *held = *size;
        }
    }
}

/// The size of a split in its own mode for a first child of `first` out of `available` points.
pub fn resized(size: SplitSize, first: f32, available: f32) -> Option<SplitSize> {
    if available <= 0.0 || !available.is_finite() || !first.is_finite() {
        return None;
    }
    let first = first.clamp(0.0, available);
    Some(match size {
        SplitSize::Ratio(_) => SplitSize::Ratio((first / available).clamp(0.0, 1.0)),
        SplitSize::FixedFirst(_) => SplitSize::FixedFirst(first),
        SplitSize::FixedSecond(_) => SplitSize::FixedSecond(available - first),
        _ => return None,
    })
}

/// The least and most size of a subtree.
#[derive(Clone, Copy)]
struct Bounds {
    min: Vec2,
    max: Vec2,
}

const OPEN_MAX: Vec2 = Vec2::new(SizeLimits::BOUND, SizeLimits::BOUND);

fn extent(rect: Rect, axis: Axis) -> f32 {
    match axis {
        Axis::X => rect.width(),
        _ => rect.height(),
    }
}

fn along(v: Vec2, axis: Axis) -> f32 {
    match axis {
        Axis::X => v.x,
        _ => v.y,
    }
}

fn is_dock<P>(node: &Node<P>) -> bool {
    matches!(node, Node::Dock { .. })
}

/// A split's two children as cells with the divider between them.
struct Cells {
    first: Rect,
    divider: Rect,
    second: Rect,
    first_size: f32,
    available: f32,
}

/// Divides `rect` into two cells and a divider; the first cell's size is clamped into the room.
fn split_cells(rect: Rect, axis: Axis, size: SplitSize, gap: f32) -> Option<Cells> {
    if !(rect.min.is_finite() && rect.max.is_finite() && rect.min.x <= rect.max.x && rect.min.y <= rect.max.y) || !gap.is_finite() || gap < 0.0 {
        return None;
    }
    let total = extent(rect, axis);
    let gap = gap.min(total);
    let available = total - gap;
    let space = f64::from(available);
    let wanted = match size {
        SplitSize::Ratio(ratio) => space * f64::from(ratio),
        SplitSize::FixedFirst(points) => f64::from(points),
        SplitSize::FixedSecond(points) => space - f64::from(points),
        _ => return None,
    };
    if !wanted.is_finite() {
        return None;
    }
    let first_size = wanted.clamp(0.0, space) as f32;
    let (mut first, mut divider, mut second) = (rect, rect, rect);
    match axis {
        Axis::X => {
            first.max.x = (rect.min.x + first_size).min(rect.max.x);
            divider.min.x = first.max.x;
            divider.max.x = (divider.min.x + gap).min(rect.max.x);
            second.min.x = divider.max.x;
        }
        _ => {
            first.max.y = (rect.min.y + first_size).min(rect.max.y);
            divider.min.y = first.max.y;
            divider.max.y = (divider.min.y + gap).min(rect.max.y);
            second.min.y = divider.max.y;
        }
    }
    Some(Cells { first, divider, second, first_size, available })
}

/// The range the first child's size may take in `available` points: its own and the second child's limits, scaled
/// down together when they cannot both fit.
fn split_bounds(available: f32, axis: Axis, first: Bounds, second: Bounds) -> (f32, f32) {
    let (a_min, a_max, b_min, b_max) = (along(first.min, axis), along(first.max, axis), along(second.min, axis), along(second.max, axis));
    let scale = if a_min + b_min > available { available / (a_min + b_min).max(1.0) } else { 1.0 };
    let lo = (a_min * scale).min(available);
    let hi = (available - b_min * scale).max(lo).min(available);
    let constrained_lo = lo.max(available - b_max);
    let constrained_hi = hi.min(a_max);
    if constrained_lo <= constrained_hi { (constrained_lo, constrained_hi) } else { (lo, hi) }
}

type Children<'n, P> = [(&'n Node<P>, Rect); 2];

struct Sizer<'r, 'a, P> {
    rules: &'r FrameRules<'a, P>,
    sizes: &'r DockSizes,
}

impl<P> Sizer<'_, '_, P> {
    fn panel(&self, panel: &P) -> Bounds {
        let limits = (self.rules.limits)(panel);
        Bounds { min: limits.min, max: limits.max.unwrap_or(OPEN_MAX) }
    }

    fn stripless(&self, panels: &[P]) -> bool {
        matches!(panels, [panel] if self.rules.stripless.is_some_and(|stripless| stripless(panel)))
    }

    /// The size policy of a subtree.
    fn bounds(&self, node: &Node<P>) -> Bounds {
        let sizes = self.sizes;
        let bound = SizeLimits::BOUND;
        match node {
            Node::Dock { columns, .. } => {
                let width = self.rules.dock_width_of(columns, sizes.column);
                Bounds { min: vec2(width.clamp(sizes.least_pane, bound), sizes.least_pane), max: OPEN_MAX }
            }
            // A group is as large as its largest panel allows: a panel without a maximum lifts the maximum of
            // the panels it joins.
            Node::Tabs { panels, .. } => {
                let mut min = Vec2::ZERO;
                let mut max = if panels.is_empty() { OPEN_MAX } else { Vec2::ZERO };
                for panel in panels {
                    let panel = self.panel(panel);
                    min = min.max(panel.min).max(Vec2::splat(sizes.least_pane));
                    max = max.max(panel.max);
                }
                max = max.max(min);
                if !self.stripless(panels) {
                    min.y = (min.y + sizes.strip).min(bound);
                    max.y = (max.y + sizes.strip).min(bound);
                }
                Bounds { min, max }
            }
            Node::Stack { entries, .. } => {
                let mut min = Vec2::ZERO;
                let mut max = OPEN_MAX;
                if !entries.is_empty() {
                    max.x = 0.0;
                }
                for entry in entries {
                    let panel = self.panel(&entry.panel);
                    min.x = min.x.max(panel.min.x).max(sizes.least_pane);
                    min.y += if entry.open { panel.min.y.max(sizes.least_pane) } else { 0.0 } + sizes.strip;
                    max.x = max.x.max(panel.max.x);
                }
                min = min.min(Vec2::splat(bound));
                Bounds { min, max: max.max(min) }
            }
            Node::Split { axis, first, second, .. } => {
                let (a, b) = (self.bounds(first), self.bounds(second));
                let gap = sizes.gap;
                let (min, max) = match axis {
                    Axis::X => (vec2(a.min.x + b.min.x + gap, a.min.y.max(b.min.y)), vec2(a.max.x + b.max.x + gap, a.max.y.min(b.max.y))),
                    _ => (vec2(a.min.x.max(b.min.x), a.min.y + b.min.y + gap), vec2(a.max.x.min(b.max.x), a.max.y + b.max.y + gap)),
                };
                Bounds { min: min.min(Vec2::splat(bound)), max: max.max(min).min(Vec2::splat(bound)) }
            }
            _ => Bounds { min: Vec2::ZERO, max: OPEN_MAX },
        }
    }

    /// The split as drawn: its stored size, clamped so both children fit their limits.
    fn split(&self, axis: Axis, size: SplitSize, first: &Node<P>, second: &Node<P>, rect: Rect) -> Option<Cells> {
        let gap = self.sizes.gap;
        let wanted = split_cells(rect, axis, size, gap)?;
        let (lo, hi) = split_bounds(wanted.available, axis, self.bounds(first), self.bounds(second));
        split_cells(rect, axis, SplitSize::FixedFirst(wanted.first_size.clamp(lo, hi)), gap)
    }

    /// The rects of a dock and of the node beside it in `rect`.
    fn dock_cells(&self, edge: DockEdge, columns: &[Column<P>], other: &Node<P>, rect: Rect) -> (Rect, Rect) {
        let gap = self.sizes.gap;
        let room = (rect.width() - gap - self.bounds(other).min.x).clamp(0.0, rect.width());
        let width = self.rules.dock_width_of(columns, self.sizes.column).clamp(0.0, room);
        match edge {
            DockEdge::End => (
                Rect::from_min_max(pos2(rect.right() - width, rect.top()), rect.max),
                Rect::from_min_max(rect.min, pos2((rect.right() - width - gap).max(rect.left()), rect.bottom())),
            ),
            _ => (
                Rect::from_min_max(rect.min, pos2(rect.left() + width, rect.bottom())),
                Rect::from_min_max(pos2((rect.left() + width + gap).min(rect.right()), rect.top()), rect.max),
            ),
        }
    }

    /// The children of a split drawn in `rect`, with their rects.
    fn children<'n>(&self, node: &'n Node<P>, rect: Rect) -> Option<Children<'n, P>> {
        let Node::Split { axis, size, first, second, .. } = node else { return None };
        match (&**first, &**second) {
            (Node::Dock { side, columns, .. }, other) if *axis == Axis::X => {
                let (dock, rest) = self.dock_cells(*side, columns, other, rect);
                Some([(first, dock), (second, rest)])
            }
            (other, Node::Dock { side, columns, .. }) if *axis == Axis::X => {
                let (dock, rest) = self.dock_cells(*side, columns, other, rect);
                Some([(first, rest), (second, dock)])
            }
            _ => {
                let cells = self.split(*axis, *size, first, second, rect)?;
                Some([(first, cells.first), (second, cells.second)])
            }
        }
    }

    fn collect<'n>(&self, node: &'n Node<P>, rect: Rect, path: &mut Vec<bool>, out: &mut Frames<'n, P>) {
        let Some(children) = self.children(node, rect) else {
            let bounds = self.bounds(node);
            out.groups.push(GroupFrame { node, path: path.clone(), rect, limits: SizeLimits { min: bounds.min, max: Some(bounds.max) } });
            return;
        };
        if let Node::Split { axis, size, first, second, .. } = node
            && !is_dock(first)
            && !is_dock(second)
            && let Some(cells) = self.split(*axis, *size, first, second, rect)
        {
            out.handles.push(SplitHandle { path: path.clone(), axis: *axis, divider: cells.divider, split: rect });
        }
        for (second_child, (child, cell)) in children.into_iter().enumerate() {
            path.push(second_child == 1);
            self.collect(child, cell, path, out);
            path.pop();
        }
    }

    /// How far the end edge (`end`) or start edge of `node` drawn in `rect` can move along `axis`: inward until every
    /// group along the axis from that edge is at its minimum, and outward while the groups touching it stay below
    /// their maximums. Groups side by side across the axis move together, so the first of them to reach its limit
    /// stops them.
    fn travel(&self, node: &Node<P>, rect: Rect, axis: Axis, end: bool) -> (f32, f32) {
        let docked = matches!(node, Node::Split { first, second, .. } if is_dock(first) || is_dock(second));
        match (node, self.children(node, rect)) {
            (Node::Split { first, .. }, Some([a, b])) if docked => {
                let (other, rect) = if is_dock(first) { b } else { a };
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
                let bounds = self.bounds(node);
                let size = extent(rect, axis);
                ((size - along(bounds.min, axis)).max(0.0), (along(bounds.max, axis) - size).max(0.0))
            }
        }
    }

    /// The sizes that move the end (`end`) or start edge of `node` drawn in `rect` outward by `grow` points.
    /// Growing, only the groups touching that edge change. Shrinking, the group at the edge gives way down to its
    /// minimum, then the next one along the axis, and so on. Paths are relative to `path`.
    fn follow(&self, (node, rect): (&Node<P>, Rect), axis: Axis, end: bool, grow: f32, path: &mut Vec<bool>, out: &mut Vec<(Vec<bool>, SplitSize)>) {
        let Some([a, b]) = self.children(node, rect) else { return };
        let Node::Split { axis: split_axis, size, first, second, .. } = node else { return };
        let docked = is_dock(first) || is_dock(second);
        let visit = |child: (&Node<P>, Rect), second_child: bool, grow: f32, path: &mut Vec<bool>, out: &mut Vec<(Vec<bool>, SplitSize)>| {
            path.push(second_child);
            self.follow(child, axis, end, grow, path, out);
            path.pop();
        };
        if docked {
            let dock_first = is_dock(first);
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

    /// The ancestors that continue the line of the split at the end of `chain` along `axis`: each with its children
    /// and whether the line runs through its second child, nearest first. The line ends at a split across the axis
    /// or beside a dock.
    fn line<'n>(&self, chain: &[(&'n Node<P>, Rect)], path: &[bool], axis: Axis) -> Vec<(usize, Children<'n, P>, bool)> {
        let mut line = Vec::new();
        for depth in (0..path.len()).rev() {
            let Some(&(node, rect)) = chain.get(depth) else { break };
            let Node::Split { axis: split_axis, first, second, .. } = node else { break };
            if *split_axis != axis || is_dock(first) || is_dock(second) {
                break;
            }
            let (Some(children), Some(&second_child)) = (self.children(node, rect), path.get(depth)) else { break };
            line.push((depth, children, second_child));
        }
        line
    }

    /// The divider of the split at `path` under `root` moved by `delta` points, clamped to its travel. The groups
    /// touching the divider on the side it moves toward give way, then the groups beyond them along the axis, one
    /// after another, as far as the line of same-axis splits reaches; the groups on the other side that touch the
    /// divider take the room. Paths are absolute, like `path`.
    fn drag(&self, root: (&Node<P>, Rect), path: &[bool], delta: f32) -> Vec<(Vec<bool>, SplitSize)> {
        let Some(chain) = self.chain(root, path) else { return Vec::new() };
        let Some(&(node, rect)) = chain.last() else { return Vec::new() };
        let Node::Split { axis, size, first, second, .. } = node else { return Vec::new() };
        let axis = *axis;
        let Some(cells) = self.split(axis, *size, first, second, rect) else { return Vec::new() };
        let (lo, hi) = self.range(root, path);
        let delta = delta.clamp(lo, hi);
        if delta == 0.0 || !delta.is_finite() || cells.available <= 0.0 {
            return Vec::new();
        }
        // Toward the end edge the second child gives way, toward the start the first.
        let forward = delta > 0.0;
        let moved = delta.abs();
        let given = if forward {
            moved.min(self.travel(second, cells.second, axis, false).0)
        } else {
            moved.min(self.travel(first, cells.first, axis, true).0)
        };
        let (first_grow, second_grow) = if forward { (moved, -given) } else { (-given, moved) };
        let mut out = Vec::new();
        let mut path = path.to_vec();
        let first_size = cells.first_size + first_grow;
        push_size(&path, *size, first_size, first_size + extent(cells.second, axis) + second_grow, &mut out);
        path.push(false);
        self.follow((first, cells.first), axis, true, first_grow, &mut path, &mut out);
        path.pop();
        path.push(true);
        self.follow((second, cells.second), axis, false, second_grow, &mut path, &mut out);
        path.pop();
        // What the split's own child could not give, the ancestors' children beyond it give: the split grows by
        // `rest` toward them, and so does each ancestor until one absorbs it.
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

    /// How far the divider of the split at `path` under `root` can move: `(lo, hi)`, with `lo <= 0 <= hi`. Each way
    /// it is the lesser of the room the groups touching the divider have to grow and the room every group beyond the
    /// divider, along the line of same-axis splits, has to shrink.
    fn range(&self, root: (&Node<P>, Rect), path: &[bool]) -> (f32, f32) {
        let Some(chain) = self.chain(root, path) else { return (0.0, 0.0) };
        let Some(&(node, rect)) = chain.last() else { return (0.0, 0.0) };
        let Node::Split { axis, size, first, second, .. } = node else { return (0.0, 0.0) };
        let Some(cells) = self.split(*axis, *size, first, second, rect) else { return (0.0, 0.0) };
        let (mut first_in, first_out) = self.travel(first, cells.first, *axis, true);
        let (mut second_in, second_out) = self.travel(second, cells.second, *axis, false);
        for (_, [a, b], second_child) in self.line(&chain, path, *axis) {
            if second_child {
                first_in += self.travel(a.0, a.1, *axis, true).0;
            } else {
                second_in += self.travel(b.0, b.1, *axis, false).0;
            }
        }
        (-first_in.min(second_out), first_out.min(second_in))
    }

    /// Whether every divider of `root` across `axis` that runs along a side of the group drawn at `group` can travel
    /// `DROP_MARGIN` both ways: a group at its minimum holds no divider still while the groups beyond it can give way.
    fn frees(&self, root: (&Node<P>, Rect), group: Rect, axis: Axis) -> bool {
        let edges = |rect: Rect| match axis {
            Axis::X => (rect.left(), rect.right()),
            _ => (rect.top(), rect.bottom()),
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

/// The size of a split at `path` whose first child is `first` of `available` points, in the split's own mode `size`:
/// a fixed side keeps its points, a ratio stays a ratio.
fn push_size(path: &[bool], size: SplitSize, first: f32, available: f32, out: &mut Vec<(Vec<bool>, SplitSize)>) {
    if let Some(size) = resized(size, first, available) {
        out.push((path.to_vec(), size));
    }
}
