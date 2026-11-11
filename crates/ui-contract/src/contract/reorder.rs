//! The shared parts of reordering by pointer or key: where a drop lands between items, where a moved item ends up,
//! and the Alt+arrow intent. A collection's row drag and a section stack both use them.

use crate::runtime::drag::DropMark;
use crate::runtime::keys::Intents;
use craft_behavior::{Axis, Intent, NavOutline};
use egui::{Pos2, Rect};

/// The gap a drop would land in.
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
pub struct Gap {
    /// The insert position among the rects: the gap before rect `insert`, `rects.len()` after the last.
    pub insert: usize,
    /// The insertion line across the first rect's extent.
    pub mark: DropMark,
}

/// The gap nearest `at` among `rects`, which lie in order along `axis`: before the first rect whose middle is past
/// the pointer, else after the last. `None` without rects. With one rect, `insert` is 0 above its middle and 1 below.
pub fn gap_at(rects: &[Rect], axis: Axis, at: Pos2) -> Option<Gap> {
    let (first, last) = (rects.first()?, rects.last()?);
    let along = |point: Pos2| match axis {
        Axis::X => point.x,
        _ => point.y,
    };
    let pointer = along(at);
    let insert = rects.iter().position(|rect| pointer < along(rect.center())).unwrap_or(rects.len());
    let mark = match axis {
        Axis::X => {
            let x = rects.get(insert).map_or(last.max.x, |rect| rect.min.x);
            DropMark::line(Pos2::new(x, first.min.y), Pos2::new(x, first.max.y))
        }
        _ => {
            let y = rects.get(insert).map_or(last.max.y, |rect| rect.min.y);
            DropMark::line(Pos2::new(first.min.x, y), Pos2::new(first.max.x, y))
        }
    };
    Some(Gap { insert, mark })
}

/// The index item `from` has once it moves to the gap before item `insert` of the list it is in: it leaves its place
/// first, so a gap below it counts one less.
pub fn move_to(from: usize, insert: usize) -> usize {
    move_all_to(&[from], insert)
}

/// The index the first of `places` has once all of them move to the gap before item `insert`: every moved item above
/// the gap leaves first.
pub fn move_all_to(places: &[usize], insert: usize) -> usize {
    insert - places.iter().filter(|&&place| place < insert).count()
}

/// The index an Alt+arrow nudge moves item `at` of `len` to: one step toward the end when `forward`, else toward the
/// start, stopping at the ends.
pub fn nudge_to(at: usize, len: usize, forward: bool) -> usize {
    if forward { (at + 1).min(len.saturating_sub(1)) } else { at.saturating_sub(1) }
}

/// The Alt+arrow move of the focused item this pass: `Some(true)` toward the end, `Some(false)` toward the start.
pub fn take_move_item(intents: &mut Intents) -> Option<bool> {
    if intents.take_exact(&Intent::move_item(true)) {
        Some(true)
    } else if intents.take_exact(&Intent::move_item(false)) {
        Some(false)
    } else {
        None
    }
}

/// The share of a group row's height, at its top and at its bottom, where a drop lands before or after the row; the
/// middle drops into the group.
const GROUP_EDGE: f32 = 0.25;

/// Where a drop in a tree lands: before row `insert` of the display order, inside the group row `parent` (`None` at the
/// top level).
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
pub struct TreeGap {
    /// The insert position among the rows, as for `Gap::insert`.
    pub insert: usize,
    /// The group row the items land in.
    pub parent: Option<usize>,
    /// The line at the gap, at the depth the items land at, or `Into` the group row.
    pub mark: DropMark,
}

/// The row that holds row `at`: the nearest row above it that is shallower. `None` for a root.
pub fn parent_of(outline: &dyn NavOutline, at: usize) -> Option<usize> {
    let depth = outline.depth(at);
    if depth == 0 { None } else { (0..at).rev().find(|&row| outline.depth(row) < depth) }
}

