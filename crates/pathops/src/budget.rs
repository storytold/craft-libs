//! What the sweep that builds a planar map may be given. The topology it builds holds every
//! crossing of the input's segments, and a crossing can only be where two segments' bounds
//! overlap (two cubics cross at most 9 times). The pairs of overlapping bounds are counted before
//! the sweep, exactly and in O(n log n), and past [`MAX_SEGMENT_PAIRS`] the operation fails as
//! [`crate::PathOpsError::TooComplex`]: one compact self-crossing path could otherwise make
//! millions of crossings, and a live effect rebuilt them on every draw.

use kurbo::{BezPath, ParamCurveExtrema, Rect};

/// Most pairs of segments whose bounds overlap that a planar map is built from. Neighbouring
/// segments of a path always overlap at their shared end; real art of 65,536 segments (the live
/// Pathfinder's cap) overlaps a few times per segment, far below this.
pub const MAX_SEGMENT_PAIRS: usize = 1 << 22;

/// Fenwick tree of counts over its slots.
struct Counts(Vec<isize>);

#[inline]
fn fenwick_step(i: usize) -> usize {
    // Same as `i.isolate_lowest_one()`; that API is still unstable on the workspace MSRV.
    1 << i.trailing_zeros()
}

impl Counts {
    fn add(&mut self, slot: usize, d: isize) {
        let mut i = slot + 1;
        while let Some(c) = self.0.get_mut(i) {
            *c += d;
            i += fenwick_step(i);
        }
    }

    /// Sum of the slots before `end`.
    fn before(&self, end: usize) -> isize {
        let (mut i, mut s) = (end.min(self.0.len().saturating_sub(1)), 0);
        while i > 0 {
            s += self.0.get(i).copied().unwrap_or(0);
            i -= fenwick_step(i);
        }
        s
    }
}

/// Do at most `cap` pairs of `boxes` overlap (touching counts)? A sweep in x keeps the open boxes
/// in two Fenwick trees over their y extents, so this costs O(n log n) however they lie, and stops
/// as soon as the count passes `cap`.
pub(crate) fn overlaps_within(boxes: &[Rect], cap: usize) -> bool {
    // `f64::total_cmp` puts -0.0 before +0.0 although they are equal, which would hide an overlap
    // between a box ending at -0.0 and one starting at +0.0; adding 0.0 maps -0.0 to +0.0.
    let boxes: Vec<Rect> = boxes.iter().map(|b| Rect::new(b.x0 + 0.0, b.y0 + 0.0, b.x1 + 0.0, b.y1 + 0.0)).collect();
    let boxes = boxes.as_slice();
    let mut ys: Vec<f64> = boxes.iter().flat_map(|b| [b.y0, b.y1]).collect();
    ys.sort_by(f64::total_cmp);
    ys.dedup_by(|a, b| a.total_cmp(b).is_eq());
    let below = |y: f64| ys.partition_point(|v| v.total_cmp(&y).is_lt());
    let upto = |y: f64| ys.partition_point(|v| v.total_cmp(&y).is_le());
    // (x, is end, box); starts sort before ends at the same x since touching boxes overlap.
    let mut events: Vec<(f64, bool, usize)> = boxes.iter().enumerate().flat_map(|(i, b)| [(b.x0, false, i), (b.x1, true, i)]).collect();
    events.sort_by(|a, b| a.0.total_cmp(&b.0).then(a.1.cmp(&b.1)));
    let mut by_y0 = Counts(vec![0; ys.len() + 1]);
    let mut by_y1 = Counts(vec![0; ys.len() + 1]);
    let (mut active, mut overlaps) = (0isize, 0usize);
    for (_, end, i) in events {
        let Some(b) = boxes.get(i) else { continue };
        if end {
            by_y0.add(below(b.y0), -1);
            by_y1.add(below(b.y1), -1);
            active -= 1;
            continue;
        }
        // Open boxes neither entirely above b nor entirely below it.
        let above = active - by_y0.before(upto(b.y1));
        let under = by_y1.before(below(b.y0));
        overlaps = overlaps.saturating_add(usize::try_from(active - above - under).unwrap_or(0));
        if overlaps > cap {
            return false;
        }
        by_y0.add(below(b.y0), 1);
        by_y1.add(below(b.y1), 1);
        active += 1;
    }
    true
}

/// Can a planar map be built of `paths` within `cap` segment pairs?
pub(crate) fn sweep_fits_within<'a>(paths: impl IntoIterator<Item = &'a BezPath>, cap: usize) -> bool {
    let boxes: Vec<Rect> = paths.into_iter().flat_map(|p| p.segments().map(|s| s.bounding_box())).collect();
    overlaps_within(&boxes, cap)
}

/// Can a planar map be built of `paths` within [`MAX_SEGMENT_PAIRS`]?
pub(crate) fn sweep_fits<'a>(paths: impl IntoIterator<Item = &'a BezPath>) -> bool {
    sweep_fits_within(paths, MAX_SEGMENT_PAIRS)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Pairs of overlapping boxes are counted exactly: a grid of n crossing bars makes n² pairs.
    #[test]
    fn crossing_bars_count_as_their_pairs() {
        let bars = |n: usize| -> Vec<Rect> {
            let across = (0..n).map(|i| Rect::new(0.0, i as f64 * 2.0, 100.0, i as f64 * 2.0 + 1.0));
            let down = (0..n).map(|i| Rect::new(i as f64 * 2.0, 0.0, i as f64 * 2.0 + 1.0, 100.0));
            across.chain(down).collect()
        };
        assert!(overlaps_within(&bars(10), 100));
        assert!(!overlaps_within(&bars(10), 99));
        // Disjoint boxes, many of them: none overlap.
        let apart: Vec<Rect> = (0..10_000).map(|i| Rect::new(i as f64 * 3.0, 0.0, i as f64 * 3.0 + 1.0, 1.0)).collect();
        assert!(overlaps_within(&apart, 0));
    }

    /// Boxes that touch at a signed zero still overlap: -0.0 and +0.0 are the same position.
    #[test]
    fn touching_at_a_signed_zero_counts_as_an_overlap() {
        let below = Rect::new(0.0, -1.0, 1.0, -0.0);
        let above = Rect::new(0.0, 0.0, 1.0, 1.0);
        assert!(!overlaps_within(&[below, above], 0));
        assert!(overlaps_within(&[below, above], 1));
    }
}
