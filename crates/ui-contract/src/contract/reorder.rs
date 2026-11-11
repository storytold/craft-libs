//! The shared parts of reordering by pointer or key: where a drop lands between items, where a moved item ends up,
//! and the stacking keys' move. A collection's row drag and a section stack both use them.

use crate::runtime::drag::DropMark;
use crate::runtime::keys::Intents;
use craft_behavior::{Axis, Intent, NavOutline, keys};
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

/// A stacking-key move of this pass (`Intent::MoveItem`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct ItemMove {
    /// Toward the front (`]`).
    pub front: bool,
    /// All the way, not one place.
    pub all_the_way: bool,
}

impl ItemMove {
    /// Whether the move goes toward the end of an order laid out along `axis`.
    pub fn toward_end(self, axis: Axis) -> bool {
        keys::toward_end(self.front, axis)
    }
}

/// The stacking-key move routed to the focused item this pass.
pub fn take_move_item(intents: &mut Intents) -> Option<ItemMove> {
    [(true, false), (false, false), (true, true), (false, true)]
        .into_iter()
        .find(|&(front, all_the_way)| intents.take_exact(&Intent::move_item(front, all_the_way)))
        .map(|(front, all_the_way)| ItemMove { front, all_the_way })
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

/// Where a stacking-key move lands: the gap before position `insert` of the display order (`len` after the last), in
/// the group row `parent` (`None` at the top level).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct KeyGap {
    /// The insert position among the rows, as for `Gap::insert`.
    pub insert: usize,
    /// The group row the items land in.
    pub parent: Option<usize>,
}

/// Where a stacking key moves the items at `places` (display positions in order, none inside another) of an order of
/// `len` items: one place toward the end when `toward_end`, else toward the start, or all the way with `all_the_way`.
/// With `tree` (a reorder rule that lands `into_groups`) the items move as subtrees: one place toward the start lands
/// before the row above the first item, at that row's depth; one place toward the end steps out of a group past its
/// last child, into an open group past its header, and otherwise past the next row; all the way lands at the start or
/// end of the top level. `None` when the items cannot move that way.
pub fn key_move(len: usize, tree: Option<&dyn NavOutline>, places: &[usize], toward_end: bool, all_the_way: bool) -> Option<KeyGap> {
    let (&first, &last) = (places.first()?, places.last()?);
    let depth = |row: usize| tree.map_or(0, |outline| outline.depth(row));
    let parent = |row: usize| tree.and_then(|outline| parent_of(outline, row));
    // The last row of the subtree at `row`.
    let subtree_end = |row: usize| (row + 1..len).find(|&below| depth(below) <= depth(row)).map_or(len - 1, |below| below - 1);
    let block_end = subtree_end(last);
    let packed = places.windows(2).all(|pair| pair[1] == subtree_end(pair[0]) + 1);
    let gap = match (toward_end, all_the_way) {
        (false, false) if first > 0 => KeyGap { insert: first - 1, parent: parent(first - 1) },
        (false, true) if !(first == 0 && packed) => KeyGap { insert: 0, parent: None },
        (true, false) if block_end + 1 < len => {
            let next = block_end + 1;
            let opens = tree.is_some_and(|outline| outline.has_children(next) && outline.open(next));
            if depth(next) < depth(last) {
                KeyGap { insert: next, parent: parent(next) }
            } else if opens && next + 1 < len && depth(next + 1) > depth(next) {
                KeyGap { insert: next + 1, parent: Some(next) }
            } else {
                KeyGap { insert: subtree_end(next) + 1, parent: parent(next) }
            }
        }
        (true, true) if !(block_end + 1 == len && packed) => KeyGap { insert: len, parent: None },
        _ => return None,
    };
    let mut held = gap.parent;
    while let Some(row) = held {
        if places.contains(&row) {
            return None;
        }
        held = parent(row);
    }
    Some(gap)
}
