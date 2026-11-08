// Carved from VectorCraft's crates/pathops (src/budget.rs) at storytold/vectorcraft ff9deee46d1c59dc9ecef0b668c25e57cd80b55e.
// Copyright (c) 2026 ArtCraft Team and the VectorCraft contributors. MIT OR Apache-2.0.

//! What the sweep that builds a planar map may be given.
//!
//! The topology it builds holds every crossing of the input's segments, and a crossing can only be
//! where two segments' bounds overlap (two cubics cross at most 9 times). The pairs of overlapping
//! bounds are counted before the sweep, exactly and in O(n log n), and past [`MAX_SEGMENT_PAIRS`]
//! the operation fails as [`crate::PathOpsError::TooComplex`]: one compact self-crossing path could
//! otherwise make millions of crossings, and an interactive caller rebuilt them on every redraw.
//!
//! The sweep's time is not bounded by its crossings alone: the sweep line is a list of the pieces
//! it crosses, and finding where a piece goes in scans that list. A row of shapes that share a
//! height (dots, glyphs on a line) cross the sweep line together without their bounds touching
//! one another, so [`MAX_SWEEP_LINE_PAIRS`] bounds the pairs of pieces whose heights overlap too.

//!
//! The bounds above, and those on fitting, joining and the shape builder, are the defaults of
//! [`Limits`], which an application can choose for the operations it runs ([`with_limits`]).

use std::cell::Cell;

use kurbo::{BezPath, ParamCurve, ParamCurveExtrema, PathSeg, Point, Rect};

use crate::work::{self, Work};

/// Most pairs of segments whose bounds overlap that a planar map is built from (the default of
/// [`Limits::segment_pairs`]). Neighbouring segments of a path always overlap at their shared end;
/// real art of 65,536 segments overlaps a few times per segment, far below this.
pub const MAX_SEGMENT_PAIRS: usize = 1 << 22;

/// Most pairs of pieces of the input, cut where their direction turns, whose heights overlap:
/// 268,435,456 (the default of [`Limits::sweep_line_pairs`]). The sweep takes 7 to 25 ns for each in a release build (a row of 5,500 dots at one
/// height, just under it, takes 3 s; a stroke crossing itself a lot 6 s). Past it an operation fails
/// as [`crate::PathOpsError::TooComplex`] before it sweeps.
pub const MAX_SWEEP_LINE_PAIRS: usize = 1 << 28;

/// Most searches of the sweep line that the events of a sweep make, as counted before it: the pairs
/// of segments whose bounds overlap (an upper bound on their crossings, and with them the ends of
/// segments; a pair of curves may cross up to nine times, so those count nine) times the most
/// pieces on the line at one height. 4,294,967,296. Each crossing finds two pieces on the line, and
/// each segment's end one, by a scan of it: a diagonal hatch of 400 strokes each way, a few thousand
/// elements, makes some billion of them and takes three seconds, and of curves it would take
/// many times that, with nothing else refusing it. Past the limit an operation fails as
/// [`crate::PathOpsError::TooComplex`] before it sweeps.
pub const MAX_SWEEP_EVENT_WORK: usize = {
    // Where `usize` is 32 bits (the web), the most it holds.
    let wanted = 1u64 << 32;
    if wanted > usize::MAX as u64 { usize::MAX } else { wanted as usize }
};

/// Most ends of segments at one point (within the sweep's tolerance, a point and its neighbours
/// counted as one): 512. The sweep takes about the fourth power of the number of segments that
/// meet at a point, which no other limit counts (a pie of k slices has 2k ends at its centre and k
/// segments' worth of everything else): 256 slices, 512 ends, take 0.5 to 0.9 s, 400 slices 3 s, and a
/// thousand pass the pair and height limits and would take minutes by that growth. Past it an operation fails as
/// [`crate::PathOpsError::TooComplex`] before it sweeps.
pub const MAX_ENDS_AT_A_POINT: usize = 512;

/// Most entries the windings of the arrangement of several inputs hold: 33,554,432 (16 bytes each,
/// half a gigabyte, and the sweep holds them twice). A side of a segment has an entry for each input
/// that covers it, so shapes nested n deep hold about 4n²: 2,000 nested squares 270 MB, 8,000 would
/// need four and a half gigabytes, which ends a web page that has 32-bit addresses. Counted before the sweep, from
/// the pairs of a segment's bounds and an input's bounds that overlap (an upper bound: an input's
/// bounds can hold what it does not cover). Past it an operation fails as
/// [`crate::PathOpsError::TooComplex`].
pub const MAX_COVERAGE_ENTRIES: usize = 1 << 25;

/// Most segments the contour passes of Merge, Divide, Trim and Crop walk in all: 33,554,432. The
/// classes of faces go into batches whose faces share no vertex, one pass of the whole arrangement for
/// each batch, and classes that meet at one point are each a batch: 208 shapes at a point beside
/// 40,000 squares take 2.2 s to merge and 3.3 s to divide, where the squares alone take 0.4 s (the
/// time grows with the shapes at the point). About 50 to 90 ns a segment in a pass. Counted before the first pass; past it the operation gives no shapes.
pub const MAX_PASS_WORK: usize = 1 << 25;

/// Most path elements (a move, a close, and a segment each) all the paths of one operation may
/// hold together: 524,288 (the default of [`Limits::elements`]). Every operation reads its input a few times and keeps several copies
/// of it (the sweep's segments cost a few hundred bytes each: a union of two paths of this size
/// takes about a gigabyte), so an input past this would take gigabytes more. Past it an operation
/// answers [`crate::PathOpsError::TooComplex`], an empty result, or the path unchanged, as its
/// documentation says.
pub const MAX_ELEMENTS: usize = 1 << 19;

