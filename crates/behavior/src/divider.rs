//! The push-neighbours resize of a row of segments separated by dividers.
//!
//! Dragging the divider after segment `i` toward one side makes the segment before it (toward the start) or after it
//! grow, and the segments on the other side give way: the one touching the divider gives way down to its minimum,
//! then the next one, and so on. The row's total stays the same.

/// One segment of a row: its extent along the row and its limits.
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
pub struct Segment {
    /// The extent along the row.
    pub extent: f32,
    /// The least extent.
    pub min: f32,
    /// The most extent; infinite for a segment that grows without limit.
    pub max: f32,
}

impl Segment {
    /// A segment of `extent` that stays within `min` and `max`.
    pub const fn new(extent: f32, min: f32, max: f32) -> Self {
        Segment { extent, min, max }
    }

    /// A segment of `extent` with no upper limit.
    pub const fn unbounded(extent: f32, min: f32) -> Self {
        Segment { extent, min, max: f32::INFINITY }
    }
}

/// How far the divider after `row[index]` can move: `(lo, hi)`, with `lo <= 0 <= hi`. Toward the start the segments
/// before it give way down to their minimums and the segment after it grows to its maximum at most; toward the end it
/// is the other way round.
pub fn travel(row: &[Segment], index: usize) -> (f32, f32) {
    let room = |items: &[Segment]| items.iter().map(|item| (item.extent - item.min).max(0.0)).sum::<f32>();
    let grow = |item: Option<&Segment>| item.map_or(0.0, |item| (item.max - item.extent).max(0.0));
    let (before, after) = row.split_at((index + 1).min(row.len()));
    (-room(before).min(grow(after.first())), room(after).min(grow(before.last())))
}

/// The extents of `row` after the divider after `row[index]` moves to `target`, a distance from the row's start, clamped
/// to its [`travel`]. The extents depend on the row as it stands and on the target alone, so a drag that goes past a
/// limit and back puts the divider under the pointer again.
pub fn resize(row: &[Segment], index: usize, target: f32) -> Vec<f32> {
    let at: f32 = row.iter().take(index + 1).map(|item| item.extent).sum();
    let (lo, hi) = travel(row, index);
    let delta = if target.is_finite() { (target - at).clamp(lo, hi) } else { 0.0 };
    let mut extents: Vec<f32> = row.iter().map(|item| item.extent).collect();
    let (grows, givers): (usize, Vec<usize>) =
        if delta > 0.0 { (index, (index + 1..row.len()).collect()) } else { (index + 1, (0..=index).rev().collect()) };
    let mut rest = delta.abs();
    for giver in givers {
        let (Some(extent), Some(item)) = (extents.get_mut(giver), row.get(giver)) else { continue };
        let taken = rest.min((item.extent - item.min).max(0.0));
        *extent -= taken;
        rest -= taken;
    }
    if let Some(extent) = extents.get_mut(grows) {
        *extent += delta.abs() - rest;
    }
    extents
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row() -> Vec<Segment> {
        vec![Segment::unbounded(100.0, 40.0), Segment::unbounded(100.0, 40.0), Segment::unbounded(100.0, 40.0)]
    }

    #[test]
    fn the_segments_past_the_divider_give_way_in_turn() {
        // The divider after the first segment moves 80 points toward the end: the second gives 60, the third 20.
        assert_eq!(resize(&row(), 0, 180.0), vec![180.0, 40.0, 80.0]);
    }
}
