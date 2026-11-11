//! The things a ruler edits (the indent markers and the stops), their parts, and the edits on [`RulerValues`].

use super::{RulerValues, SAME_POSITION, TabStop};
use craft_behavior::PartKey;
use craft_behavior::part::PartRef;

/// The first-line indent marker.
pub(super) const FIRST: PartKey = PartKey::new("first");
/// The hanging indent marker (moves the left indent, the first line stays).
pub(super) const HANGING: PartKey = PartKey::new("hanging");
/// The left indent box (moves first line and left indent together).
pub(super) const LEFT: PartKey = PartKey::new("left");
/// The right indent marker.
pub(super) const RIGHT: PartKey = PartKey::new("right");
/// One tab stop, `STOP.at(i)`.
pub(super) const STOP: PartKey = PartKey::new("stop");
/// One table column border, `COLUMN.at(i)`.
pub(super) const COLUMN: PartKey = PartKey::new("column");
/// The tab selector in the corner.
pub(super) const SELECTOR: PartKey = PartKey::new("selector");
/// The strip: a press on the empty column places a stop.
pub(super) const STRIP: PartKey = PartKey::new("strip");

/// A marker or stop on the ruler.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Item {
    FirstLine,
    /// The triangle under the first-line one: moves the left indent, the first line stays.
    Hanging,
    /// The box under the hanging triangle: moves first line and left indent together.
    LeftBox,
    Right,
    /// The stop at this index of [`RulerValues::stops`].
    Stop(usize),
}

impl Item {
    /// Every item in the order Page Down visits them: the markers, then the stops.
    pub(super) fn all(values: &RulerValues) -> Vec<Item> {
        let markers = [Item::FirstLine, Item::Hanging, Item::LeftBox, Item::Right];
        markers.into_iter().chain((0..values.stops.len()).map(Item::Stop)).collect()
    }

    /// The part of the item.
    pub(super) fn part(self) -> PartRef {
        match self {
            Item::FirstLine => FIRST.into(),
            Item::Hanging => HANGING.into(),
            Item::LeftBox => LEFT.into(),
            Item::Right => RIGHT.into(),
            Item::Stop(at) => STOP.at(at as u64),
        }
    }

    /// The item a part belongs to.
    pub(super) fn of_part(part: PartRef) -> Option<Item> {
        match part.key {
            key if key == FIRST => Some(Item::FirstLine),
            key if key == HANGING => Some(Item::Hanging),
            key if key == LEFT => Some(Item::LeftBox),
            key if key == RIGHT => Some(Item::Right),
            key if key == STOP => part.index.map(|at| Item::Stop(at as usize)),
            _ => None,
        }
    }

    /// Where the item sits, points from the column's start edge.
    pub(super) fn position(self, values: &RulerValues, width: f32) -> f32 {
        match self {
            Item::FirstLine => values.left_indent + values.first_line_indent,
            Item::Hanging | Item::LeftBox => values.left_indent,
            Item::Right => width - values.right_indent,
            Item::Stop(at) => values.stops.get(at).map_or(0.0, |stop| stop.position),
        }
    }
}

/// `values` with `stop` set: stops within [`SAME_POSITION`] of it go, the rest stay sorted. The
/// index of `stop` comes with it.
pub(super) fn place(values: &RulerValues, stop: TabStop) -> (RulerValues, usize) {
    let mut placed = values.clone();
    placed.stops.retain(|other| (other.position - stop.position).abs() >= SAME_POSITION);
    placed.stops.push(stop);
    placed.stops.sort_by(|a, b| a.position.total_cmp(&b.position));
    let at = placed.stops.iter().position(|other| *other == stop).unwrap_or(0);
    (placed, at)
}

/// `values` without the stop at `at`.
pub(super) fn without(values: &RulerValues, at: usize) -> RulerValues {
    let mut cleared = values.clone();
    if at < cleared.stops.len() {
        cleared.stops.remove(at);
    }
    cleared
}

/// `values` with `item` moved to `to` points (kept within the column), and where the item is
/// afterwards (a stop moved past another has a new index).
pub(super) fn moved(values: &RulerValues, item: Item, to: f32, width: f32) -> (RulerValues, Item) {
    if !to.is_finite() {
        return (values.clone(), item);
    }
    let to = to.clamp(0.0, width.max(0.0));
    let mut next = values.clone();
    match item {
        Item::FirstLine => next.first_line_indent = to - values.left_indent,
        Item::LeftBox => next.left_indent = to,
        Item::Hanging => {
            next.left_indent = to;
            next.first_line_indent = values.left_indent + values.first_line_indent - to;
        }
        Item::Right => next.right_indent = (width - to).clamp(0.0, (width - values.left_indent).max(0.0)),
        Item::Stop(at) => {
            let Some(stop) = values.stops.get(at).copied() else { return (next, item) };
            let (placed, now) = place(&without(values, at), TabStop { position: to, ..stop });
            return (placed, Item::Stop(now));
        }
    }
    (next, item)
}

/// The item after (or before) `picked` in `items`, staying at the ends.
pub(super) fn pick(items: &[Item], picked: Option<Item>, forward: bool) -> Option<Item> {
    let at = picked.and_then(|picked| items.iter().position(|item| *item == picked));
    let to = match (at, forward) {
        (None, true) => 0,
        (None, false) => items.len().saturating_sub(1),
        (Some(at), true) => (at + 1).min(items.len().saturating_sub(1)),
        (Some(at), false) => at.saturating_sub(1),
    };
    items.get(to).copied()
}