/// What an operation may be given before it fails, or leaves its input as it is (each is
/// documented where it applies). [`Limits::default`] holds the constants of this crate,
/// [`MAX_ELEMENTS`] and the others, chosen so that an operation on input at them takes seconds and
/// not minutes and a few gigabytes at most. An application that knows its machine, or that runs
/// an operation off the interface thread where it can wait, chooses its own with [`with_limits`];
/// time and memory grow with a limit raised, and a limit lowered refuses more ordinary art.
///
/// The fields can be read and assigned (`let mut l = Limits::default(); l.elements = 1 << 20;`);
/// the type is `#[non_exhaustive]`, so a limit added later does not break a caller.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct Limits {
    /// Most path elements all the paths of one operation hold together ([`MAX_ELEMENTS`]); also
    /// the most elements of a stroke's outline.
    pub elements: usize,
    /// Most pairs of segments with overlapping bounds a planar map is built from ([`MAX_SEGMENT_PAIRS`]).
    pub segment_pairs: usize,
    /// The same for an offset ([`crate::MAX_OFFSET_SEGMENT_PAIRS`]).
    pub offset_segment_pairs: usize,
    /// Most pairs of pieces at one height on the sweep line ([`MAX_SWEEP_LINE_PAIRS`]).
    pub sweep_line_pairs: usize,
    /// Most segments the contour passes of a classified walk (Merge, Divide, Trim, Crop) walk in all:
    /// batches of classes times segments ([`MAX_PASS_WORK`]).
    pub pass_work: usize,
    /// Most segment ends at one point ([`MAX_ENDS_AT_A_POINT`]).
    pub ends_at_a_point: usize,
    /// Most entries the windings of an arrangement of several inputs hold, as counted before it
    /// ([`MAX_COVERAGE_ENTRIES`]).
    pub coverage_entries: usize,
    /// Most searches of the sweep line the crossings and ends of segments make, counted as the pairs
    /// of segments whose bounds overlap times the most pieces on the line at one height
    /// ([`MAX_SWEEP_EVENT_WORK`]).
    pub sweep_event_work: usize,
    /// Most segments [`crate::shape_builder`] sweeps into its map ([`crate::SHAPE_BUILDER_MAX_SEGMENTS`]).
    pub shape_builder_segments: usize,
    /// Most samples of one run of segments `simplify` fits ([`crate::MAX_FIT_POINTS`]).
    pub fit_points: usize,
    /// Most work `simplify` does in all ([`crate::FIT_WORK`]).
    pub fit_work: usize,
    /// Most work `join` does ([`crate::JOIN_WORK`]).
    pub join_work: usize,
    /// Most work `remove_redundant_points` does ([`crate::REDUNDANT_WORK`]).
    pub redundant_work: usize,
    /// Most flags a boolean operation sets to tell its predicate what covers the faces, and entries Divide
    /// and `regions` walk to list the inputs that cover each side of an edge ([`crate::MASK_WORK`]).
    pub mask_work: usize,
}

impl Limits {
    /// The defaults, as a constant.
    pub const DEFAULT: Self = Self {
        elements: MAX_ELEMENTS,
        segment_pairs: MAX_SEGMENT_PAIRS,
        offset_segment_pairs: crate::offset::MAX_OFFSET_SEGMENT_PAIRS,
        sweep_line_pairs: MAX_SWEEP_LINE_PAIRS,
        sweep_event_work: MAX_SWEEP_EVENT_WORK,
        pass_work: MAX_PASS_WORK,
        ends_at_a_point: MAX_ENDS_AT_A_POINT,
        coverage_entries: MAX_COVERAGE_ENTRIES,
        shape_builder_segments: crate::planar::SHAPE_BUILDER_MAX_SEGMENTS,
        fit_points: crate::edit::MAX_FIT_POINTS,
        fit_work: crate::edit::FIT_WORK,
        join_work: crate::edit::JOIN_WORK,
        redundant_work: crate::edit::REDUNDANT_WORK,
        mask_work: crate::boolean::MASK_WORK,
    };
}

impl Default for Limits {
    fn default() -> Self {
        Self::DEFAULT
    }
}

thread_local! {
    static LIMITS: Cell<Limits> = const { Cell::new(Limits::DEFAULT) };
}

/// The limits in force on this thread.
pub(crate) fn limits() -> Limits {
    LIMITS.with(Cell::get)
}

/// Run `f` with `limits` in force for every operation of this crate it calls on this thread; the
/// limits of the caller come back when `f` returns (also when it unwinds). Calls nest: the
/// innermost wins. Another thread keeps its own, the defaults unless it sets them.
///
/// ```
/// use craft_pathops::{FillRule, Limits, PathOpsError, try_area, with_limits};
///
/// let mut square = kurbo::BezPath::new();
/// square.move_to((0.0, 0.0));
/// square.line_to((10.0, 0.0));
/// square.line_to((10.0, 10.0));
/// square.line_to((0.0, 10.0));
/// square.close_path();
/// let mut small = Limits::default();
/// small.elements = 3;
/// assert_eq!(with_limits(small, || try_area(&square, FillRule::NonZero)), Err(PathOpsError::TooComplex));
/// assert_eq!(try_area(&square, FillRule::NonZero), Ok(100.0));
/// ```
pub fn with_limits<R>(limits: Limits, f: impl FnOnce() -> R) -> R {
    struct Restore(Limits);
    impl Drop for Restore {
        fn drop(&mut self) {
            LIMITS.with(|l| l.set(self.0));
        }
    }
    let _restore = Restore(LIMITS.with(|l| l.replace(limits)));
    f()
}

/// Do `paths` together hold at most the limit on elements? Costs one length read per path.
pub(crate) fn fits_elements<'a>(paths: impl IntoIterator<Item = &'a BezPath>) -> bool {
    let mut total = 0usize;
    for p in paths {
        total = total.saturating_add(p.elements().len());
        if total > limits().elements {
            return false;
        }
    }
    true
}

/// Fenwick tree of counts over its slots.
struct Counts(Vec<isize>);

#[inline]
fn fenwick_step(i: usize) -> usize {
    // Same as `i.isolate_lowest_one()`; that API is still unstable on this crate's MSRV.
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

/// Do at most `cap` pairs of `boxes` overlap (touching counts)?
#[cfg(test)]
pub(crate) fn overlaps_within(boxes: &[Rect], cap: usize) -> bool {
    overlap_pairs(boxes, cap).is_some()
}

/// How many pairs of `boxes` overlap (touching counts), if at most `cap`. A sweep in x keeps the
/// open boxes in two Fenwick trees over their y extents, so this costs O(n log n) however they
/// lie, and stops as soon as the count passes `cap`.
pub(crate) fn overlap_pairs(boxes: &[Rect], cap: usize) -> Option<usize> {
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
            return None;
        }
        by_y0.add(below(b.y0), 1);
        by_y1.add(below(b.y1), 1);
        active += 1;
    }
    Some(overlaps)
}

