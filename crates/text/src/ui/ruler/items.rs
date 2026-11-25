//! The things a ruler edits (the indent markers and the stops), their parts, and the edits on [`RulerValues`].

use super::{GapEdge, RulerValues, SAME_POSITION, TabStop};
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
/// One edge of a column gap, `GAP.at(gap_part(i, edge))`: below the stops and indents in priority, so a marker sitting
/// on a gap edge still wins the press.
pub(super) const GAP: PartKey = PartKey::new("gap");
/// The rank of a gap edge among the handles.
pub(super) const GAP_RANK: u8 = 0;
/// The rank of the markers, stops and table column borders, above [`GAP_RANK`].
pub(super) const MARKER_RANK: u8 = 1;
/// The tab selector in the corner.
pub(super) const SELECTOR: PartKey = PartKey::new("selector");
/// The strip: a press on the empty column places a stop.
pub(super) const STRIP: PartKey = PartKey::new("strip");

/// The part index of `edge` of gap `index`: two per gap.
pub(super) fn gap_part(index: usize, edge: GapEdge) -> u64 {
    index as u64 * 2 + u64::from(edge == GapEdge::Start)
}

/// The gap and edge a part index of [`GAP`] stands for.
pub(super) fn gap_of_part(part: u64) -> (usize, GapEdge) {
    ((part / 2) as usize, if part.is_multiple_of(2) { GapEdge::End } else { GapEdge::Start })
}

/// A marker or stop on the ruler.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum RulerItem {
    /// The first-line indent marker.
    FirstLine,
    /// The triangle under the first-line one: moves the left indent, the first line stays.
    Hanging,
    /// The box under the hanging triangle: moves first line and left indent together.
    LeftBox,
    /// The right indent marker.
    Right,
    /// The stop at this index of [`RulerValues::stops`].
    Stop(usize),
}

impl RulerItem {
    /// Every item in the order Page Down visits them: the markers, then the stops.
    pub(super) fn all(values: &RulerValues) -> Vec<RulerItem> {
        let markers = [RulerItem::FirstLine, RulerItem::Hanging, RulerItem::LeftBox, RulerItem::Right];
        markers.into_iter().chain((0..values.stops.len()).map(RulerItem::Stop)).collect()
    }

    /// The part of the item.
    pub(super) fn part(self) -> PartRef {
        match self {
            RulerItem::FirstLine => FIRST.into(),
            RulerItem::Hanging => HANGING.into(),
            RulerItem::LeftBox => LEFT.into(),
            RulerItem::Right => RIGHT.into(),
            RulerItem::Stop(at) => STOP.at(at as u64),
        }
    }

    /// The item a part belongs to.
    pub(super) fn of_part(part: PartRef) -> Option<RulerItem> {
        match part.key {
            key if key == FIRST => Some(RulerItem::FirstLine),
            key if key == HANGING => Some(RulerItem::Hanging),
            key if key == LEFT => Some(RulerItem::LeftBox),
            key if key == RIGHT => Some(RulerItem::Right),
            key if key == STOP => part.index.map(|at| RulerItem::Stop(at as usize)),
            _ => None,
        }
    }

    /// Where the item sits, points from the column's start edge.
    pub(super) fn position(self, values: &RulerValues, width: f32) -> f32 {
        match self {
            RulerItem::FirstLine => values.left_indent + values.first_line_indent,
            RulerItem::Hanging | RulerItem::LeftBox => values.left_indent,
            RulerItem::Right => width - values.right_indent,
            RulerItem::Stop(at) => values.stops.get(at).map_or(0.0, |stop| stop.position),
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

/// `values` with every stop moved by the offset that takes the stop `grab` to `to` points: spacing and kinds stay, the
/// lowest stop stays at 0 or above and the highest within the column. A column the stops do not fit in moves none.
pub(super) fn moved_all(values: &RulerValues, grab: usize, to: f32, width: f32) -> RulerValues {
    let mut next = values.clone();
    let Some(start) = values.stops.get(grab).map(|stop| stop.position) else { return next };
    if !to.is_finite() {
        return next;
    }
    let lowest = values.stops.iter().map(|stop| stop.position).fold(f32::INFINITY, f32::min);
    let highest = values.stops.iter().map(|stop| stop.position).fold(f32::NEG_INFINITY, f32::max);
    let (least, most) = (-lowest, width.max(0.0) - highest);
    let offset = if least <= most { (to - start).clamp(least, most) } else { 0.0 };
    for stop in &mut next.stops {
        stop.position += offset;
    }
    next
}

/// `values` with `item` moved to `to` points (kept within the column), and where the item is
/// afterwards (a stop moved past another has a new index).
pub(super) fn moved(values: &RulerValues, item: RulerItem, to: f32, width: f32) -> (RulerValues, RulerItem) {
    if !to.is_finite() {
        return (values.clone(), item);
    }
    let to = to.clamp(0.0, width.max(0.0));
    let mut next = values.clone();
    match item {
        RulerItem::FirstLine => next.first_line_indent = to - values.left_indent,
        RulerItem::LeftBox => next.left_indent = to,
        RulerItem::Hanging => {
            next.left_indent = to;
            next.first_line_indent = values.left_indent + values.first_line_indent - to;
        }
        RulerItem::Right => next.right_indent = (width - to).clamp(0.0, (width - values.left_indent).max(0.0)),
        RulerItem::Stop(at) => {
            let Some(stop) = values.stops.get(at).copied() else { return (next, item) };
            let (placed, now) = place(&without(values, at), TabStop { position: to, ..stop });
            return (placed, RulerItem::Stop(now));
        }
    }
    (next, item)
}
