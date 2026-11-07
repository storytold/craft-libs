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

use kurbo::{BezPath, ParamCurve, ParamCurveExtrema, PathSeg, Rect};

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
}

impl Limits {
    /// The defaults, as a constant.
    pub const DEFAULT: Self = Self {
        elements: MAX_ELEMENTS,
        segment_pairs: MAX_SEGMENT_PAIRS,
        offset_segment_pairs: crate::offset::MAX_OFFSET_SEGMENT_PAIRS,
        sweep_line_pairs: MAX_SWEEP_LINE_PAIRS,
        shape_builder_segments: crate::planar::SHAPE_BUILDER_MAX_SEGMENTS,
        fit_points: crate::edit::MAX_FIT_POINTS,
        fit_work: crate::edit::FIT_WORK,
        join_work: crate::edit::JOIN_WORK,
        redundant_work: crate::edit::REDUNDANT_WORK,
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

/// Can a planar map be built of `paths` within `cap` segment pairs and
/// [`MAX_SWEEP_LINE_PAIRS`] pairs of pieces on the sweep line?
pub(crate) fn sweep_fits_within<'a>(paths: impl IntoIterator<Item = &'a BezPath>, cap: usize) -> bool {
    let (mut boxes, mut heights) = (Vec::new(), Vec::new());
    for p in paths {
        work::add(Work::Visited, p.elements().len());
        for s in p.segments() {
            boxes.push(s.bounding_box());
            piece_heights(&s, &mut heights);
        }
    }
    overlaps_within(&boxes, cap) && heights_overlap_within(&heights, limits().sweep_line_pairs)
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

    /// Boxes that touch at a signed zero still overlap: -0.0 and +0.0 are the same position.
    #[test]
    fn touching_at_a_signed_zero_counts_as_an_overlap() {
        let below = Rect::new(0.0, -1.0, 1.0, -0.0);
        let above = Rect::new(0.0, 0.0, 1.0, 1.0);
        assert!(!overlaps_within(&[below, above], 0));
        assert!(overlaps_within(&[below, above], 1));
    }
}