/// The heights `(lowest y, highest y)` of the pieces `seg` is cut into where its x or y turns
/// (the sweep cuts curves at its y extrema; the x ones are cut too here, which only counts more).
fn piece_heights(seg: &PathSeg, out: &mut Vec<(f64, f64)>) {
    let mut cuts = vec![0.0];
    let mut turns: Vec<f64> = seg.extrema().into_iter().filter(|t| *t > 0.0 && *t < 1.0).collect();
    turns.sort_by(f64::total_cmp);
    cuts.extend(turns);
    cuts.push(1.0);
    let ys: Vec<f64> = cuts.iter().map(|&t| seg.eval(t).y).collect();
    out.extend(ys.windows(2).map(|w| (w[0].min(w[1]), w[0].max(w[1]))));
}

/// Do at most `cap` pairs of `heights` overlap (touching counts)? The sweep line holds the pieces
/// that rise or fall through it, so those are paired with each other: each is counted with the ones
/// before it in order of `low` whose `high` reaches `low`, which two sorted lists and a binary
/// search give in O(n log n). A level piece (`low == high`) is not on the line: it is handled where
/// it lies, against the pieces that cross its height, so it is counted with those and not with other
/// level pieces (a rectangle with 20,000 collinear vertices on each side has 40,000 of them at two
/// heights, and sweeps in a blink).
pub(crate) fn heights_overlap_within(heights: &[(f64, f64)], cap: usize) -> bool {
    // Adding 0.0 maps -0.0 to +0.0, which `total_cmp` would otherwise put before it.
    let (tall, level): (Vec<_>, Vec<_>) = heights.iter().map(|h| (h.0 + 0.0, h.1 + 0.0)).partition(|h: &(f64, f64)| h.0 < h.1);
    let mut lows: Vec<f64> = tall.iter().map(|h| h.0).collect();
    let mut highs: Vec<f64> = tall.iter().map(|h| h.1).collect();
    lows.sort_by(f64::total_cmp);
    highs.sort_by(f64::total_cmp);
    let mut pairs = 0usize;
    for (before, low) in lows.iter().enumerate() {
        // Every piece that ended below `low` started below it too, so it is among those `before`.
        let ended = highs.partition_point(|h| h.total_cmp(low).is_lt());
        pairs = pairs.saturating_add(before.saturating_sub(ended));
        if pairs > cap {
            return false;
        }
    }
    // Each level piece, with the rising or falling ones whose heights hold its height.
    for (y, _) in level {
        let started = lows.partition_point(|l| l.total_cmp(&y).is_le());
        let ended = highs.partition_point(|h| h.total_cmp(&y).is_lt());
        pairs = pairs.saturating_add(started.saturating_sub(ended));
        if pairs > cap {
            return false;
        }
    }
    true
}

/// The most pieces of `heights` that rise or fall through one height (touching counts): the longest
/// the sweep line is. Pieces of no height are not on it.
pub(crate) fn peak_depth(heights: &[(f64, f64)]) -> usize {
    let mut lows: Vec<f64> = heights.iter().filter(|h| h.0 < h.1).map(|h| h.0 + 0.0).collect();
    let mut highs: Vec<f64> = heights.iter().filter(|h| h.0 < h.1).map(|h| h.1 + 0.0).collect();
    lows.sort_by(f64::total_cmp);
    highs.sort_by(f64::total_cmp);
    // The line is longest just after some piece starts: those started so far, less those ended below.
    lows.iter()
        .enumerate()
        .map(|(started, low)| started + 1 - highs.partition_point(|h| h.total_cmp(low).is_lt()).min(started + 1))
        .max()
        .unwrap_or(0)
}

/// The most ends of segments (`ends` are the first and last points of the segments, `extent` the
/// largest coordinate among them) that lie at one point, a point being a square of four of the
/// sweep's tolerances and the squares round it (so that a point on the edge between two squares
/// is not split between them).
pub(crate) fn most_ends_at_a_point(ends: &[Point], extent: f64) -> usize {
    let eps = (extent * f64::EPSILON * 64.0).max(1e-6);
    let cell = 4.0 * eps;
    let key = |p: &Point| ((p.x / cell).floor() as i64, (p.y / cell).floor() as i64);
    let mut counts: std::collections::HashMap<(i64, i64), usize> = std::collections::HashMap::new();
    for p in ends {
        *counts.entry(key(p)).or_default() += 1;
    }
    counts
        .keys()
        .map(|&(x, y)| {
            (x.saturating_sub(1)..=x.saturating_add(1))
                .flat_map(|i| (y.saturating_sub(1)..=y.saturating_add(1)).map(move |j| (i, j)))
                .filter_map(|c| counts.get(&c))
                .sum::<usize>()
        })
        .max()
        .unwrap_or(0)
}