/// The row that holds a row of `depth` placed right below row `above`: the nearest row from `above` upward that is
/// shallower than `depth`.
fn holder_below(outline: &dyn NavOutline, above: usize, depth: usize) -> Option<usize> {
    if depth == 0 { None } else { (0..=above).rev().find(|&row| outline.depth(row) < depth) }
}

/// Where a drop at `at` over row `over` of a tree lands. `row` is the row's rect and `step` its indent step. A row that
/// has children takes the drop into itself over its middle; over its top or bottom, and over any other row's top or
/// bottom half, the drop lands before or after the row at the row's depth. Below the last row of a group the pointer's
/// indent picks the depth, between the row's own and the next row's.
pub fn tree_gap_at(outline: &dyn NavOutline, over: usize, row: Rect, rtl: bool, step: f32, at: Pos2) -> TreeGap {
    let holds = outline.has_children(over);
    let depth = outline.depth(over);
    let edge = if holds { GROUP_EDGE } else { 0.5 };
    let from_top = if row.height() > 0.0 { (at.y - row.min.y) / row.height() } else { 0.0 };
    let line = |y: f32, depth: usize| {
        let reach = depth as f32 * step;
        let (from, to) = if rtl { (row.min.x, row.max.x - reach) } else { (row.min.x + reach, row.max.x) };
        DropMark::line(Pos2::new(from, y), Pos2::new(to, y))
    };
    if holds && from_top >= edge && from_top < 1.0 - edge {
        return TreeGap { insert: over + 1, parent: Some(over), mark: DropMark::Into(row) };
    }
    if from_top < edge {
        return TreeGap { insert: over, parent: parent_of(outline, over), mark: line(row.min.y, depth) };
    }
    let next = if over + 1 < outline.len() { outline.depth(over + 1) } else { 0 };
    if holds && next > depth {
        return TreeGap { insert: over + 1, parent: Some(over), mark: line(row.max.y, depth + 1) };
    }
    let lead = if rtl { row.max.x - at.x } else { at.x - row.min.x };
    let picked = if step > 0.0 { (lead.max(0.0) / step).floor() as usize } else { depth };
    let landed = picked.clamp(next.min(depth), depth);
    TreeGap { insert: over + 1, parent: holder_below(outline, over, landed), mark: line(row.max.y, landed) }
}

/// Where an Alt+arrow nudge of row `at` in a tree puts it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct TreeNudge {
    /// The row's new position, counted in the order that remains once it is taken out.
    pub to: usize,
    /// The group row the row lands in; `None` at the top level.
    pub parent: Option<usize>,
}

/// The move of Alt+arrow on row `at` of a tree: one display row toward the end when `forward`, else toward the start.
/// Moving onto a group's header from outside enters it (first child going forward); moving past the end of an open
/// group's children from below enters it as its last child. Moving past a group's first or last child leaves it. `None`
/// at the ends of the top level.
pub fn nudge_in_tree(outline: &dyn NavOutline, at: usize, forward: bool) -> Option<TreeNudge> {
    let depth = outline.depth(at);
    let parent = parent_of(outline, at);
    let enters = |row: usize| outline.has_children(row) && outline.open(row);
    if forward {
        let next = at + 1;
        if next >= outline.len() || outline.depth(next) < depth {
            return parent.map(|group| TreeNudge { to: at, parent: parent_of(outline, group) });
        }
        if outline.depth(next) > depth {
            return None;
        }
        Some(if enters(next) { TreeNudge { to: next, parent: Some(next) } } else { TreeNudge { to: next, parent } })
    } else {
        let above = at.checked_sub(1)?;
        let above_depth = outline.depth(above);
        if above_depth > depth {
            let sibling = (0..above).rev().find(|&row| outline.depth(row) <= depth).filter(|&row| outline.depth(row) == depth)?;
            return Some(TreeNudge { to: at, parent: Some(sibling) });
        }
        if above_depth < depth {
            return Some(TreeNudge { to: above, parent: parent_of(outline, above) });
        }
        Some(if enters(above) { TreeNudge { to: at, parent: Some(above) } } else { TreeNudge { to: above, parent } })
    }
}