/// Can a planar map be built of `paths` within `cap` segment pairs, [`MAX_SWEEP_LINE_PAIRS`] pairs of
/// pieces on the sweep line, and [`MAX_SWEEP_EVENT_WORK`] searches of it?
pub(crate) fn sweep_fits_within<'a>(paths: impl IntoIterator<Item = &'a BezPath>, cap: usize) -> bool {
    let (mut boxes, mut lines, mut curves, mut heights) = (Vec::new(), Vec::new(), Vec::new(), Vec::new());
    let (mut ends, mut extent, mut inputs): (Vec<Point>, f64, Vec<Rect>) = (Vec::new(), 0.0, Vec::new());
    for p in paths {
        work::add(Work::Visited, p.elements().len());
        let mut whole: Option<Rect> = None;
        for s in p.segments() {
            let b = s.bounding_box();
            boxes.push(b);
            if matches!(s, PathSeg::Line(_)) {
                lines.push(b);
            } else {
                curves.push(b);
            }
            whole = Some(whole.map_or(b, |w| w.union(b)));
            let (first, last) = (s.start(), s.end());
            ends.extend([first, last]);
            extent = extent.max(first.x.abs()).max(first.y.abs()).max(last.x.abs()).max(last.y.abs());
            piece_heights(&s, &mut heights);
        }
        inputs.extend(whole);
    }
    // Segments that meet at one point cost about the fourth power of their number in the sweep.
    if most_ends_at_a_point(&ends, extent) > limits().ends_at_a_point {
        return false;
    }
    let Some(pairs) = overlap_pairs(&boxes, cap) else { return false };
    if !heights_overlap_within(&heights, limits().sweep_line_pairs) {
        return false;
    }
    // The crossings: a pair of lines crosses once, a pair of a line and a curve up to three times, a
    // pair of curves up to nine; each pair counts for as many (at most), so the pairs of bounds
    // that overlap are counted by the kinds of segment in them.
    let (Some(line_pairs), Some(curve_pairs)) = (overlap_pairs(&lines, cap), overlap_pairs(&curves, cap)) else { return false };
    let mixed = pairs.saturating_sub(line_pairs).saturating_sub(curve_pairs);
    // In 64 bits: the products pass what a 32-bit `usize` holds.
    let crossings = (line_pairs as u64).saturating_add((mixed as u64).saturating_mul(3)).saturating_add((curve_pairs as u64).saturating_mul(9));
    if crossings.saturating_mul(peak_depth(&heights) as u64) > limits().sweep_event_work as u64 {
        return false;
    }
    coverage_entries_within(&boxes, pairs, &inputs, limits().coverage_entries)
}

/// Can the windings of the sweep of these inputs be held in `cap` entries? Each side of each segment
/// has an entry for every input that covers it, and an input can only cover a segment whose bounds
/// lie within its own, so the pairs of a segment's bounds and an input's bounds that overlap (each
/// segment with its own input, too) bound them, two sides to a segment. Nested squares of
/// n inputs hold about 4n² of them. `pairs` are those of the segments' bounds among themselves.
/// Two inputs or fewer hold two at most for each segment, and are not counted.
fn coverage_entries_within(segments: &[Rect], pairs: usize, inputs: &[Rect], cap: usize) -> bool {
    if inputs.len() <= 2 {
        return true;
    }
    let mut all = segments.to_vec();
    all.extend_from_slice(inputs);
    let (Some(everything), Some(among_inputs)) = (overlap_pairs(&all, usize::MAX), overlap_pairs(inputs, usize::MAX)) else { return false };
    let across = everything.saturating_sub(pairs).saturating_sub(among_inputs);
    let entries = (across as u64).saturating_mul(2);
    work::add(Work::CoverageEntries, usize::try_from(entries).unwrap_or(usize::MAX));
    entries <= cap as u64
}

/// Can a planar map be built of `paths` within [`MAX_SEGMENT_PAIRS`] (and [`MAX_SWEEP_LINE_PAIRS`])?
pub(crate) fn sweep_fits<'a>(paths: impl IntoIterator<Item = &'a BezPath>) -> bool {
    sweep_fits_within(paths, limits().segment_pairs)
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

    /// The segment cap counts every element, and sums over the paths of an operation.
    #[test]
    fn the_segment_cap_counts_elements_over_all_paths() {
        let path = |n: usize| {
            let mut bp = BezPath::new();
            bp.move_to((0.0, 0.0));
            for i in 1..n {
                bp.line_to((i as f64, 0.0));
            }
            bp
        };
        assert!(fits_elements([&path(MAX_ELEMENTS)]));
        assert!(!fits_elements([&path(MAX_ELEMENTS + 1)]));
        assert!(fits_elements([&path(MAX_ELEMENTS / 2), &path(MAX_ELEMENTS / 2)]));
        assert!(!fits_elements([&path(MAX_ELEMENTS / 2), &path(MAX_ELEMENTS / 2 + 1)]));
        assert!(fits_elements(std::iter::empty()));
    }

    /// The pairs of `heights` that overlap, one by one, but for two level pieces.
    fn brute_force_pairs(heights: &[(f64, f64)]) -> usize {
        let mut pairs = 0;
        for (i, a) in heights.iter().enumerate() {
            for b in heights.iter().skip(i + 1) {
                let both_level = a.0 == a.1 && b.0 == b.1;
                pairs += usize::from(!both_level && a.0 <= b.1 && b.0 <= a.1);
            }
        }
        pairs
    }

    /// The heights of the pieces of every segment of `path`.
    fn heights_of(path: &BezPath) -> Vec<(f64, f64)> {
        let mut heights = Vec::new();
        for s in path.segments() {
            piece_heights(&s, &mut heights);
        }
        heights
    }

    /// Pairs of heights are counted exactly: n equal ones make n(n-1)/2, touching ones count, and
    /// ones that are apart make none.
    #[test]
    fn pairs_of_overlapping_heights_are_counted_exactly() {
        let same: Vec<(f64, f64)> = vec![(1.0, 2.0); 50];
        assert!(heights_overlap_within(&same, 50 * 49 / 2));
        assert!(!heights_overlap_within(&same, 50 * 49 / 2 - 1));
        let touching = [(0.0, 1.0), (1.0, 2.0), (2.0, 3.0)];
        assert!(heights_overlap_within(&touching, 2) && !heights_overlap_within(&touching, 1));
        let apart: Vec<(f64, f64)> = (0..10_000).map(|i| (f64::from(i) * 3.0, f64::from(i) * 3.0 + 1.0)).collect();
        assert!(heights_overlap_within(&apart, 0));
        // A signed zero is no gap.
        assert!(!heights_overlap_within(&[(-1.0, -0.0), (0.0, 1.0)], 0));
        // Random heights, against the pairs counted one by one.
        let mut seed = 0x9e37_79b9_7f4a_7c15_u64;
        let mut next = || {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            (seed % 1000) as f64 / 10.0
        };
        let random: Vec<(f64, f64)> = (0..300)
            .map(|_| {
                let low = next();
                (low, low + next() / 10.0)
            })
            .collect();
        let pairs = brute_force_pairs(&random);
        assert!(pairs > 1000, "{pairs}");
        assert!(heights_overlap_within(&random, pairs) && !heights_overlap_within(&random, pairs - 1));
        // With level pieces among them (some at heights other pieces start or end at).
        let mut mixed = random.clone();
        mixed.extend((0..100).map(|k| {
            let y = if k % 2 == 0 { next() } else { random[k].0 };
            (y, y)
        }));
        let pairs = brute_force_pairs(&mixed);
        assert!(pairs > brute_force_pairs(&random), "{pairs}");
        assert!(heights_overlap_within(&mixed, pairs) && !heights_overlap_within(&mixed, pairs - 1));
    }

    /// Level pieces are not paired with each other, only with the pieces that cross their height:
    /// a rectangle with 20,000 collinear vertices on top and bottom has 40,000 level pieces at two
    /// heights, and sweeps; one rising piece through both heights is counted once per level piece.
    #[test]
    fn level_pieces_are_counted_with_the_pieces_across_them_and_not_with_each_other() {
        let level: Vec<(f64, f64)> = (0..40_000).map(|i| if i % 2 == 0 { (0.0, 0.0) } else { (1.0, 1.0) }).collect();
        assert!(heights_overlap_within(&level, 0));
        let mut crossed = level.clone();
        crossed.push((-1.0, 2.0));
        assert!(heights_overlap_within(&crossed, 40_000) && !heights_overlap_within(&crossed, 39_999));
        // A piece that only touches a level one counts, one that stops short does not.
        assert!(!heights_overlap_within(&[(0.0, 0.0), (0.0, 1.0)], 0));
        assert!(!heights_overlap_within(&[(1.0, 1.0), (0.0, 1.0)], 0));
        assert!(heights_overlap_within(&[(1.5, 1.5), (0.0, 1.0)], 0));
        assert!(heights_overlap_within(&[(-0.5, -0.5), (0.0, 1.0)], 0));
        // The rectangle itself, end to end.
        use crate::{FillRule, PathOpsError};
        let mut b = BezPath::new();
        b.move_to((0.0, 0.0));
        for i in 1..=20_000 {
            b.line_to((f64::from(i), 0.0));
        }
        b.line_to((20_000.0, 1.0));
        for i in (0..20_000).rev() {
            b.line_to((f64::from(i), 1.0));
        }
        b.close_path();
        assert!(sweep_fits(std::iter::once(&b)));
        work::reset();
        let area = crate::try_area(&b, FillRule::NonZero);
        assert!(matches!(area, Ok(a) if (a - 20_000.0).abs() < 1e-6), "{area:?}");
        assert_ne!(area, Err(PathOpsError::TooComplex));
        assert_eq!(work::take(Work::Sweeps), 1);
    }

    /// A row of 30 lenses at one height: every piece crosses the lines through the others, so the
    /// pairs counted are those of the pieces, and a column of them (heights apart) makes none.
    #[test]
    fn a_row_of_lenses_counts_the_pairs_of_its_pieces_and_a_column_none() {
        let row = heights_of(&crate::testutil::row_of_lenses(30));
        // Each lens is two cubics, each cut once where it turns: 4 pieces, all at the same height
        // (the upper two from 20 to 23, the lower two from 17 to 20).
        assert_eq!(row.len(), 4 * 30);
        let pairs = brute_force_pairs(&row);
        assert!(pairs > 3000, "{pairs}");
        assert!(heights_overlap_within(&row, pairs) && !heights_overlap_within(&row, pairs - 1));
        let column = heights_of(&crate::testutil::column_of_lenses(30));
        // Each lens's own pieces meet at its ends; the lenses are 10 apart and 6 high: no more.
        let own = brute_force_pairs(&column);
        assert_eq!(own, 30 * 6, "{own}");
    }

    /// A row of lenses is refused before anything is swept once its pieces' pairs pass the cap, though no
    /// two bounds touch (MAX_SEGMENT_PAIRS counts none of them); the same number in a column is swept.
    #[test]
    fn a_long_row_of_lenses_is_refused_before_the_sweep() {
        use crate::{FillRule, PathOpsError};
        // 4 pieces a lens: 7,000 lenses make 28,000 pieces, 392 million pairs, past 268 million.
        let row = crate::testutil::row_of_lenses(7_000);
        work::reset();
        assert_eq!(crate::try_area(&row, FillRule::NonZero), Err(PathOpsError::TooComplex));
        assert_eq!(crate::try_unite_all(&[(&row, FillRule::NonZero)]), Err(PathOpsError::TooComplex));
        assert_eq!(work::take(Work::Sweeps), 0, "refused before the sweep was built");
        // The same number of shapes stacked in a column are no trouble to the sweep line.
        let column = crate::testutil::column_of_lenses(1_000);
        assert!(crate::try_area(&column, FillRule::NonZero).is_ok());
        assert_eq!(work::take(Work::Sweeps), 1);
        // And no bounds of the row touch: the bound pairs are few (each lens meets itself).
        let boxes: Vec<Rect> = row.segments().map(|s| s.bounding_box()).collect();
        assert!(overlaps_within(&boxes, 2 * 7_000 + 7_000));
    }

    /// Past [`MAX_ELEMENTS`] an operation does no work on the path at all: no element is read,
    /// copied or swept, whether the path has half a million elements more or millions.
    #[test]
    fn millions_of_elements_are_refused_as_quickly() {
        use crate::{BoolOp, FillRule, PathfinderOp, Shape};
        let rule = FillRule::NonZero;
        let mut zigzag = BezPath::new();
        zigzag.move_to((0.0, 0.0));
        for i in 0..MAX_ELEMENTS {
            zigzag.line_to((i as f64, if i % 2 == 0 { 7.0 } else { 0.0 }));
        }
        let mut millions = zigzag.clone();
        for _ in 0..4 {
            millions.extend(zigzag.iter());
        }
        assert!(millions.elements().len() > 2_000_000);
        let ok = crate::testutil::rect(0.0, 0.0, 10.0, 10.0);
        work::reset();
        for huge in [&zigzag, &millions] {
            let shapes = [Shape::new(ok.clone(), rule, 0), Shape::new(huge.clone(), rule, 1)];
            assert!(crate::try_area(huge, rule).is_err());
            assert!(crate::try_boolean(huge, rule, &ok, rule, BoolOp::Union, 0.01).is_err());
            assert!(crate::try_normalize(huge, rule).is_err());
            assert!(crate::try_unite_all(&[(huge, rule)]).is_err());
            assert!(crate::try_offset_path(huge, 1.0, crate::Join::Round, 4.0).is_err());
            assert!(crate::try_outline_stroke(huge, 1.0, crate::Cap::Butt, crate::Join::Round, 4.0).is_err());
            assert!(crate::pathfinder(PathfinderOp::Divide, &shapes).is_empty());
            assert!(crate::regions(&shapes).is_empty());
            assert!(crate::live_paint(&shapes).0.is_empty());
            assert!(crate::shape_builder(&shapes, true).regions.is_empty());
            assert_eq!(crate::interior_point(huge), None);
        }
        // Not an element visited, not a sweep, not a curve looked up, not a stroke made.
        for what in [Work::Visited, Work::Sweeps, Work::IndexCandidates, Work::StrokeOutput, Work::Winding] {
            assert_eq!(work::take(what), 0, "{what:?}");
        }
    }

    /// A limit raised lets through what the default refuses, before anything is swept: the count is
    /// all that is asked here. (A row of 7,000 lenses is past the default; sweeping it takes seconds.)
    #[test]
    fn a_raised_limit_lets_through_what_the_default_refuses() {
        let row = crate::testutil::row_of_lenses(7_000);
        assert!(!sweep_fits([&row]));
        let raised = Limits { sweep_line_pairs: usize::MAX, ..Limits::default() };
        assert!(with_limits(raised, || sweep_fits([&row])));
        let long = {
            let mut zigzag = BezPath::new();
            zigzag.move_to((0.0, 0.0));
            for i in 1..=MAX_ELEMENTS {
                zigzag.line_to((i as f64, 0.0));
            }
            zigzag
        };
        assert!(!fits_elements([&long]));
        let roomy = Limits { elements: MAX_ELEMENTS + 1, ..Limits::default() };
        assert!(with_limits(roomy, || fits_elements([&long])));
        // And the count of segment pairs: a square's four sides meet at their ends.
        let square = crate::testutil::rect(0.0, 0.0, 10.0, 10.0);
        assert!(sweep_fits([&square]));
        let none = Limits { segment_pairs: 0, ..Limits::default() };
        assert!(!with_limits(none, || sweep_fits([&square])));
    }

    /// The pieces on the line at once, counted by the sorted lists, are those a count one height at
    /// a time gives; level pieces are on no line.
    #[test]
    fn the_peak_depth_is_the_most_pieces_through_one_height() {
        assert_eq!(peak_depth(&[]), 0);
        assert_eq!(peak_depth(&[(1.0, 1.0), (2.0, 2.0)]), 0);
        assert_eq!(peak_depth(&[(0.0, 1.0), (2.0, 3.0)]), 1);
        assert_eq!(peak_depth(&[(0.0, 1.0), (1.0, 2.0)]), 2, "touching counts");
        assert_eq!(peak_depth(&[(0.0, 10.0), (1.0, 2.0), (3.0, 4.0), (1.5, 3.5)]), 3);
        assert_eq!(peak_depth(&[(-1.0, -0.0), (0.0, 1.0)]), 2, "a signed zero is no gap");
        let mut seed = 0x2545_f491_4f6c_dd1d_u64;
        let mut next = || {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            (seed % 1000) as f64 / 10.0
        };
        for _ in 0..20 {
            let heights: Vec<(f64, f64)> = (0..120)
                .map(|_| {
                    let low = next();
                    (low, low + next() / 5.0)
                })
                .collect();
            // At the low of some piece, the pieces that hold it.
            let brute = heights
                .iter()
                .filter(|h| h.0 < h.1)
                .map(|h| heights.iter().filter(|g| g.0 < g.1 && g.0 <= h.0 && h.0 <= g.1).count())
                .max()
                .unwrap_or(0);
            assert_eq!(peak_depth(&heights), brute);
        }
    }

    /// Pairs of boxes are counted exactly, not only against a cap.
    #[test]
    fn overlap_pairs_counts_the_pairs() {
        let bars: Vec<Rect> = (0..10)
            .map(|i| Rect::new(0.0, f64::from(i) * 2.0, 100.0, f64::from(i) * 2.0 + 1.0))
            .chain((0..10).map(|i| Rect::new(f64::from(i) * 2.0, 0.0, f64::from(i) * 2.0 + 1.0, 100.0)))
            .collect();
        assert_eq!(overlap_pairs(&bars, usize::MAX), Some(100));
        assert_eq!(overlap_pairs(&bars, 100), Some(100));
        assert_eq!(overlap_pairs(&bars, 99), None);
        assert_eq!(overlap_pairs(&[], 0), Some(0));
    }

    /// The pairs of overlapping bounds of two lines, of a line and a curve, and of two curves, and the
    /// most pieces at one height.
    fn event_counts(path: &BezPath) -> (usize, usize, usize, usize) {
        let (mut boxes, mut lines, mut curves, mut heights) = (Vec::new(), Vec::new(), Vec::new(), Vec::new());
        for s in path.segments() {
            boxes.push(s.bounding_box());
            if matches!(s, PathSeg::Line(_)) {
                lines.push(s.bounding_box());
            } else {
                curves.push(s.bounding_box());
            }
            piece_heights(&s, &mut heights);
        }
        let all = overlap_pairs(&boxes, usize::MAX).unwrap_or(0);
        let (ll, cc) = (overlap_pairs(&lines, usize::MAX).unwrap_or(0), overlap_pairs(&curves, usize::MAX).unwrap_or(0));
        (ll, all - ll - cc, cc, peak_depth(&heights))
    }

    /// The crossings the budget counts: a pair of lines once, a line and a curve three times, two curves nine.
    fn weighted(counts: (usize, usize, usize, usize)) -> usize {
        let (lines, mixed, curves, depth) = counts;
        (lines + 3 * mixed + 9 * curves) * depth
    }

    /// Crossings that the other budgets pass but whose searches of the sweep line are many are
    /// refused before the sweep: 480 strokes each way slanting across each other are 3.8 thousand
    /// elements, under four million overlapping bounds and a few million pieces at one height, yet
    /// every crossing scans a line of thousands (some billion of those, seconds).
    #[test]
    fn a_dense_diagonal_hatch_is_refused_before_it_is_swept() {
        use crate::{FillRule, PathOpsError};
        let hatch = crate::testutil::diagonals(480);
        let (pairs, mixed, curve_pairs, depth) = event_counts(&hatch);
        assert_eq!(mixed, 0);
        assert!(pairs <= MAX_SEGMENT_PAIRS, "{pairs} overlapping bounds");
        assert!(heights_overlap_within(&heights_of(&hatch), MAX_SWEEP_LINE_PAIRS), "the pieces at one height are within their limit");
        assert_eq!(curve_pairs, 0);
        assert!(depth > 1_500 && pairs * depth > MAX_SWEEP_EVENT_WORK, "{pairs} pairs, {depth} on the line");
        let unlimited = Limits { sweep_event_work: usize::MAX, ..Limits::default() };
        assert!(with_limits(unlimited, || sweep_fits([&hatch])), "only the searches of the line refuse it");
        work::reset();
        assert!(!sweep_fits([&hatch]));
        assert_eq!(crate::try_area(&hatch, FillRule::NonZero), Err(PathOpsError::TooComplex));
        assert_eq!(crate::try_unite_all(&[(&hatch, FillRule::NonZero)]), Err(PathOpsError::TooComplex));
        assert_eq!(work::take(Work::Sweeps), 0, "refused before the sweep was built");
        // A smaller hatch is swept, and the product is exactly the limit: one under it refuses, at it passes.
        let small = crate::testutil::diagonals(40);
        assert!(sweep_fits([&small]));
        assert!(crate::try_area(&small, FillRule::NonZero).is_ok());
        let (pairs, _, _, depth) = event_counts(&small);
        assert!(pairs * depth > 0);
        assert!(with_limits(Limits { sweep_event_work: pairs * depth, ..Limits::default() }, || sweep_fits([&small])));
        assert!(!with_limits(Limits { sweep_event_work: pairs * depth - 1, ..Limits::default() }, || sweep_fits([&small])));
    }

    /// A pair of curves can cross nine times, a pair of lines once: curves count for more.
    #[test]
    fn pairs_of_curves_count_nine_crossings_and_pairs_of_lines_one() {
        // The same strokes as lines and as cubics bowed a little: the same bounds overlapping, but for the bow.
        let strokes = |bow: f64| {
            let mut b = BezPath::new();
            for i in 0..60 {
                let x = f64::from(i) * 5.0;
                for flip in [1.0, -1.0] {
                    b.move_to((x, 0.0));
                    b.curve_to((x + flip * 60.0 + bow, 100.0), (x + flip * 120.0 - bow, 200.0), (x + flip * 180.0, 300.0));
                    b.line_to((x + flip * 180.0 + 1.0, 300.0));
                    b.curve_to((x + flip * 120.0 - bow + 1.0, 200.0), (x + flip * 60.0 + bow + 1.0, 100.0), (x + 1.0, 0.0));
                    b.close_path();
                }
            }
            b
        };
        let bowed = strokes(30.0);
        let counts = event_counts(&bowed);
        let (lines, mixed, curves, _) = counts;
        assert!(curves > lines && mixed > 0, "{lines} pairs of lines, {mixed} mixed, {curves} of curves");
        let wanted = weighted(counts);
        assert!(wanted > 4 * (lines + mixed + curves) * counts.3, "{wanted} against {} unweighted", (lines + mixed + curves) * counts.3);
        assert!(with_limits(Limits { sweep_event_work: wanted, ..Limits::default() }, || sweep_fits([&bowed])));
        assert!(!with_limits(Limits { sweep_event_work: wanted - 1, ..Limits::default() }, || sweep_fits([&bowed])));
    }

    /// A line and a curve can cross three times: those pairs count three, between a line and a line one and two curves nine.
    #[test]
    fn a_pair_of_a_line_and_a_curve_counts_three_crossings() {
        // Curved strokes across straight ones: every pair of bounds overlapping is a line and a curve, but the stroke's own.
        let mut b = BezPath::new();
        for i in 0..40 {
            let y = f64::from(i) * 3.0;
            b.move_to((0.0, y));
            b.line_to((300.0, y + 0.5));
            b.line_to((300.0, y + 1.0));
            b.line_to((0.0, y + 1.0));
            b.close_path();
        }
        for i in 0..40 {
            let x = f64::from(i) * 7.0;
            b.move_to((x, -10.0));
            b.curve_to((x + 20.0, 40.0), (x - 20.0, 80.0), (x, 130.0));
            b.curve_to((x - 19.0, 80.0), (x + 21.0, 40.0), (x, -10.0));
            b.close_path();
        }
        let counts = event_counts(&b);
        let (lines, mixed, curves, depth) = counts;
        assert!(mixed > lines && mixed > curves && depth > 40, "{lines} pairs of lines, {mixed} mixed, {curves} of curves, {depth} deep");
        // Weighted three, the mixed pairs make more than counted once and less than counted nine times.
        let wanted = weighted(counts);
        assert!(wanted > (lines + mixed + curves) * depth && wanted < 9 * (lines + mixed + curves) * depth);
        assert!(with_limits(Limits { sweep_event_work: wanted, ..Limits::default() }, || sweep_fits([&b])));
        assert!(!with_limits(Limits { sweep_event_work: wanted - 1, ..Limits::default() }, || sweep_fits([&b])));
    }

    /// A pie of k slices: the segments of all the slices that meet at its centre.
    fn pie(k: usize) -> BezPath {
        let mut p = BezPath::new();
        for i in 0..k {
            let (a, c) = (std::f64::consts::TAU * i as f64 / k as f64, std::f64::consts::TAU * (i + 1) as f64 / k as f64);
            p.move_to((0.0, 0.0));
            p.line_to((100.0 * a.cos(), 100.0 * a.sin()));
            p.line_to((100.0 * c.cos(), 100.0 * c.sin()));
            p.close_path();
        }
        p
    }

    fn ends_of(path: &BezPath) -> (Vec<Point>, f64) {
        let ends: Vec<Point> = path.segments().flat_map(|s| [s.start(), s.end()]).collect();
        let extent = ends.iter().fold(0.0_f64, |m, p| m.max(p.x.abs()).max(p.y.abs()));
        (ends, extent)
    }

    /// The most ends at one point are counted, at the sweep's tolerance and across the edge between two squares of it.
    #[test]
    fn the_most_ends_at_a_point_are_counted() {
        for k in [3usize, 20, 100] {
            let (ends, extent) = ends_of(&pie(k));
            assert_eq!(most_ends_at_a_point(&ends, extent), 2 * k, "a pie of {k} slices");
        }
        // Four squares that share a corner: each has two ends there.
        let mut four = BezPath::new();
        for (x, y) in [(0.0, 0.0), (-1.0, 0.0), (0.0, -1.0), (-1.0, -1.0)] {
            four.extend(crate::testutil::rect(x, y, x + 1.0, y + 1.0).iter());
        }
        let (ends, extent) = ends_of(&four);
        assert_eq!(most_ends_at_a_point(&ends, extent), 8);
        // Points a hair apart (inside the tolerance) are one point, however the squares of the hash fall.
        let near: Vec<Point> = (0..50).map(|i| Point::new(f64::from(i) * 1e-8, f64::from(i % 3) * 1e-8)).collect();
        assert_eq!(most_ends_at_a_point(&near, 1.0), 50);
        let across: Vec<Point> = (0..50).map(|i| Point::new(-1e-6 + f64::from(i) * 4e-8, 0.0)).collect();
        assert_eq!(most_ends_at_a_point(&across, 1.0), 50, "points either side of an edge of the squares");
        // Points further apart than that are not.
        let apart: Vec<Point> = (0..50).map(|i| Point::new(f64::from(i) * 1e-3, 0.0)).collect();
        assert_eq!(most_ends_at_a_point(&apart, 1.0), 1);
        assert_eq!(most_ends_at_a_point(&[], 0.0), 0);
    }

    /// 512 ends at a point pass and 514 do not (a pie of 256 slices and of 257), before the sweep.
    #[test]
    fn a_pie_of_many_slices_is_refused_before_the_sweep_past_the_cap() {
        use crate::{FillRule, PathOpsError};
        assert_eq!(MAX_ENDS_AT_A_POINT, 512);
        let (at, over) = (pie(256), pie(257));
        assert!(sweep_fits([&at]) && !sweep_fits([&over]));
        work::reset();
        assert_eq!(crate::try_area(&over, FillRule::NonZero), Err(PathOpsError::TooComplex));
        assert_eq!(crate::try_unite_all(&[(&over, FillRule::NonZero)]), Err(PathOpsError::TooComplex));
        assert_eq!(work::take(Work::Sweeps), 0, "refused before the sweep was built");
        // The cap is the limit: one slice more with a limit of 514, one fewer end with 513.
        assert!(with_limits(Limits { ends_at_a_point: 514, ..Limits::default() }, || sweep_fits([&over])));
        assert!(!with_limits(Limits { ends_at_a_point: 513, ..Limits::default() }, || sweep_fits([&over])));
        // Ends that are not at one point are no matter: the same slices side by side.
        let mut spread = BezPath::new();
        for i in 0..300 {
            let x = f64::from(i) * 200.0;
            spread.extend(crate::testutil::rect(x, 0.0, x + 10.0, 10.0).iter());
        }
        assert!(sweep_fits([&spread]));
    }

    /// Nested squares: n inputs hold 4n(n+1) entries in the windings of the arrangement, counted before the sweep.
    #[test]
    fn nested_squares_are_counted_in_entries_before_the_sweep() {
        use crate::{FillRule, PathOpsError, PathfinderOp, Shape};
        let nested = |n: usize| -> Vec<BezPath> { (1..=n).map(|k| crate::testutil::rect(-(k as f64), -(k as f64), k as f64, k as f64)).collect() };
        for n in [20usize, 50] {
            let squares = nested(n);
            work::reset();
            assert!(sweep_fits(squares.iter()));
            let counted = work::take(Work::CoverageEntries);
            assert_eq!(counted, 4 * n * (n + 1), "{n} nested squares");
            // It is an upper bound on the entries the windings hold.
            let arr = crate::boolean::Arrangement::new(squares.iter().map(|s| (s.clone(), FillRule::NonZero)).collect()).unwrap();
            let held: usize = arr
                .top
                .segment_indices()
                .map(|s| arr.top.winding_clockwise(s.first_half()).len() + arr.top.winding_counter_clockwise(s.first_half()).len())
                .sum();
            assert!(held <= counted && held * 2 > counted, "{held} held, {counted} counted");
        }
        // The limit is on that count: at it the operation is made, one under it is refused before the sweep.
        let shapes: Vec<Shape> = nested(40).into_iter().enumerate().map(|(i, p)| Shape::new(p, FillRule::NonZero, i as u64)).collect();
        let exact = 4 * 40 * 41;
        let at = Limits { coverage_entries: exact, ..Limits::default() };
        assert_eq!(with_limits(at, || crate::pathfinder(PathfinderOp::Exclude, &shapes)).len(), 1);
        let under = Limits { coverage_entries: exact - 1, ..Limits::default() };
        work::reset();
        assert!(with_limits(under, || crate::pathfinder(PathfinderOp::Exclude, &shapes)).is_empty());
        let paths: Vec<(&BezPath, FillRule)> = shapes.iter().map(|s| (&s.path, FillRule::NonZero)).collect();
        assert_eq!(with_limits(under, || crate::try_boolean_n(&paths, |m| m[0])), Err(PathOpsError::TooComplex));
        assert_eq!(work::take(Work::Sweeps), 0, "refused before the sweep was built");
        // By default 2,000 nested squares (4 x 2,000 x 2,001 = 16 million) are within it, and 3,000 (36 million) are not.
        let (some, many) = (nested(2_000), nested(3_000));
        assert!(sweep_fits(some.iter()));
        work::reset();
        assert!(!sweep_fits(many.iter()));
        assert_eq!(work::take(Work::CoverageEntries), 4 * 3_000 * 3_001);
        // Two inputs hold two entries for each side at most: not counted.
        work::reset();
        assert!(sweep_fits(nested(2).iter()));
        assert_eq!(work::take(Work::CoverageEntries), 0);
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
