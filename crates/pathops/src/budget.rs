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

use kurbo::{BezPath, CubicBez, Line, ParamCurve, ParamCurveExtrema, PathSeg, Point, Rect};

use crate::crossings::{Curve, cubics_cross_at_most, line_cubic_cross_at_most};
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
/// pieces on the line at one height. 2^32, 4,294,967,296, where `usize` holds it; on a target where
/// `usize` is 32 bits (the web) it is the most `usize` holds, 4,294,967,295 (the count itself is
/// made in 64 bits, so a larger one is refused there). Each crossing finds two pieces on the line, and
/// each segment's end one, by a scan of it: a diagonal hatch of 400 strokes each way, a few thousand
/// elements, makes some billion of them and takes three seconds, and of curves it would take
/// many times that, with nothing else refusing it. Past the limit an operation fails as
/// [`crate::PathOpsError::TooComplex`] before it sweeps.
pub const MAX_SWEEP_EVENT_WORK: usize = {
    // Where `usize` is 32 bits (the web), the most it holds.
    let wanted = 1u64 << 32;
    if wanted > usize::MAX as u64 { usize::MAX } else { wanted as usize }
};

/// Most searches of the sweep line that the crossings of segments make, as counted before the sweep from the
/// pairs that really cross: 2^30, 1,073,741,824. [`MAX_SWEEP_EVENT_WORK`] counts every pair of
/// segments whose bounds overlap as crossing, which is far more than cross where strokes slant across each
/// other (a hatch of 400 and 400 crosses in a fifth of the pairs counted), and is priced for that; where
/// every pair of lines whose bounds overlap does cross, one polygon whose edges all cross one another (a star
/// of n points joined to the ones nearly opposite crosses in about n²/2 places), the same count is about
/// seven times as much time and a good deal more memory a unit: 2,001 edges, which pass the other limits,
/// take 19 s and 3.5 GB. The places where segments cross, found by testing the pairs whose bounds overlap (no more than
/// [`MAX_SEGMENT_PAIRS`]; [`MAX_REAL_CROSSINGS`] says how, for lines, curves and for lines nearly parallel), times the most
/// pieces on the sweep line, are held to this: 1,001 edges take 2.4 s and 0.9 GB, and 1,291 (the most) 4.3 s. The places are
/// counted whenever the most the count could come to (the pairs of bounds, priced by their kinds as there, a line pair one
/// and a curve pair eighteen) times the pieces on the line could pass the limit, or the pairs could make more than
/// [`MAX_REAL_CROSSINGS`]. Past it an operation fails as [`crate::PathOpsError::TooComplex`] before it sweeps. The default of
/// [`Limits::crossed_work`].
pub const MAX_CROSSED_WORK: usize = 1 << 30;

/// Most places where the segments of one sweep really cross one another, whatever the pieces on the sweep line:
/// 2^20, 1,048,576 (the default of [`Limits::real_crossings`]). The sweep's time and memory follow its crossings
/// themselves, about 3.5 microseconds and 1.6 kilobytes for each, and [`MAX_CROSSED_WORK`] counts them times the
/// pieces on the line, which is little where shapes that cross a great deal are stacked so that their heights do not
/// overlap: 206 stars of 201 points, each edge to the vertex nearly opposite, each star above and beside the last, hold
/// 4.1 million crossings with 201 pieces on the line (8.3e8 searches, under both limits on them, and 4.1 million pairs of
/// bounds, under the limit on those) and took 14.7 s and 6.5 GB. Curves crossed as much (100 such stars of bowed cubics
/// took 14.8 s and 3.7 GB), for only the crossings of lines were counted. The places where segments cross are counted by
/// testing the pairs of bounds that overlap, each pair for at most as many places as it can have, and never fewer than it
/// has: two lines are tested, and cross once or not (two nearly parallel ones, whose crossing the rounding of the test cannot place, count one unless the ends of one lie on one side of the other, with room for the rounding); a line and a cubic count the changes of sign of the distances of the
/// cubic's control points from the line (three at the most); two cubics count one if the directions of their tangents
/// are apart (as lines, and by pieces of an eighth where the whole curves are not), and nine if they are not (see
/// `crossings.rs`: two arcs that cross twice both have a tangent along the chord between the crossings), and none where
/// the pair meets only at an end it shares and the two leave it apart. Each place counts for one where both are lines and
/// for two where a curve is one of them, and a cubic that may curl into a loop counts two more for itself: the sweep
/// takes 3 microseconds and 1.6 KB for a place where lines cross, and about twice the time (and half the memory) where
/// curves do. They are counted whenever the most the count can come to, priced so, could pass the limit. Past it an
/// operation fails as [`crate::PathOpsError::TooComplex`] before it sweeps.
pub const MAX_REAL_CROSSINGS: usize = 1 << 20;

/// Most ends of segments at one point (within the sweep's tolerance, a point and its neighbours
/// counted as one): 512. The sweep takes about the fourth power of the number of segments that
/// meet at a point, which no other limit counts (a pie of k slices has 2k ends at its centre and k
/// segments' worth of everything else): 256 slices, 512 ends, take 0.5 to 0.9 s, 400 slices 3 s, and a
/// thousand pass the pair and height limits and would take minutes by that growth. Past it an operation fails as
/// [`crate::PathOpsError::TooComplex`] before it sweeps.
pub const MAX_ENDS_AT_A_POINT: usize = 512;

/// Most work of the ends of segments at the points where they meet, summed over the points: 4,000,000,000 (the default of
/// [`Limits::ends_work`]), in the nanoseconds the sweep takes. [`MAX_ENDS_AT_A_POINT`] holds the ends at any one point, and the
/// sweep takes time for each point that grows with the number of ends there (about the fourth power of it, as measured:
/// pies of 64, 96, 128, 192 and 256 slices, each with 2 ends at its centre for each slice, take 12, 40, 76, 270 and 1,300 ms; the cost
/// at a point of `n` ends is taken as the greater of `5.6 n^3` and `0.019 n^4` nanoseconds, which is that, and what 100
/// polygons of 20 sides one on another take (0.9 s) too), which the limit on any one point does not add up: 8 pies of 256
/// slices (512 ends at each centre, 8,192 elements) stacked on a diagonal took 15 s, and 30 of them 66 s and 2.7 GB. The
/// cost of every point (the ends within a point's square and the squares round it, `most_ends_at_a_point`) is added, and
/// past the limit an operation fails as [`crate::PathOpsError::TooComplex`] before it sweeps: 3 such pies are made in 3.3
/// to 3.7 s, 4 are refused. A fan of 256 slices alone takes 1.3 s.
pub const MAX_ENDS_WORK: usize = 4_000_000_000;

/// Most lines crossing at one point, as work: 16,777,216. The sweep takes about the cube of the number
/// of lines that cross at one point (a pie of k slices has them end there, [`MAX_ENDS_AT_A_POINT`]; these
/// pass through with no end), which the count of crossings does not see: 150 lines through one point take
/// 0.2 s, 300 take 1.7 s, 450 take 8 s, and a thousand pass the pair and event limits and would take a
/// minute and a half. A line that crosses `m` others at one point counts `m`², so `c` lines through a
/// point count `c` (`c` - 1)², and 256 is the most that pass (about a second). Found before the sweep by
/// computing the crossings of the lines whose bounds overlap, when that many could cost more than the
/// limit; past it an operation fails as [`crate::PathOpsError::TooComplex`]. Where two lines cross is known to the rounding of the
/// arithmetic over the sine of the angle between them (crossings are taken as one point when they lie within the tolerance and
/// that of each), and two lines too near parallel to place a crossing at all count as crossing somewhere along their length,
/// which adds them to the work of both lines, so 450 lines through one point at angles of 1e-13 are held as the 450 they are,
/// and so are lines that lie on one line and overlap (250 on a diagonal took 13 s, and 500 over a minute and a half), except level
/// ones, which are not on the sweep line. Crossings of curves are not looked for.
pub const MAX_CROSSING_WORK: usize = 1 << 24;

/// Most entries the windings of the arrangement of several inputs hold: 33,554,432 (about 13 bytes
/// each, 450 MB). A side of a piece of a segment has an entry for each input that covers it. Shapes
/// nested n deep hold about 4n²: 2,000 nested squares 270 MB, 8,000 would need four and a half
/// gigabytes, which ends a web page that has 32-bit addresses; and the sweep cuts each segment where
/// others cross it, each piece keeping its own list, so n nested squares crossed by m thin bars hold
/// about 4n²m (300 and 300: 1.3 GB). An estimate made before the sweep from the bounds: the pieces
/// of a segment are one more than the segments whose bounds overlap it (apart from those that follow
/// it in its path), a curve three more, each piece having an entry on each side for every input whose
/// bounds overlap the segment's. Exact for lines that cross, more than the entries for lines whose
/// bounds overlap without crossing, and less for curves that cross each other more than once. Past it
/// an operation fails as [`crate::PathOpsError::TooComplex`].
pub const MAX_COVERAGE_ENTRIES: usize = 1 << 25;

/// Most segments the contour passes of Merge, Divide, Trim and Crop walk in all: 33,554,432. The
/// classes of faces go into batches whose faces share no vertex, one pass of the whole arrangement for
/// each batch, and classes that meet at one point are each a batch: 208 shapes at a point beside
/// 40,000 squares take 2.2 s to merge and 3.3 s to divide, where the squares alone take 0.4 s (the
/// time grows with the shapes at the point). About 50 to 90 ns a segment in a pass. Counted before the first pass; past it the operation gives no shapes.
pub const MAX_PASS_WORK: usize = 1 << 25;

/// Most segments a planar map ([`crate::live_paint`], [`crate::shape_builder`], and Divide with an open
/// path) winds points against, in all, to find what fills its faces and which face each hole is in:
/// 67,108,864. A face is tested against each subpath of each input whose bounds hold a point of it,
/// and a hole against each face whose bounds hold it, so contours nested n deep cost about the square
/// of n: rectangles nested 4,000 deep in one shape about 64 million (each of four segments). Each
/// test is charged before it is made; past the limit the map is not built (`live_paint` gives
/// nothing, and `shape_builder` and Divide fall back to the filled areas, as for any map that cannot be
/// built: an answer, but not the arrangement asked for, its faces unsplit by the open paths).
pub const MAX_FILL_WORK: usize = 1 << 26;

/// Most steps of the searches a Pathfinder Outline and an inset make after their sweeps: 2^30,
/// 1,073,741,824. The sweep is bounded by the limits above; what is done with its output is not by them. A
/// Pathfinder Outline cuts each segment where junctions lie on it, and finds them among the junctions inside
/// the segment's bounds (a grid of cutters makes a million junctions, and every long arch above it holds all
/// of them in its bounds, where the curve is nowhere near): a junction looked at costs a step, a block of 64
/// of them one, and the nearest point of a curve to a junction 64 (about 0.3 microseconds
/// in a release build, against 5 nanoseconds for a step). An inset checks that every segment of its subpaths
/// is covered by the boundary of their fill, a search of the same kind for each segment of the boundary
/// and each segment of the subpaths near it. About 5 seconds of the slowest of them. Past it a Pathfinder
/// Outline fails as [`crate::PathOpsError::TooComplex`] (no shapes, for [`crate::pathfinder`]), and an inset
/// takes the boundary of the fill, which is right for any input and differs from the subpaths themselves only in
/// how it is cut. The default of [`Limits::search_work`].
pub const MAX_SEARCH_WORK: usize = 1 << 30;

/// What the nearest point of a curve to a point costs, in the steps of [`MAX_SEARCH_WORK`].
pub(crate) const NEAREST_STEPS: usize = 64;

/// What a search after a sweep may still spend ([`Limits::search_work`]), as [`crate::planar`]'s fill
/// work is: each step is charged before it is made, and the one that does not fit empties the budget.
pub(crate) struct SearchBudget(Cell<usize>);

impl SearchBudget {
    pub(crate) fn new() -> Self {
        Self(Cell::new(limits().search_work))
    }

    /// Takes `n` steps, or empties the budget and says there were not that many left.
    pub(crate) fn charge(&self, n: usize) -> bool {
        match self.0.get().checked_sub(n) {
            Some(left) => {
                self.0.set(left);
                work::add(Work::SearchSteps, n);
                true
            }
            None => {
                self.0.set(0);
                false
            }
        }
    }
}

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
    /// Most segments a planar map winds points against to find the fills of its faces and the faces
    /// its holes are in ([`MAX_FILL_WORK`]).
    pub fill_work: usize,
    /// Most segment ends at one point ([`MAX_ENDS_AT_A_POINT`]).
    pub ends_at_a_point: usize,
    /// Most work of the ends of segments at all the points where they meet, added up: nanoseconds, by the cost of a point of `n`
    /// ends, the greater of `5.6 n^3` and `0.019 n^4` ([`MAX_ENDS_WORK`]).
    pub ends_work: usize,
    /// Most work of the lines that cross at one point: the square of the others each crosses there,
    /// summed over the lines and the points ([`MAX_CROSSING_WORK`]).
    pub crossing_work: usize,
    /// Most entries the windings of an arrangement of several inputs hold, as estimated before it
    /// from the pieces the sweep cuts the segments into ([`MAX_COVERAGE_ENTRIES`]).
    pub coverage_entries: usize,
    /// Most searches of the sweep line the crossings and ends of segments make, counted as the pairs
    /// of segments whose bounds overlap times the most pieces on the line at one height
    /// ([`MAX_SWEEP_EVENT_WORK`]: 2^32, or the most `usize` holds where that is less).
    pub sweep_event_work: usize,
    /// Most searches of the sweep line the crossings of segments make, counted from the pairs that
    /// really cross times the most pieces on the line at one height ([`MAX_CROSSED_WORK`]).
    pub crossed_work: usize,
    /// Most places where the segments cross one another, counted (as a bound, never below the number there are) by testing
    /// the pairs of segments whose bounds overlap, whatever the pieces on the sweep line ([`MAX_REAL_CROSSINGS`]).
    pub real_crossings: usize,
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
    /// Most steps of the searches a Pathfinder Outline and an inset make on curves after their sweeps
    /// ([`MAX_SEARCH_WORK`]).
    pub search_work: usize,
}

impl Limits {
    /// The defaults, as a constant.
    pub const DEFAULT: Self = Self {
        elements: MAX_ELEMENTS,
        segment_pairs: MAX_SEGMENT_PAIRS,
        offset_segment_pairs: crate::offset::MAX_OFFSET_SEGMENT_PAIRS,
        sweep_line_pairs: MAX_SWEEP_LINE_PAIRS,
        sweep_event_work: MAX_SWEEP_EVENT_WORK,
        crossed_work: MAX_CROSSED_WORK,
        real_crossings: MAX_REAL_CROSSINGS,
        pass_work: MAX_PASS_WORK,
        fill_work: MAX_FILL_WORK,
        ends_at_a_point: MAX_ENDS_AT_A_POINT,
        ends_work: MAX_ENDS_WORK,
        crossing_work: MAX_CROSSING_WORK,
        coverage_entries: MAX_COVERAGE_ENTRIES,
        shape_builder_segments: crate::planar::SHAPE_BUILDER_MAX_SEGMENTS,
        fit_points: crate::edit::MAX_FIT_POINTS,
        fit_work: crate::edit::FIT_WORK,
        join_work: crate::edit::JOIN_WORK,
        redundant_work: crate::edit::REDUNDANT_WORK,
        mask_work: crate::boolean::MASK_WORK,
        search_work: MAX_SEARCH_WORK,
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
    /// Adds `d` to the count of `slot`.
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
/// largest coordinate among all the points of the paths, control points too, for those set the sweep's
/// tolerance, [`crate::boolean::sweep_tolerance`]) that lie at one point, a point being a square of
/// four of the sweep's tolerances and the squares round it (so that a point on the edge between two
/// squares is not split between them).
#[cfg(test)]
pub(crate) fn most_ends_at_a_point(ends: &[Point], extent: f64) -> usize {
    ends_at_points(ends, extent).0
}

/// The most ends at one point (`most_ends_at_a_point`), and the work of them at all the points added up, in the nanoseconds
/// the sweep takes ([`MAX_ENDS_WORK`]): for each square that holds an end, the cost of the ends in it and the squares round it,
/// in proportion to the ends in the square (a cluster across the edge of two squares, or of four, is counted once).
pub(crate) fn ends_at_points(ends: &[Point], extent: f64) -> (usize, u64) {
    let eps = crate::boolean::sweep_tolerance(extent);
    let cell = 4.0 * eps;
    let key = |p: &Point| ((p.x / cell).floor() as i64, (p.y / cell).floor() as i64);
    let mut counts: std::collections::HashMap<(i64, i64), usize> = std::collections::HashMap::new();
    for p in ends {
        *counts.entry(key(p)).or_default() += 1;
    }
    let (mut most, mut work) = (0usize, 0u64);
    for &(x, y) in counts.keys() {
        let n = (x.saturating_sub(1)..=x.saturating_add(1))
            .flat_map(|i| (y.saturating_sub(1)..=y.saturating_add(1)).map(move |j| (i, j)))
            .filter_map(|c| counts.get(&c))
            .sum::<usize>();
        most = most.max(n);
        // The cost of the ends round this square, shared among the squares by their ends, so that a cluster across two or four
        // of them is counted once, as its whole cost, and not for each.
        let own = counts.get(&(x, y)).copied().unwrap_or(0) as f64;
        let n = n as f64;
        work = work.saturating_add(((5.6 * n * n * n).max(0.019 * n * n * n * n) * own / n) as u64);
    }
    (most, work)
}

/// For each query `(a, b)`, how many of the points `(p, q)` have `p < a` and `q < b`. O(n log n): the
/// points go into a Fenwick tree over their `q` in order of `p`, as the queries are met in order of `a`.
fn dominated(points: &[(f64, f64)], queries: &[(f64, f64)]) -> Vec<usize> {
    let mut qs: Vec<f64> = points.iter().map(|p| p.1).collect();
    qs.sort_by(f64::total_cmp);
    qs.dedup_by(|a, b| a.total_cmp(b).is_eq());
    let mut by_p: Vec<(f64, usize)> = points.iter().map(|p| (p.0, qs.partition_point(|q| q.total_cmp(&p.1).is_lt()))).collect();
    by_p.sort_by(|a, b| a.0.total_cmp(&b.0));
    let mut order: Vec<usize> = (0..queries.len()).collect();
    order.sort_by(|&a, &b| queries.get(a).map(|q| q.0).unwrap_or(0.0).total_cmp(&queries.get(b).map(|q| q.0).unwrap_or(0.0)));
    let mut tree = Counts(vec![0; qs.len() + 1]);
    let mut out = vec![0; queries.len()];
    let mut taken = 0;
    for qi in order {
        let Some(&(a, b)) = queries.get(qi) else { continue };
        while let Some(&(p, rank)) = by_p.get(taken) {
            if p.total_cmp(&a).is_lt() {
                tree.add(rank, 1);
                taken += 1;
            } else {
                break;
            }
        }
        let below = qs.partition_point(|q| q.total_cmp(&b).is_lt());
        if let Some(slot) = out.get_mut(qi) {
            *slot = usize::try_from(tree.before(below)).unwrap_or(0);
        }
    }
    out
}

/// For each of `queries`, how many of `targets` its bounds overlap (touching counts), in O((n + m)
/// log (n + m)) however they lie: the targets wholly to the left, right, below or above it are
/// subtracted from all of them, and those counted twice (left and below, and so on, each a count of
/// the points below it in both coordinates) added back.
pub(crate) fn overlap_degrees(queries: &[Rect], targets: &[Rect]) -> Vec<usize> {
    // Adding 0.0 maps -0.0 to +0.0, which `total_cmp` would otherwise put before it; `0.0 - v` negates likewise.
    let norm = |b: &Rect| Rect::new(b.x0 + 0.0, b.y0 + 0.0, b.x1 + 0.0, b.y1 + 0.0);
    let (q, t): (Vec<Rect>, Vec<Rect>) = (queries.iter().map(norm).collect(), targets.iter().map(norm).collect());
    let neg = |v: f64| 0.0 - v;
    let sorted = |f: fn(&Rect) -> f64| {
        let mut v: Vec<f64> = t.iter().map(f).collect();
        v.sort_by(f64::total_cmp);
        v
    };
    let (x1s, x0s, y1s, y0s) = (sorted(|b| b.x1), sorted(|b| b.x0), sorted(|b| b.y1), sorted(|b| b.y0));
    // Left and below: a target's right edge before the query's left edge, its top before the query's bottom.
    let left_below = dominated(&t.iter().map(|b| (b.x1, b.y1)).collect::<Vec<_>>(), &q.iter().map(|b| (b.x0, b.y0)).collect::<Vec<_>>());
    let left_above = dominated(&t.iter().map(|b| (b.x1, neg(b.y0))).collect::<Vec<_>>(), &q.iter().map(|b| (b.x0, neg(b.y1))).collect::<Vec<_>>());
    let right_below = dominated(&t.iter().map(|b| (neg(b.x0), b.y1)).collect::<Vec<_>>(), &q.iter().map(|b| (neg(b.x1), b.y0)).collect::<Vec<_>>());
    let right_above =
        dominated(&t.iter().map(|b| (neg(b.x0), neg(b.y0))).collect::<Vec<_>>(), &q.iter().map(|b| (neg(b.x1), neg(b.y1))).collect::<Vec<_>>());
    let total = t.len();
    q.iter()
        .enumerate()
        .map(|(i, b)| {
            let left = x1s.partition_point(|v| v.total_cmp(&b.x0).is_lt());
            let right = total - x0s.partition_point(|v| v.total_cmp(&b.x1).is_le());
            let below = y1s.partition_point(|v| v.total_cmp(&b.y0).is_lt());
            let above = total - y0s.partition_point(|v| v.total_cmp(&b.y1).is_le());
            let corners: usize = [&left_below, &left_above, &right_below, &right_above].iter().map(|c| c.get(i).copied().unwrap_or(0)).sum();
            (total + corners).saturating_sub(left + right + below + above)
        })
        .collect()
}

/// Calls `found(i, j)` for each pair of `boxes` that overlap (touching counts), `i` the one that
/// starts later in x, and gives up (false) as soon as more than `cap` have been found. A sweep in
/// x keeps the open boxes in a tree over their order by bottom edge, each node the highest top of
/// those below it, so the boxes that overlap a new one in y are walked down to, and the work is
/// O((n + pairs) log n).
fn each_overlap(boxes: &[Rect], cap: usize, mut found: impl FnMut(usize, usize)) -> bool {
    let boxes: Vec<Rect> = boxes.iter().map(|b| Rect::new(b.x0 + 0.0, b.y0 + 0.0, b.x1 + 0.0, b.y1 + 0.0)).collect();
    let mut by_bottom: Vec<usize> = (0..boxes.len()).collect();
    by_bottom.sort_by(|&a, &b| boxes.get(a).map(|r| r.y0).unwrap_or(0.0).total_cmp(&boxes.get(b).map(|r| r.y0).unwrap_or(0.0)));
    let bottoms: Vec<f64> = by_bottom.iter().filter_map(|&i| boxes.get(i).map(|r| r.y0)).collect();
    let mut rank = vec![0usize; boxes.len()];
    for (r, &i) in by_bottom.iter().enumerate() {
        if let Some(slot) = rank.get_mut(i) {
            *slot = r;
        }
    }
    let leaves = boxes.len().next_power_of_two().max(1);
    // The highest top of the open boxes in each node's range of ranks (negative infinity: none).
    let mut top = vec![f64::NEG_INFINITY; 2 * leaves];
    let mut events: Vec<(f64, bool, usize)> = boxes.iter().enumerate().flat_map(|(i, b)| [(b.x0, false, i), (b.x1, true, i)]).collect();
    events.sort_by(|a, b| a.0.total_cmp(&b.0).then(a.1.cmp(&b.1)));
    let set = |top: &mut Vec<f64>, r: usize, value: f64| {
        let mut node = leaves + r;
        if let Some(t) = top.get_mut(node) {
            *t = value;
        }
        while node > 1 {
            node /= 2;
            let high = top.get(2 * node).copied().unwrap_or(f64::NEG_INFINITY).max(top.get(2 * node + 1).copied().unwrap_or(f64::NEG_INFINITY));
            if let Some(t) = top.get_mut(node) {
                *t = high;
            }
        }
    };
    let mut count = 0usize;
    let mut stack: Vec<(usize, usize, usize)> = Vec::new();
    for (_, end, i) in events {
        let (Some(b), Some(&r)) = (boxes.get(i), rank.get(i)) else { continue };
        if end {
            set(&mut top, r, f64::NEG_INFINITY);
            continue;
        }
        // The open boxes whose bottom is not above this one's top (a prefix of the ranks) and whose top
        // is not below its bottom.
        let upto = bottoms.partition_point(|y| y.total_cmp(&b.y1).is_le());
        stack.clear();
        stack.push((1, 0, leaves));
        while let Some((node, lo, hi)) = stack.pop() {
            if lo >= upto || top.get(node).is_none_or(|t| t.total_cmp(&b.y0).is_lt()) {
                continue;
            }
            if hi - lo == 1 {
                if let Some(&other) = by_bottom.get(lo) {
                    count += 1;
                    if count > cap {
                        return false;
                    }
                    found(i, other);
                }
                continue;
            }
            let mid = (lo + hi) / 2;
            stack.push((2 * node + 1, mid, hi));
            stack.push((2 * node, lo, mid));
        }
        set(&mut top, r, b.y1);
    }
    true
}

/// A segment: the line (an index into `line_segs`) or the curve (into `curve_segs`) it is.
#[derive(Clone, Copy)]
enum Kind {
    Line(usize),
    Curve(usize),
}

/// What the check of a sweep reads of the segments of all its paths.
#[derive(Default)]
struct Segments {
    boxes: Vec<Rect>,
    /// The bounds of the lines, and the lines, in the same order.
    lines: Vec<Rect>,
    line_segs: Vec<Line>,
    curves: Vec<Rect>,
    curve_segs: Vec<PathSeg>,
    /// For each segment: which line or curve it is, and for a curve how many curves of its path it
    /// touches at an end (they share the end, so their bounds overlap without their crossing).
    kind: Vec<Kind>,
    touching_curves: Vec<usize>,
    /// How many pairs of segments of a path touch at an end, which is no crossing, and how many of
    /// them are pairs of lines.
    touching_pairs: usize,
    touching_lines: usize,
    heights: Vec<(f64, f64)>,
    ends: Vec<Point>,
    /// The largest coordinate of any point of any element, control points too (what sets the tolerance).
    extent: f64,
    /// The bounds of each path.
    inputs: Vec<Rect>,
}

impl Segments {
    /// Reads the segments of `p`: their bounds, kinds, ends and the heights of their pieces, which of them
    /// follow one another, the largest coordinate, and the bounds of the whole path.
    fn add_path(&mut self, p: &BezPath) {
        work::add(Work::Visited, p.elements().len());
        for el in p.elements() {
            for pt in crate::boolean::el_points(el) {
                self.extent = self.extent.max(pt.x.abs()).max(pt.y.abs());
            }
        }
        let mut whole: Option<Rect> = None;
        // The subpath being read (its first segment, where that starts, its length) and the segment before.
        let mut sub: Option<(usize, Point, usize)> = None;
        let mut last: Option<(usize, Point)> = None;
        for s in p.segments() {
            let idx = self.boxes.len();
            let b = s.bounding_box();
            self.boxes.push(b);
            match s {
                PathSeg::Line(l) => {
                    self.kind.push(Kind::Line(self.line_segs.len()));
                    self.lines.push(b);
                    self.line_segs.push(l);
                }
                _ => {
                    self.kind.push(Kind::Curve(self.curve_segs.len()));
                    self.curves.push(b);
                    self.curve_segs.push(s);
                }
            }
            self.touching_curves.push(0);
            whole = Some(whole.map_or(b, |w| w.union(b)));
            let (first, end) = (s.start(), s.end());
            self.ends.extend([first, end]);
            piece_heights(&s, &mut self.heights);
            match last {
                Some((prev, at)) if at == first => {
                    self.touch(prev, idx);
                    if let Some(sub) = sub.as_mut() {
                        sub.2 += 1;
                    }
                }
                _ => {
                    self.close(sub.take(), last);
                    sub = Some((idx, first, 1));
                }
            }
            last = Some((idx, end));
        }
        self.close(sub.take(), last);
        self.inputs.extend(whole);
    }

    /// Counts the segments `a` and `b` as touching where one ends and the other starts.
    fn touch(&mut self, a: usize, b: usize) {
        if let (Some(Kind::Curve(_)), Some(Kind::Curve(_))) = (self.kind.get(a), self.kind.get(b)) {
            for i in [a, b] {
                if let Some(t) = self.touching_curves.get_mut(i) {
                    *t += 1;
                }
            }
        }
        self.touching_pairs += 1;
        if let (Some(Kind::Line(_)), Some(Kind::Line(_))) = (self.kind.get(a), self.kind.get(b)) {
            self.touching_lines += 1;
        }
    }

    /// The pieces more than one a segment is cut into by its own turns: none for a line; a curve is cut
    /// where its x or y turns, and where it crosses itself.
    fn turns(&self, i: usize) -> u64 {
        if matches!(self.kind.get(i), Some(Kind::Curve(_))) { 3 } else { 0 }
    }

    /// A subpath that comes back to where it started: its last segment touches its first.
    fn close(&mut self, sub: Option<(usize, Point, usize)>, last: Option<(usize, Point)>) {
        if let (Some((first, start, count)), Some((tail, at))) = (sub, last)
            && count >= 3
            && at == start
        {
            self.touch(tail, first);
        }
    }
}

/// What is known of where two lines cross.
enum Cross {
    /// They do not cross away from their ends (or they meet at an end they share exactly, where they meet and nowhere else).
    No,
    /// They cross, at these distances along `a` and along `b` from the start of each, each known to within `err` (the rounding
    /// of the arithmetic, which is large where the lines are nearly parallel).
    At { s: f64, r: f64, err: f64 },
    /// They may cross, and where is not known: the arithmetic cannot tell them from lines that are parallel, and neither
    /// has both ends strictly on one side of the other.
    Somewhere,
}

/// Where the lines `a` and `b` cross, away from both their ends (by more than `slack`: segments that end at a point are counted
/// as ends, `most_ends_at_a_point`); a pair is never taken for one that does not cross where it may:
///
/// - Two lines that are the same line (both ends shared exactly) cross nowhere: what they cost is in their ends.
/// - Two lines that share one end exactly meet there and nowhere else, unless they lie on one line (the rounding cannot tell them
///   from lines that do, the last item): a fan of 250 segments from one point along one line, which share their start exactly and
///   overlap one another, took 11 s in the sweep.
/// - Two level lines (the heights of each end equal) are parallel, and lie along the height of the sweep line and not across it:
///   pieces of no height are not on the line, and what they cost is in the pairs of bounds and the searches ([`MAX_SWEEP_EVENT_WORK`]),
///   as it was when 250 overlapping on one line, which the sweep takes 30 ms for, were not counted. (Lines that lie on one line
///   at any other angle are, and 250 of them took 13 s, and 500 over a minute and a half.)
/// - Two lines whose directions cross to more than the rounding of that product have a crossing whose distances along them are
///   found to within `err`, `16 ε` of the largest of the lengths and the distance between their starts over the sine of the
///   angle; a crossing as near the ends as that is counted as away from them.
/// - Two nearly parallel lines (the product is no more than the rounding of it, so that where they cross, or whether, is not
///   told) cross somewhere unless both ends of one lie strictly on one side of the other (by more than the rounding of the
///   distance), which is what two lines apart are, or unless the parts of them that lie along one another are no longer than
///   the slack, which is what two lines that touch end to end are.
fn cross_of(a: &Line, b: &Line, slack: f64) -> Cross {
    let shared = a.p0 == b.p0 || a.p0 == b.p1 || a.p1 == b.p0 || a.p1 == b.p1;
    // The same line twice (or the closing segment of a subpath of one line, which comes back over it): its cost is in the ends it
    // has at its two points ([`MAX_ENDS_WORK`] prices stacked copies: 200 squares one on another took 4 s), and they cross nowhere.
    if (a.p0 == b.p0 && a.p1 == b.p1) || (a.p0 == b.p1 && a.p1 == b.p0) {
        return Cross::No;
    }
    if a.p0.y == a.p1.y && b.p0.y == b.p1.y {
        return Cross::No;
    }
    let (d, e, w) = (a.p1 - a.p0, b.p1 - b.p0, b.p0 - a.p0);
    let (la, lb) = (d.hypot(), e.hypot());
    // A line no longer than two tolerances is an end, and has no inside to cross in.
    if !(la > 2.0 * slack && lb > 2.0 * slack) {
        return Cross::No;
    }
    let den = d.cross(e);
    if !den.is_finite() {
        return Cross::No;
    }
    let reach = w.hypot().max(la).max(lb);
    if den.abs() <= 16.0 * f64::EPSILON * la * lb {
        let tol = 16.0 * f64::EPSILON * reach;
        let apart = |u: f64, v: f64| (u > tol && v > tol) || (u < -tol && v < -tol);
        let (a0, a1) = (d.cross(b.p0 - a.p0) / la, d.cross(b.p1 - a.p0) / la);
        let (b0, b1) = (e.cross(a.p0 - b.p0) / lb, e.cross(a.p1 - b.p0) / lb);
        if apart(a0, a1) || apart(b0, b1) {
            return Cross::No;
        }
        // On one line, to the rounding, what they have in common is the part of `b` that lies along `a` (an end they share exactly
        // is no more than where that part begins): where it is no more than the slack they meet at an end, as lines that touch end
        // to end do, and nowhere else.
        let (t0, t1) = (w.dot(d) / la, (w + e).dot(d) / la);
        let along = t0.max(t1).min(la) - t0.min(t1).max(0.0);
        return if along + 2.0 * tol <= slack { Cross::No } else { Cross::Somewhere };
    }
    // Lines that are not on one line meet in one point, which is the end they share if they share one.
    if shared {
        return Cross::No;
    }
    let err = 16.0 * f64::EPSILON * reach * la * lb / den.abs();
    let (t, u) = (w.cross(e) / den, w.cross(d) / den);
    let (s, r) = (t * la, u * lb);
    if s > slack - err && s < la - slack + err && r > slack - err && r < lb - slack + err { Cross::At { s, r, err } } else { Cross::No }
}

/// Are two lines of lengths `la` and `lb`, whose directions cross to `den`, parallel (to within the
/// noise of the arithmetic: a crossing found of lines nearly on one line is anywhere along it)?
fn parallel(den: f64, la: f64, lb: f64) -> bool {
    !den.is_finite() || den.abs() <= 1e-10 * la * lb
}

/// How many places the segments of `seg` cross one another, at the most, away from the ends they share: each pair of
/// segments whose bounds overlap is tested (`cap` of them at most, the limit on pairs) and counts the places it crosses
/// at, or at the most it can cross at where that is not known, never fewer than there are. Two lines are tested, and
/// cross once or not at all. A line and a cubic count by [`crate::crossings::line_cubic_cross_at_most`] (three at the
/// most), two cubics by [`crate::crossings::cubics_cross_at_most`] (nine), and a cubic that may loop one more for itself.
/// Each place where a curve is one of the two counts for two, for the sweep takes about twice as long over it. `None`
/// when there are more pairs than `cap`.
fn real_crossings(seg: &Segments, slack: f64, cap: usize) -> Option<u64> {
    let curves: Vec<Curve> = seg
        .curve_segs
        .iter()
        .map(|s| {
            Curve::new(match s {
                PathSeg::Cubic(c) => *c,
                PathSeg::Quad(q) => q.raise(),
                PathSeg::Line(l) => CubicBez::new(l.p0, l.p0.lerp(l.p1, 1.0 / 3.0), l.p0.lerp(l.p1, 2.0 / 3.0), l.p1),
            })
        })
        .collect();
    let mut crossed: u64 = 2 * curves.iter().filter(|c| c.may_loop()).count() as u64;
    let within = each_overlap(&seg.boxes, cap, |i, j| {
        work::add(Work::CrossingPairs, 1);
        let (Some(&ka), Some(&kb)) = (seg.kind.get(i), seg.kind.get(j)) else { return };
        crossed += match (ka, kb) {
            (Kind::Line(x), Kind::Line(y)) => match (seg.line_segs.get(x), seg.line_segs.get(y)) {
                (Some(a), Some(b)) => u64::from(!matches!(cross_of(a, b, slack), Cross::No)),
                _ => 0,
            },
            (Kind::Line(x), Kind::Curve(y)) | (Kind::Curve(y), Kind::Line(x)) => match (seg.line_segs.get(x), curves.get(y)) {
                (Some(l), Some(c)) => 2 * line_cubic_cross_at_most(l, c),
                _ => 0,
            },
            (Kind::Curve(x), Kind::Curve(y)) => match (curves.get(x), curves.get(y)) {
                (Some(a), Some(b)) => 2 * cubics_cross_at_most(a, b, slack),
                _ => 0,
            },
        };
    });
    within.then_some(crossed)
}

/// The work of the lines of `seg` that cross at one point: for each line, its crossings with others
/// at (within a tolerance of) one point, `m` of them where `m` is two or more, count `m`², so a point
/// where `c` lines cross costs about `c`³, the sweep's cost for it. The crossings are found only when
/// all `pairs` of line bounds that overlap, were they at one point, could cost more than the limit.
fn crossing_work_within(seg: &Segments, pairs: usize, eps: f64, cap: usize) -> bool {
    let limit = limits().crossing_work as u64;
    // Lines through one point: c(c - 1) / 2 pairs, and c (c - 1)² of work.
    let c = ((1.0 + (1.0 + 8.0 * pairs as f64).sqrt()) / 2.0).ceil();
    if c * (c - 1.0) * (c - 1.0) <= limit as f64 {
        return true;
    }
    // For each line, where it is crossed (and to what the position is known), and by how many lines it is not known where.
    let mut at: Vec<Vec<(f64, f64)>> = vec![Vec::new(); seg.line_segs.len()];
    let mut anywhere: Vec<u64> = vec![0; seg.line_segs.len()];
    let slack = 2.0 * eps;
    let within = each_overlap(&seg.lines, cap, |i, j| {
        work::add(Work::CrossingPairs, 1);
        let (Some(a), Some(b)) = (seg.line_segs.get(i), seg.line_segs.get(j)) else { return };
        match cross_of(a, b, slack) {
            Cross::No => {}
            Cross::At { s, r, err } => {
                if let Some(list) = at.get_mut(i) {
                    list.push((s, err));
                }
                if let Some(list) = at.get_mut(j) {
                    list.push((r, err));
                }
            }
            Cross::Somewhere => {
                for k in [i, j] {
                    if let Some(n) = anywhere.get_mut(k) {
                        *n += 1;
                    }
                }
            }
        }
    });
    if !within {
        return false;
    }
    let mut total = 0u64;
    for (mut list, unknown) in at.into_iter().zip(anywhere) {
        if unknown > 0 {
            // Where they cross is not known: all of them, and the rest, may be at one point.
            let m = unknown.saturating_add(list.len() as u64);
            total = total.saturating_add(m.saturating_mul(m));
        } else {
            list.sort_by(|x, y| x.0.total_cmp(&y.0));
            // Crossings at one point lie within a tolerance and the uncertainty of each of one another: the position is
            // `err` off at the most (the rounding, which is `slack` over the sine of the angle for lines nearly parallel).
            let mut run = 1u64;
            for (k, &(s, err)) in list.iter().enumerate() {
                let next = list.get(k + 1);
                if next.is_some_and(|&(n, n_err)| n - s <= slack + err + n_err) {
                    run += 1;
                    continue;
                }
                if run >= 2 {
                    total = total.saturating_add(run * run);
                }
                run = 1;
            }
        }
        if total > limit {
            return false;
        }
    }
    true
}

/// Can the pieces the sweep cuts the segments into hold their windings in [`Limits::coverage_entries`]?
/// A piece has, on each of its two sides, an entry for every input that covers it, and an input can
/// only cover a piece whose segment's bounds lie within its own bounds, so the entries are at most
/// two for each piece and each input whose bounds overlap its segment's. A segment is cut into one
/// piece more than the segments that cross it (a curve into three more, for its turns and where it
/// crosses itself). When the estimate by the bounds alone passes, that is all; when it does not, the
/// segments whose bounds overlap are walked, and where lines (or a line and a curve) cross is found,
/// so that a fan of lines that meet at a point, whose bounds overlap in thousands, cuts none. Curves
/// count as cutting each other once if an eighth of the one lies within the bounds of an eighth of the other,
/// apart from those that follow one another in a path, so curves that cross more than once are cut into
/// more pieces than counted. Nested squares of
/// n inputs hold 4n(n + 1) entries, and crossed by m thin bars cut each side of each square into 2m + 1
/// pieces. `pairs` are those of the segments' bounds among themselves. Two inputs or fewer hold two at
/// most for each piece, and are not counted.
fn coverage_entries_within(seg: &Segments, pairs: usize) -> bool {
    let inputs = &seg.inputs;
    if inputs.len() <= 2 {
        return true;
    }
    let mut all = seg.boxes.clone();
    all.extend_from_slice(inputs);
    let (Some(everything), Some(among_inputs)) = (overlap_pairs(&all, usize::MAX), overlap_pairs(inputs, usize::MAX)) else { return false };
    // Pairs of a segment's bounds and an input's.
    let across = everything.saturating_sub(pairs).saturating_sub(among_inputs) as u64;
    let crossings = pairs.saturating_sub(seg.touching_pairs) as u64;
    let cap = limits().coverage_entries as u64;
    // Three estimates, each cheaper than the next and no smaller than it is: with no more known than
    // that no segment's bounds overlap more inputs than there are; with the most inputs that any one
    // does; and with, for each, the segments that cross it.
    let factor = if seg.curves.is_empty() { 1 } else { 4 };
    let coarse = across.saturating_mul(2).saturating_mul(factor).saturating_add((inputs.len() as u64).saturating_mul(crossings).saturating_mul(4));
    let entries = if coarse <= cap {
        coarse
    } else {
        let covering = overlap_degrees(&seg.boxes, inputs);
        let most = covering.iter().copied().max().unwrap_or(0) as u64;
        let alone: u64 = covering.iter().enumerate().map(|(i, &d)| 2 * (1 + seg.turns(i)) * d as u64).sum();
        let middle = alone.saturating_add(most.saturating_mul(crossings).saturating_mul(4));
        if middle <= cap { middle } else { piece_entries(seg, &covering) }
    };
    work::add(Work::CoverageEntries, usize::try_from(entries).unwrap_or(usize::MAX));
    entries <= cap
}

/// The entries of the windings of the pieces of the segments, from the inputs (`covering`) and the
/// pieces each segment is cut into ([`coverage_entries_within`]), found by looking at the segments
/// whose bounds overlap: where lines (or a line and a curve) cross, and so cut each other, is found
/// exactly, and curves as cutting each other once when an eighth of the one lies within the bounds of an
/// eighth of the other.
fn piece_entries(seg: &Segments, covering: &[usize]) -> u64 {
    let slack = 2.0 * crate::boolean::sweep_tolerance(seg.extent);
    // Curves can only cross where an eighth of one lies within the bounds of an eighth of the other.
    let eighths: Vec<[Rect; 8]> =
        seg.curve_segs.iter().map(|c| std::array::from_fn(|k| c.subsegment(k as f64 / 8.0..(k + 1) as f64 / 8.0).bounding_box())).collect();
    let touch = |a: &Rect, b: &Rect| a.x0 <= b.x1 && b.x0 <= a.x1 && a.y0 <= b.y1 && b.y0 <= a.y1;
    let mut cuts = vec![0usize; seg.boxes.len()];
    // The pairs are no more than the limit on them, which was checked before.
    let _ = each_overlap(&seg.boxes, usize::MAX, |i, j| {
        work::add(Work::CrossingPairs, 1);
        let (Some(&a), Some(&b)) = (seg.kind.get(i), seg.kind.get(j)) else { return };
        let (ci, cj) = match (a, b) {
            (Kind::Line(x), Kind::Line(y)) => match (seg.line_segs.get(x), seg.line_segs.get(y)) {
                (Some(x), Some(y)) => line_cuts(x, y, slack),
                _ => (0, 0),
            },
            (Kind::Line(x), Kind::Curve(y)) => match (seg.line_segs.get(x), seg.curve_segs.get(y)) {
                (Some(x), Some(y)) => line_curve_cuts(x, y, slack),
                _ => (0, 0),
            },
            (Kind::Curve(x), Kind::Line(y)) => match (seg.curve_segs.get(x), seg.line_segs.get(y)) {
                (Some(x), Some(y)) => {
                    let (line, curve) = line_curve_cuts(y, x, slack);
                    (curve, line)
                }
                _ => (0, 0),
            },
            (Kind::Curve(x), Kind::Curve(y)) => match (eighths.get(x), eighths.get(y)) {
                (Some(x), Some(y)) => {
                    let near = usize::from(x.iter().any(|p| y.iter().any(|q| touch(p, q))));
                    (near, near)
                }
                _ => (1, 1),
            },
        };
        for (k, n) in [(i, ci), (j, cj)] {
            if let Some(c) = cuts.get_mut(k) {
                *c += n;
            }
        }
    });
    let mut total = 0u64;
    for (i, &inputs) in covering.iter().enumerate() {
        // Curves that follow one another in a path were counted as cutting each other at their shared end.
        let cut = cuts.get(i).copied().unwrap_or(0).saturating_sub(seg.touching_curves.get(i).copied().unwrap_or(0));
        total = total.saturating_add(2u64.saturating_mul(1 + cut as u64 + seg.turns(i)).saturating_mul(inputs as u64));
    }
    total
}

/// How many pieces the lines `a` and `b` cut each other into (as the pieces more than before): one
/// each where they cross away from an end of the one cut, and where they lie on one line and overlap, two.
fn line_cuts(a: &Line, b: &Line, slack: f64) -> (usize, usize) {
    let (d, e, w) = (a.p1 - a.p0, b.p1 - b.p0, b.p0 - a.p0);
    let (la, lb) = (d.hypot(), e.hypot());
    let den = d.cross(e);
    if parallel(den, la, lb) {
        // Parallel: lines on one line that overlap are cut at each other's ends (end to end, they are not).
        if la <= 0.0 || w.cross(d).abs() / la > slack {
            // Lines apart cut nothing; lines nearly parallel that may cross cut each other once.
            return if matches!(cross_of(a, b, slack), Cross::No) { (0, 0) } else { (1, 1) };
        }
        let (t0, t1) = (w.dot(d) / la, (w + e).dot(d) / la);
        let overlap = t0.max(t1).min(la) - t0.min(t1).max(0.0);
        return if overlap > slack { (2, 2) } else { (0, 0) };
    }
    let (s, r) = (w.cross(e) / den * la, w.cross(d) / den * lb);
    if s < -slack || s > la + slack || r < -slack || r > lb + slack {
        return (0, 0);
    }
    let inside = |x: f64, length: f64| usize::from(x > slack && x < length - slack);
    (inside(s, la), inside(r, lb))
}

/// The same for a line and a curve: the line is cut where the curve crosses it, and the curve where it
/// crosses the line, each away from the ends.
fn line_curve_cuts(line: &Line, curve: &PathSeg, slack: f64) -> (usize, usize) {
    let length = (line.p1 - line.p0).hypot();
    let (mut on_line, mut on_curve) = (0, 0);
    for hit in curve.intersect_line(*line) {
        on_line += usize::from(hit.line_t * length > slack && hit.line_t * length < length - slack);
        on_curve += usize::from(hit.segment_t > 1e-9 && hit.segment_t < 1.0 - 1e-9);
    }
    (on_line, on_curve)
}

/// Can a sweep of `paths` be built within `cap` pairs of segments whose bounds overlap,
/// [`MAX_SWEEP_LINE_PAIRS`] pairs of pieces on the sweep line, [`MAX_SWEEP_EVENT_WORK`] searches of it,
/// and the limits on segment ends at one point, lines crossing at one point, and entries of windings?
pub(crate) fn sweep_fits_within<'a>(paths: impl IntoIterator<Item = &'a BezPath>, cap: usize) -> bool {
    let mut seg = Segments::default();
    for p in paths {
        seg.add_path(p);
    }
    // Segments that meet at one point cost about the fourth power of their number in the sweep.
    let (most_ends, ends_work) = ends_at_points(&seg.ends, seg.extent);
    work::add(Work::PricedEnds, usize::try_from(ends_work).unwrap_or(usize::MAX));
    if most_ends > limits().ends_at_a_point || ends_work > limits().ends_work as u64 {
        return false;
    }
    let Some(pairs) = overlap_pairs(&seg.boxes, cap) else { return false };
    if !heights_overlap_within(&seg.heights, limits().sweep_line_pairs) {
        return false;
    }
    // The crossings: a pair of lines crosses once, a pair of a line and a curve up to three times, a
    // pair of curves up to nine; each pair counts for as many (at most), so the pairs of bounds
    // that overlap are counted by the kinds of segment in them.
    let (Some(line_pairs), Some(curve_pairs)) = (overlap_pairs(&seg.lines, cap), overlap_pairs(&seg.curves, cap)) else { return false };
    let mixed = pairs.saturating_sub(line_pairs).saturating_sub(curve_pairs);
    // In 64 bits: the products pass what a 32-bit `usize` holds.
    let crossings = (line_pairs as u64).saturating_add((mixed as u64).saturating_mul(3)).saturating_add((curve_pairs as u64).saturating_mul(9));
    if crossings.saturating_mul(peak_depth(&seg.heights) as u64) > limits().sweep_event_work as u64 {
        return false;
    }
    // The segments that really cross: the count above takes every pair of bounds that overlap for a crossing, and
    // where all of them cross (a polygon whose edges cross one another) the sweep takes several times as long. They
    // are held to their own number ([`MAX_REAL_CROSSINGS`], whatever the pieces on the line) and to their number times the
    // pieces on the sweep line ([`MAX_CROSSED_WORK`]). They are counted whenever the most the count can come to, as it
    // prices them (a line pair one, a pair with a curve at most three and two curves at most nine, each such place twice,
    // and a cubic that loops once, twice), could pass either limit.
    let depth = peak_depth(&seg.heights) as u64;
    let most = limits().real_crossings as u64;
    let crossed_work = limits().crossed_work as u64;
    let priced =
        (line_pairs as u64).saturating_add(6 * mixed as u64).saturating_add(18 * curve_pairs as u64).saturating_add(2 * seg.curves.len() as u64);
    if priced > most || priced.saturating_mul(depth) > crossed_work {
        let Some(crossed) = real_crossings(&seg, crate::boolean::sweep_tolerance(seg.extent), cap) else { return false };
        work::add(Work::Crossings, usize::try_from(crossed).unwrap_or(usize::MAX));
        if crossed > most || crossed.saturating_mul(depth) > crossed_work {
            return false;
        }
    }
    // Segments that follow one another in a path touch at the end they share, and cross nowhere.
    if !crossing_work_within(&seg, line_pairs.saturating_sub(seg.touching_lines), crate::boolean::sweep_tolerance(seg.extent), cap) {
        return false;
    }
    coverage_entries_within(&seg, pairs)
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
        // Taken in 64 bits: on a 32-bit target the limit is `usize::MAX` and the product may not fit in `usize`.
        let estimate = pairs as u64 * depth as u64;
        assert!(depth > 1_500 && estimate > MAX_SWEEP_EVENT_WORK as u64, "{pairs} pairs, {depth} on the line");
        // A limit past what `usize` holds cannot be set, so there is nothing that lets the hatch through
        // on a target where the estimate does not fit.
        if usize::try_from(estimate).is_ok() {
            let unlimited = Limits { sweep_event_work: usize::MAX, crossed_work: usize::MAX, ..Limits::default() };
            assert!(with_limits(unlimited, || sweep_fits([&hatch])), "only the searches of the line refuse it");
            // Each of the two limits on them refuses it alone: it is 3.7 million lines of crossings that count, and
            // the ones that really cross, a fifth of them, times the depth are over the second as well.
            let only_pairs = Limits { crossed_work: usize::MAX, ..Limits::default() };
            let only_crossed = Limits { sweep_event_work: usize::MAX, ..Limits::default() };
            assert!(!with_limits(only_pairs, || sweep_fits([&hatch])) && !with_limits(only_crossed, || sweep_fits([&hatch])));
        }
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

    /// The default is 2^32, or the most `usize` holds where that is less (the web has 32 bits).
    #[test]
    fn the_event_limit_is_two_to_the_32_or_the_most_usize_holds() {
        assert_eq!(MAX_SWEEP_EVENT_WORK as u64, (1u64 << 32).min(usize::MAX as u64));
        assert_eq!(Limits::default().sweep_event_work, MAX_SWEEP_EVENT_WORK);
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
        // A fan of triangles with a corner at one point has two ends there for each: 256 of them are 512.
        let fan = |n: usize| {
            let mut b = BezPath::new();
            for i in 0..n {
                let angle = |k: f64| (i as f64 + k) / n as f64 * std::f64::consts::TAU;
                b.move_to((0.0, 0.0));
                b.line_to((100.0 * angle(0.0).cos(), 100.0 * angle(0.0).sin()));
                b.line_to((100.0 * angle(0.5).cos(), 100.0 * angle(0.5).sin()));
                b.close_path();
            }
            b
        };
        assert!(sweep_fits([&fan(256)]) && !sweep_fits([&fan(257)]));
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
            // The estimate is at least the entries the windings hold, and under twice them.
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

    /// `k` spikes (a triangle from an apex to two points on a circle of 100), the apexes on a ring of
    /// radius `rho`, and with `far` one cubic whose control points reach that far.
    fn spikes(k: usize, rho: f64, far: Option<f64>) -> BezPath {
        let mut p = BezPath::new();
        for i in 0..k {
            let a = std::f64::consts::TAU * i as f64 / k as f64;
            let b = a + std::f64::consts::TAU / (3.0 * k as f64);
            let m = (a + b) / 2.0;
            p.move_to((rho * m.cos(), rho * m.sin()));
            p.line_to((100.0 * a.cos(), 100.0 * a.sin()));
            p.line_to((100.0 * b.cos(), 100.0 * b.sin()));
            p.close_path();
        }
        if let Some(far) = far {
            p.move_to((200.0, 0.0));
            p.curve_to((far, 0.0), (far, 10.0), (200.0, 10.0));
            p.close_path();
        }
        p
    }

    /// The sweep's tolerance is set by every point of every element, control points too
    /// ([`crate::boolean::eps_for`]); the ends are hashed at that tolerance, not at the one the ends alone set.
    /// 400 spikes whose apexes are 0.004 apart, beside one cubic with control points at 1e12 (a tolerance
    /// of 0.014), meet at one point to the sweep, which takes 5.5 s for them.
    #[test]
    fn ends_are_hashed_at_the_tolerance_the_control_points_set() {
        use crate::{FillRule, PathOpsError};
        let (at, over) = (spikes(256, 0.004, Some(1e12)), spikes(257, 0.004, Some(1e12)));
        let eps = crate::boolean::eps_for([&over]).unwrap();
        assert!(eps > 0.01 && eps < 0.02, "{eps}");
        // The ends alone would have set a tolerance of a millionth: the apexes would be 514 points.
        let (ends, ends_only) = ends_of(&over);
        assert!(most_ends_at_a_point(&ends, ends_only) < 20, "{}", most_ends_at_a_point(&ends, ends_only));
        assert_eq!(most_ends_at_a_point(&ends, 1e12), 514);
        // At the limit of 512 ends the sweep is made, one more slice and it is refused before the sweep.
        assert!(sweep_fits([&at]) && !sweep_fits([&over]));
        work::reset();
        assert_eq!(crate::try_area(&over, FillRule::NonZero), Err(PathOpsError::TooComplex));
        assert_eq!(work::take(Work::Sweeps), 0, "refused before the sweep was built");
        assert!(with_limits(Limits { ends_at_a_point: 514, ..Limits::default() }, || sweep_fits([&over])));
        assert!(!with_limits(Limits { ends_at_a_point: 513, ..Limits::default() }, || sweep_fits([&over])));
        // Without the far cubic the apexes are apart, to a sweep that tolerates a millionth.
        assert!(sweep_fits([&spikes(257, 0.004, None)]));
    }

    /// `k` lines through the origin, each from a point on a circle of 100 to the opposite one.
    fn star_of_lines(k: usize) -> BezPath {
        let mut p = BezPath::new();
        for i in 0..k {
            let a = std::f64::consts::PI * (i as f64 + 0.5) / k as f64;
            p.move_to((100.0 * a.cos(), 100.0 * a.sin()));
            p.line_to((-100.0 * a.cos(), -100.0 * a.sin()));
        }
        p
    }

    /// Lines that cross at one point cost the cube of their number in the sweep, and the pairs of
    /// bounds do not say: a line that crosses `m` others at one point counts `m`², so `c` lines count
    /// `c` (`c` - 1)² and the limit of 2^24 is at 256 lines. The crossings are found only when all the
    /// pairs of line bounds could cost more than the limit, and in proportion to those pairs.
    #[test]
    fn lines_that_cross_at_one_point_are_counted_by_their_work() {
        use crate::{FillRule, PathOpsError};
        assert_eq!(MAX_CROSSING_WORK, 1 << 24);
        assert_eq!(256 * 255 * 255, 16_646_400);
        const { assert!(256 * 255 * 255 <= MAX_CROSSING_WORK && 257 * 256 * 256 > MAX_CROSSING_WORK) };
        // The limit is at the work: a star of 40 lines is 40 x 39² = 60,840.
        let star = star_of_lines(40);
        let wanted = 40 * 39 * 39;
        assert!(with_limits(Limits { crossing_work: wanted, ..Limits::default() }, || sweep_fits([&star])));
        work::reset();
        assert!(!with_limits(Limits { crossing_work: wanted - 1, ..Limits::default() }, || sweep_fits([&star])));
        assert_eq!(work::take(Work::CrossingPairs), 40 * 39 / 2, "every pair of the lines was walked once");
        // 256 lines pass and 257 do not, by default, and the refusal is before the sweep.
        assert!(sweep_fits([&star_of_lines(256)]));
        work::reset();
        assert!(!sweep_fits([&star_of_lines(257)]));
        assert_eq!(work::take(Work::CrossingPairs), 257 * 256 / 2);
        assert_eq!(crate::try_area(&star_of_lines(257), FillRule::NonZero), Err(PathOpsError::TooComplex));
        assert_eq!(work::take(Work::Sweeps), 0, "refused before the sweep was built");
        // A thousand lines are refused after their pairs are walked, not after a minute and a half.
        work::reset();
        assert!(!sweep_fits([&star_of_lines(1_020)]));
        assert_eq!(work::take(Work::CrossingPairs), 1_020 * 1_019 / 2);
    }

    /// The crossings are not looked for when the pairs of bounds are too few to cost the limit at one
    /// point, are walked in proportion to those pairs when they are not, and not at all when there are
    /// more than the limit on pairs.
    #[test]
    fn the_crossings_of_lines_are_walked_in_proportion_to_their_pairs() {
        // Two families of lines, slanting across each other and meeting nowhere twice.
        let hatch = |n: usize| -> BezPath {
            let mut p = BezPath::new();
            for i in 0..n {
                let y = 3.0 * i as f64;
                p.move_to((0.0, y));
                p.line_to((300.0, y + 100.7));
                p.move_to((0.0, 300.0 - y + 0.3));
                p.line_to((300.0, 200.0 - y));
            }
            p
        };
        let overlapping = |p: &BezPath| overlap_pairs(&p.segments().map(|s| s.bounding_box()).collect::<Vec<_>>(), usize::MAX).unwrap();
        // Few pairs: the bound of the work, were they all at one point, is under the limit.
        let few = hatch(10);
        work::reset();
        assert!(sweep_fits([&few]));
        assert_eq!(work::take(Work::CrossingPairs), 0);
        // With a limit under it, they are all walked, once each, and none meet at a point.
        let tight = Limits { crossing_work: 100, ..Limits::default() };
        for n in [30usize, 60, 120] {
            let p = hatch(n);
            work::reset();
            assert!(with_limits(tight, || sweep_fits([&p])), "{n}");
            assert_eq!(work::take(Work::CrossingPairs), overlapping(&p), "{n}");
        }
        // More pairs than the limit: refused before any is walked.
        let p = hatch(60);
        let pairs = overlapping(&p);
        work::reset();
        assert!(!with_limits(Limits { segment_pairs: pairs - 1, ..tight }, || sweep_fits([&p])));
        assert_eq!(work::take(Work::CrossingPairs), 0);
    }

    /// The walk of the overlapping pairs of bounds finds those the count does, and the degrees are
    /// each box's pairs.
    #[test]
    fn the_pairs_and_degrees_of_bounds_match_a_count_one_by_one() {
        let mut seed = 0x9e37_79b9_7f4a_7c15_u64;
        let mut next = move || {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            f64::from((seed % 21) as u8)
        };
        for n in [0usize, 1, 2, 7, 40, 120] {
            // Small whole coordinates, so that boxes touch, repeat, and have no area.
            let boxes: Vec<Rect> = (0..n)
                .map(|_| {
                    let (x, y) = (next(), next());
                    Rect::new(x - 0.0, y, x + (next() % 4.0), y + (next() % 4.0))
                })
                .collect();
            let others: Vec<Rect> = (0..n / 2 + 1)
                .map(|_| {
                    let (x, y) = (next(), next());
                    Rect::new(x, y, x + next() % 11.0, y + next() % 11.0)
                })
                .collect();
            let touch = |a: &Rect, b: &Rect| a.x0 <= b.x1 && b.x0 <= a.x1 && a.y0 <= b.y1 && b.y0 <= a.y1;
            let mut brute: Vec<(usize, usize)> = Vec::new();
            for i in 0..n {
                for j in 0..i {
                    if touch(&boxes[i], &boxes[j]) {
                        brute.push((i, j));
                    }
                }
            }
            assert_eq!(overlap_pairs(&boxes, usize::MAX), Some(brute.len()), "{n} boxes");
            let mut found: Vec<(usize, usize)> = Vec::new();
            assert!(each_overlap(&boxes, usize::MAX, |i, j| found.push((i.max(j), i.min(j)))));
            found.sort_unstable();
            brute.sort_unstable();
            assert_eq!(found, brute, "{n} boxes");
            // Over the cap it stops, having found a cap and one more.
            if brute.len() > 1 {
                let mut seen = 0;
                assert!(!each_overlap(&boxes, brute.len() - 1, |_, _| seen += 1));
                assert_eq!(seen, brute.len() - 1);
            }
            let want: Vec<usize> = boxes.iter().map(|b| others.iter().filter(|o| touch(b, o)).count()).collect();
            assert_eq!(overlap_degrees(&boxes, &others), want, "{n} boxes against others");
            let own: Vec<usize> = boxes.iter().map(|b| boxes.iter().filter(|o| touch(b, o)).count()).collect();
            assert_eq!(overlap_degrees(&boxes, &boxes), own, "{n} boxes against themselves");
        }
    }

    /// n nested squares crossed by m thin bars: the sweep cuts each side of each square at each bar, and
    /// each of the 2m + 1 pieces keeps a list of the squares round it, so the entries are about 4n²m
    /// (300 and 300 took 1.3 GB, with the count of the segments before the cut at 4 x 300 x 301).
    fn nested_and_barred(n: usize, m: usize) -> Vec<BezPath> {
        let mut inputs: Vec<BezPath> = (1..=n).map(|k| crate::testutil::rect(-(k as f64), -(k as f64), k as f64, k as f64)).collect();
        let mut bars = BezPath::new();
        let w = 2.0 * n as f64 / m as f64;
        for i in 0..m {
            let x = -(n as f64) + w * (i as f64 + 0.25);
            bars.extend(crate::testutil::rect(x, -(n as f64) - 1.0, x + 0.3 * w, n as f64 + 1.0).iter());
        }
        inputs.push(bars);
        inputs
    }

    #[test]
    fn entries_are_counted_over_the_pieces_after_the_cut() {
        use crate::FillRule;
        let held = |inputs: &[BezPath]| -> usize {
            let arr = crate::boolean::Arrangement::new(inputs.iter().map(|s| (s.clone(), FillRule::NonZero)).collect()).unwrap();
            arr.top
                .segment_indices()
                .map(|s| arr.top.winding_clockwise(s.first_half()).len() + arr.top.winding_counter_clockwise(s.first_half()).len())
                .sum()
        };
        // The estimate from the pieces, as the last of the three that the limit is held to: with a limit
        // under all of them it is the one counted.
        let estimate = |inputs: &[BezPath]| -> usize {
            work::reset();
            assert!(!with_limits(Limits { coverage_entries: 1, ..Limits::default() }, || sweep_fits(inputs.iter())));
            work::take(Work::CoverageEntries)
        };
        let mut previous = 0;
        for (n, m) in [(10usize, 10usize), (20, 20), (20, 40), (40, 40), (80, 80)] {
            let inputs = nested_and_barred(n, m);
            let (counted, real) = (estimate(&inputs), held(&inputs));
            assert!(real <= counted && counted < 2 * real, "{n} squares, {m} bars: {real} held, {counted} counted");
            assert!(counted > previous, "{counted}");
            previous = counted;
        }
        // The limit is on that estimate: at it the sweep is allowed, one under it is refused.
        let inputs = nested_and_barred(20, 20);
        let exact = estimate(&inputs);
        assert!(with_limits(Limits { coverage_entries: exact, ..Limits::default() }, || sweep_fits(inputs.iter())));
        assert!(!with_limits(Limits { coverage_entries: exact - 1, ..Limits::default() }, || sweep_fits(inputs.iter())));
        // By default 200 nested squares and 200 bars (430 MB) are within the limit, and 300 and 300 (1.3 GB) are not.
        let (within, over) = (nested_and_barred(200, 200), nested_and_barred(300, 300));
        assert!(sweep_fits(within.iter()));
        assert!(estimate(&within) <= MAX_COVERAGE_ENTRIES);
        assert!(!sweep_fits(over.iter()));
        assert!(estimate(&over) > 5 * MAX_COVERAGE_ENTRIES / 2, "{}", estimate(&over));
    }

    /// Rays that meet at a point, each a shape of its own, have bounds that overlap in thousands of
    /// pairs and cross nowhere: they cut no piece, and the entries are those of the segments alone.
    #[test]
    fn rays_from_a_point_cut_no_pieces() {
        let rays: Vec<BezPath> = (0..250)
            .map(|i| {
                let a = std::f64::consts::TAU * f64::from(i) / 250.0;
                let b = a + std::f64::consts::TAU / 750.0;
                crate::testutil::polyline(&[(0.0, 0.0), (100.0 * a.cos(), 100.0 * a.sin()), (100.0 * b.cos(), 100.0 * b.sin())], true)
            })
            .collect();
        let boxes: Vec<Rect> = rays.iter().flat_map(|p| p.segments().map(|s| s.bounding_box())).collect();
        assert!(overlap_pairs(&boxes, usize::MAX).unwrap() > 20_000, "the bounds of the rays overlap in many pairs");
        work::reset();
        assert!(with_limits(Limits { coverage_entries: 1, ..Limits::default() }, || !sweep_fits(rays.iter())));
        let counted = work::take(Work::CoverageEntries);
        let alone: usize = overlap_degrees(&boxes, &rays.iter().map(kurbo::Shape::bounding_box).collect::<Vec<_>>()).iter().map(|d| 2 * d).sum();
        assert_eq!(counted, alone, "no piece was cut");
        assert!(sweep_fits(rays.iter()));
        // The same rays slightly crossed (each ray's tip pushed across the next) are cut.
        let crossed: Vec<BezPath> = (0..250)
            .map(|i| {
                let a = std::f64::consts::TAU * f64::from(i) / 250.0;
                let b = a + 3.0 * std::f64::consts::TAU / 250.0;
                crate::testutil::polyline(&[(0.0, 0.0), (100.0 * a.cos(), 100.0 * a.sin()), (100.0 * b.cos(), 100.0 * b.sin())], true)
            })
            .collect();
        work::reset();
        assert!(with_limits(Limits { coverage_entries: 1, ..Limits::default() }, || !sweep_fits(crossed.iter())));
        let cut = work::take(Work::CoverageEntries);
        let boxes: Vec<Rect> = crossed.iter().flat_map(|p| p.segments().map(|s| s.bounding_box())).collect();
        let alone: usize = overlap_degrees(&boxes, &crossed.iter().map(kurbo::Shape::bounding_box).collect::<Vec<_>>()).iter().map(|d| 2 * d).sum();
        assert!(cut > 3 * alone, "{cut} entries with the crossings, {alone} without");
    }

    /// Where two lines (or a line and a curve) cut each other into pieces, away from their ends.
    #[test]
    fn lines_and_curves_cut_each_other_where_they_cross_away_from_their_ends() {
        let line = |a: (f64, f64), b: (f64, f64)| Line::new(a, b);
        let slack = 1e-6;
        // Crossing: each is cut once. A T: the one it ends on is cut. Sharing an end, or apart: none.
        assert_eq!(line_cuts(&line((0.0, 0.0), (10.0, 10.0)), &line((0.0, 10.0), (10.0, 0.0)), slack), (1, 1));
        assert_eq!(line_cuts(&line((0.0, 0.0), (10.0, 0.0)), &line((5.0, 0.0), (5.0, 5.0)), slack), (1, 0));
        assert_eq!(line_cuts(&line((0.0, 0.0), (10.0, 0.0)), &line((10.0, 0.0), (10.0, 5.0)), slack), (0, 0));
        assert_eq!(line_cuts(&line((0.0, 0.0), (10.0, 0.0)), &line((0.0, 1.0), (10.0, 1.0)), slack), (0, 0));
        assert_eq!(line_cuts(&line((0.0, 0.0), (10.0, 0.0)), &line((20.0, 5.0), (20.0, -5.0)), slack), (0, 0));
        // On one line: two each if they overlap, none end to end or apart.
        assert_eq!(line_cuts(&line((0.0, 0.0), (10.0, 0.0)), &line((5.0, 0.0), (15.0, 0.0)), slack), (2, 2));
        assert_eq!(line_cuts(&line((0.0, 0.0), (10.0, 0.0)), &line((10.0, 0.0), (20.0, 0.0)), slack), (0, 0));
        assert_eq!(line_cuts(&line((0.0, 0.0), (10.0, 0.0)), &line((20.0, 0.0), (10.0, 0.0)), slack), (0, 0));
        // A curve through a line: both cut at each crossing (an arch across a bar twice).
        let arch = PathSeg::Cubic(kurbo::CubicBez::new((0.0, 0.0), (0.0, 10.0), (10.0, 10.0), (10.0, 0.0)));
        assert_eq!(line_curve_cuts(&line((-1.0, 5.0), (11.0, 5.0)), &arch, slack), (2, 2));
        assert_eq!(line_curve_cuts(&line((5.0, -1.0), (5.0, 11.0)), &arch, slack), (1, 1));
        assert_eq!(line_curve_cuts(&line((0.0, 0.0), (0.0, -5.0)), &arch, slack), (0, 0), "sharing an end");
        assert_eq!(line_curve_cuts(&line((20.0, 5.0), (30.0, 5.0)), &arch, slack), (0, 0));
    }

    /// Nested circles crossed by thin bars: the circles are cut by the bars and the bars by the circles, and the
    /// estimate of the entries holds what the windings hold.
    #[test]
    fn a_curve_across_bars_is_cut_where_it_crosses_them() {
        use crate::FillRule;
        let mut inputs: Vec<BezPath> =
            (1..=12).map(|k| crate::testutil::ellipse(Rect::new(-f64::from(k), -f64::from(k), f64::from(k), f64::from(k)))).collect();
        let mut bars = BezPath::new();
        for i in 0..12 {
            let x = -11.5 + 2.0 * f64::from(i);
            bars.extend(crate::testutil::rect(x, -13.0, x + 0.6, 13.0).iter());
        }
        inputs.push(bars);
        let arr = crate::boolean::Arrangement::new(inputs.iter().map(|s| (s.clone(), FillRule::NonZero)).collect()).unwrap();
        let real: usize = arr
            .top
            .segment_indices()
            .map(|s| arr.top.winding_clockwise(s.first_half()).len() + arr.top.winding_counter_clockwise(s.first_half()).len())
            .sum();
        work::reset();
        assert!(!with_limits(Limits { coverage_entries: 1, ..Limits::default() }, || sweep_fits(inputs.iter())));
        let counted = work::take(Work::CoverageEntries);
        assert!(real <= counted && counted < 3 * real, "{real} held, {counted} counted");
        // Concentric circles, whose bounds all overlap and which cross nowhere, are cut into few pieces: the eighths of a
        // curve whose bounds overlap those of its neighbour's count, and only they.
        let rings: Vec<BezPath> =
            (1..=100).map(|k| crate::testutil::ellipse(Rect::new(-f64::from(k), -f64::from(k), f64::from(k), f64::from(k)))).collect();
        let boxes: Vec<Rect> = rings.iter().flat_map(|p| p.segments().map(|s| s.bounding_box())).collect();
        let wholes: Vec<Rect> = rings.iter().map(kurbo::Shape::bounding_box).collect();
        let alone: usize = overlap_degrees(&boxes, &wholes).iter().map(|d| 2 * 4 * d).sum();
        work::reset();
        assert!(with_limits(Limits { coverage_entries: 1, ..Limits::default() }, || !sweep_fits(rings.iter())));
        let counted = work::take(Work::CoverageEntries);
        assert!(alone <= counted && counted < 6 * alone, "{counted} entries counted for a hundred concentric circles, {alone} for none cut");
    }

    /// Lines that really cross are counted, and held to [`Limits::crossed_work`] with the most pieces on the sweep line.
    /// A star polygon whose every edge goes to the vertex nearly opposite crosses the other edges in `n (k - 1)`
    /// places (`k = (n - 1) / 2`, about n²/2), and every pair of its edges has overlapping bounds: 2,001 edges are
    /// 2.0 million pairs, under the limit on pairs (4.2 million) and, at 4.0e9 searches of the sweep line, under the
    /// limit on those (4.3e9), and took 19 s and 3.5 GB; 1,001 take 2.4 s. 3,001 are over the limit on pairs.
    #[test]
    fn a_polygon_whose_edges_all_cross_is_held_to_the_work_of_its_crossings() {
        use crate::testutil::star;
        let check = |n: usize| {
            let path = star(n);
            work::reset();
            let start = std::time::Instant::now();
            let fits = sweep_fits([&path]);
            (fits, work::take(Work::Crossings), start.elapsed())
        };
        // 1,001 edges fit: 500 thousand pairs of bounds, a thousand pieces on the line, 5.0e8 at most, which
        // is under the limit with no pair tested.
        let (fits, crossed, _) = check(1001);
        assert!(fits && crossed == 0);
        // 1,501 are over the limit on the pairs' count (1.7e9), and the pairs are tested: they cross in
        // `n (k - 1)` places, one for each pair of edges that cross, and are over it.
        let (fits, crossed, _) = check(1501);
        assert!(!fits && crossed == 1501 * 749, "{crossed} crossings counted");
        // So are 2,001 (4.0e9), found by testing the 2 million pairs, in well under a second in a release build
        // (a bound of a minute here, where a build for tests is much slower, and the sweep takes minutes).
        let (fits, crossed, took) = check(2001);
        assert!(!fits && crossed == 2001 * 999, "{crossed} crossings counted");
        assert!(took < std::time::Duration::from_secs(60), "{took:?}");
        // 3,001 are over the limit on pairs, which is counted first, and no pair is tested.
        let (fits, crossed, took) = check(3001);
        assert!(!fits && crossed == 0 && took < std::time::Duration::from_secs(60), "{crossed} crossings counted in {took:?}");
        // Short polygons are not counted: their pairs of bounds are too few to reach the limit.
        assert_eq!(check(301).1, 0);
        // At the work it needs a star fits, and under it by one it does not: the crossings times the pieces on the line.
        let path = star(301);
        let needed = 301 * 149 * 301;
        for (limit, fits) in [(needed, true), (needed - 1, false)] {
            let at = Limits { crossed_work: limit, ..Limits::default() };
            assert_eq!(with_limits(at, || sweep_fits([&path])), fits, "{limit}");
        }
        // A hatch of 400 strokes each way, 640 thousand real crossings of the 2.6 million pairs of bounds that overlap, with
        // 1,600 pieces on the line (1.0e9, just under the limit), is within it.
        let hatch = crate::testutil::diagonals(400);
        work::reset();
        assert!(sweep_fits([&hatch]));
        assert_eq!(work::take(Work::Crossings), 640_000);
    }

    /// The places where segments cross are held to their own number, whatever the pieces on the sweep line
    /// ([`MAX_REAL_CROSSINGS`]). 206 stars of 201 points (each edge to the vertex nearly opposite, so each star crosses
    /// itself in `n (k - 1)` = 19,899 places) are stacked on a diagonal so that their heights do not overlap: 41,818
    /// elements, 4.1 million pairs of bounds (under the 4.2 million of `MAX_SEGMENT_PAIRS`), 201 pieces on the line, and so
    /// 8.3e8 searches of it, which is under both limits on those (4.3e9 and 1.1e9), and 4.1 million crossings, which took
    /// 13 s and 6.5 GB.
    #[test]
    fn stacked_stars_are_held_to_the_places_they_cross_whatever_the_pieces_on_the_line() {
        use crate::testutil::stacked_stars;
        let per_star: u64 = 201 * 99;
        let check = |stars: usize, bow: f64| {
            let path = stacked_stars(stars, 201, bow);
            work::reset();
            let start = std::time::Instant::now();
            let fits = sweep_fits([&path]);
            (fits, work::take(Work::Crossings) as u64, start.elapsed())
        };
        let path = stacked_stars(206, 201, 0.0);
        assert_eq!(path.elements().len(), 206 * 203);
        let (pairs, mixed, curve_pairs, depth) = event_counts(&path);
        assert!(pairs <= MAX_SEGMENT_PAIRS && mixed == 0 && curve_pairs == 0, "{pairs} pairs of bounds");
        assert!((200..=210).contains(&depth), "{depth} pieces on the line");
        assert!(pairs as u64 * depth as u64 <= MAX_SWEEP_EVENT_WORK as u64, "under the limit on the searches of the line");
        assert!(206 * per_star * depth as u64 <= MAX_CROSSED_WORK as u64, "and under the one on the crossings times the pieces on it");
        // Only the number of crossings refuses it, found by testing the 4.1 million pairs (milliseconds in a release build).
        let lifted = Limits { real_crossings: usize::MAX, ..Limits::default() };
        assert!(with_limits(lifted, || sweep_fits([&path])), "nothing else refuses it");
        let (fits, crossed, took) = check(206, 0.0);
        assert!(!fits && crossed == 206 * per_star, "{crossed} crossings counted");
        assert!(took < std::time::Duration::from_secs(60), "{took:?}");
        // The most that fits: 52 stars make 1,034,748 crossings, 53 make 1,054,647, against 1,048,576.
        assert_eq!(MAX_REAL_CROSSINGS, 1 << 20);
        // (52 make 1.05 million pairs of bounds, under the limit, so that none is tested.)
        let (fits, crossed, _) = check(52, 0.0);
        assert!(fits && crossed == 0, "{crossed} crossings counted");
        let (fits, crossed, _) = check(53, 0.0);
        assert!(!fits && crossed == 53 * per_star, "{crossed} crossings counted");
        // And to the crossing: a stack fits at the limit and not under it by one.
        let stack = stacked_stars(52, 201, 0.0);
        for (limit, fits) in [(52 * per_star, true), (52 * per_star - 1, false)] {
            let at = Limits { real_crossings: limit as usize, ..Limits::default() };
            assert_eq!(with_limits(at, || sweep_fits([&stack])), fits, "{limit}");
        }
        // Short stacks are not counted either.
        assert_eq!(check(20, 0.0).1, 0);
    }

    /// Curves cross as much as lines, and were not counted: 100 stars of 201 points whose edges are bowed cubics, 20,300
    /// elements, took 14.8 s and 3.7 GB. A place where a curve crosses counts for two (the sweep takes twice as long
    /// over it), and the count of them is a bound that is not below the number there are, which for these is about a
    /// third over (a pair of edges that cross is counted by the pieces of each that overlap, one for each).
    #[test]
    fn stacked_stars_of_bowed_cubics_are_held_to_the_places_they_cross() {
        use crate::testutil::stacked_stars;
        let per_star: u64 = 201 * 99;
        let check = |stars: usize| {
            let path = stacked_stars(stars, 201, 3.0);
            work::reset();
            let fits = sweep_fits([&path]);
            (fits, work::take(Work::Crossings) as u64)
        };
        let path = stacked_stars(100, 201, 3.0);
        assert_eq!(path.elements().len(), 100 * 203);
        let (line_pairs, mixed, curve_pairs, depth) = event_counts(&path);
        assert!(curve_pairs <= MAX_SEGMENT_PAIRS && line_pairs == 0 && mixed == 0, "{curve_pairs} pairs of bounds");
        assert!(curve_pairs as u64 * 9 * depth as u64 <= MAX_SWEEP_EVENT_WORK as u64, "{curve_pairs} pairs of curves, {depth} pieces on the line");
        // The crossings times the pieces on the line pass their limit as well, at this number of them, so both are lifted.
        let lifted = Limits { real_crossings: usize::MAX, crossed_work: usize::MAX, ..Limits::default() };
        assert!(with_limits(lifted, || sweep_fits([&path])), "nothing else refuses it");
        // Counted twice, never below the crossings of the lines the edges are bowed from and under twice that.
        let (fits, crossed) = check(100);
        assert!(!fits && crossed >= 2 * 100 * per_star * 98 / 100 && crossed <= 4 * 100 * per_star, "{crossed} counted");
        // 19 stars fit and 20 do not.
        let (fits, crossed) = check(19);
        assert!(fits && crossed <= MAX_REAL_CROSSINGS as u64 && crossed >= 2 * 19 * per_star * 98 / 100, "{crossed} counted");
        let (fits, crossed) = check(20);
        assert!(!fits && crossed > MAX_REAL_CROSSINGS as u64, "{crossed} counted");
    }

    /// A star alone is counted by the same test: where its pairs of bounds could make more crossings than the limit, the
    /// places where its edges cross are counted exactly for lines (`n (k - 1)`), and for bowed cubics as a bound that is at
    /// least twice that (to the few that the bow changes) and under twice again.
    #[test]
    fn single_stars_of_lines_and_of_cubics_are_counted_by_their_crossings() {
        use crate::testutil::{bowed_star, star};
        let n = 301;
        let low = Limits { real_crossings: 1_000, ..Limits::default() };
        let crossings = |path: &BezPath| {
            work::reset();
            let fits = with_limits(low, || sweep_fits([path]));
            (fits, work::take(Work::Crossings))
        };
        let (fits, counted) = crossings(&star(n));
        assert!(!fits && counted == n * 149, "{counted}");
        let (fits, counted) = crossings(&bowed_star(n, 3.0, 0.0, 0.0));
        assert!(!fits && counted >= 2 * n * 149 * 98 / 100 && counted <= 4 * n * 149, "{counted}");
        // At the default limit a star of lines of 1,291 points (831 thousand crossings) is made and one of 1,293 refused
        // by the crossings times the pieces on the line, and a star of cubics of 851 is made and one of 891 refused (the
        // bound on the crossings of a star of cubics is about a third over, counted twice).
        assert!(sweep_fits([&star(1291)]) && !sweep_fits([&star(1293)]));
        assert!(sweep_fits([&bowed_star(851, 3.0, 0.0, 0.0)]) && !sweep_fits([&bowed_star(891, 3.0, 0.0, 0.0)]));
    }

    /// Two arches, one rising to 3 and one from 2 falling to -1, cross twice (where their chords, the levels 0 and 2, never
    /// do), and the count is not below that, twice for the curves. A cubic whose height is zero at a quarter, a half and
    /// three quarters of its way crosses a line along that height three times, which is what the control polygon says.
    #[test]
    fn curves_and_lines_are_counted_at_the_most_they_can_cross_in() {
        let count = |paths: &[BezPath]| {
            let mut seg = Segments::default();
            for p in paths {
                seg.add_path(p);
            }
            real_crossings(&seg, 1e-6, usize::MAX).unwrap_or(u64::MAX)
        };
        let curve = |p: [(f64, f64); 4]| {
            let mut bp = BezPath::new();
            bp.move_to(p[0]);
            bp.curve_to(p[1], p[2], p[3]);
            bp
        };
        let (rising, falling) =
            (curve([(0.0, 0.0), (3.0, 4.0), (7.0, 4.0), (10.0, 0.0)]), curve([(0.0, 2.0), (3.0, -2.0), (7.0, -2.0), (10.0, 2.0)]));
        let both = count(&[rising.clone(), falling]);
        assert!((4..=18).contains(&both), "two places, counted twice, at least: {both}");
        // The height `64 (t - 1/4)(t - 1/2)(t - 3/4)`, which is a cubic with the control heights below, and a line along zero.
        let zigzag = curve([(0.0, -6.0), (8.0 / 3.0, 26.0 / 3.0), (16.0 / 3.0, -26.0 / 3.0), (8.0, 6.0)]);
        let line = {
            let mut bp = BezPath::new();
            bp.move_to((-1.0, 0.0));
            bp.line_to((9.0, 0.0));
            bp
        };
        assert_eq!(count(&[zigzag, line]), 6, "three places, each counted twice");
        // A cubic that curls into a loop crosses itself, once at the most, and is counted for it (twice); one that does not has none.
        let looped = curve([(0.0, 0.0), (30.0, 30.0), (-10.0, 30.0), (20.0, 0.0)]);
        assert_eq!(count(&[looped]), 2);
        assert_eq!(count(&[curve([(0.0, 0.0), (10.0, 5.0), (20.0, 5.0), (30.0, 0.0)])]), 0);
    }

    /// A grid of lines across cubics that each cross three times: 440 horizontal lines and 440 S-shaped cubics side by side,
    /// each line crossing each cubic in three places (580 thousand of them, 1.16 million as counted, which is over the limit).
    /// The count of every pair of bounds as three crossings is 580 thousand (a pair of a line and a curve counts three),
    /// which is under it, and no pair of lines or of curves has overlapping bounds: it was not counted at all (nor was its product with the
    /// pieces on the line, 1,300 of them, over 2^30). The most the count can come to, priced
    /// as it is, is what says it is to be counted.
    #[test]
    fn lines_across_cubics_that_cross_them_three_times_are_counted_as_the_curves_they_cross() {
        let m = 440;
        let mut grid = BezPath::new();
        // The S: from (x, -10) to (x + 8, 10), crossing every height between -3 and 3 three times.
        for i in 0..m {
            let x = 20.0 * i as f64;
            grid.move_to((x, -10.0));
            grid.curve_to((x + 8.0 / 3.0, 40.0), (x + 16.0 / 3.0, -40.0), (x + 8.0, 10.0));
        }
        for i in 0..m {
            let y = -3.0 + 6.0 * i as f64 / m as f64;
            grid.move_to((-1.0, y));
            grid.line_to((20.0 * m as f64, y));
        }
        let (line_pairs, mixed, curve_pairs, depth) = event_counts(&grid);
        assert_eq!((line_pairs, mixed, curve_pairs), (0, m * m, 0), "only the pairs of a line and a curve overlap");
        let unpriced = 3 * mixed as u64;
        assert!(unpriced <= MAX_REAL_CROSSINGS as u64, "{unpriced}: the count of three for each pair was under the limit");
        assert!(unpriced * depth as u64 <= MAX_SWEEP_EVENT_WORK as u64, "and the searches of the line, {depth} of them, too");
        assert!(unpriced * depth as u64 <= MAX_CROSSED_WORK as u64, "and so was the count times the pieces on the line");
        work::reset();
        assert!(!sweep_fits([&grid]));
        let counted = work::take(Work::Crossings) as u64;
        assert_eq!(counted, 2 * 3 * (m * m) as u64, "every line crosses every curve three times, each counted twice");
        // Only the number of crossings refuses it: without that limit the product with the pieces on the line is over its limit,
        // and without both nothing refuses it.
        let no_cap = Limits { real_crossings: usize::MAX, ..Limits::default() };
        assert!(!with_limits(no_cap, || sweep_fits([&grid])));
        let none = Limits { real_crossings: usize::MAX, crossed_work: usize::MAX, ..Limits::default() };
        assert!(with_limits(none, || sweep_fits([&grid])));
    }

    /// The number of crossings times the pieces on the sweep line is counted when the most the count can come to, priced as
    /// it is, could pass its limit, and not only the pairs of lines: 20 lines across 20 cubics that cross three times is
    /// 2,400 places as counted. At the limit, the count times the pieces on the line, it fits, and under it by one it does not.
    #[test]
    fn the_crossings_of_curves_times_the_pieces_on_the_line_are_held_to_their_limit() {
        let m = 20;
        let mut grid = BezPath::new();
        for i in 0..m {
            let x = 20.0 * i as f64;
            grid.move_to((x, -10.0));
            grid.curve_to((x + 8.0 / 3.0, 40.0), (x + 16.0 / 3.0, -40.0), (x + 8.0, 10.0));
        }
        for i in 0..m {
            let y = -3.0 + 6.0 * i as f64 / m as f64;
            grid.move_to((-1.0, y));
            grid.line_to((20.0 * m as f64, y));
        }
        let (_, _, _, depth) = event_counts(&grid);
        let needed = 2 * 3 * (m * m) as u64 * depth as u64;
        for (limit, fits) in [(needed, true), (needed - 1, false)] {
            let at = Limits { crossed_work: limit as usize, ..Limits::default() };
            assert_eq!(with_limits(at, || sweep_fits([&grid])), fits, "{limit}");
        }
    }

    /// A cubic that may curl into a loop is counted for crossing itself, twice, and the count is made for them: 600 of them far
    /// apart, which no pair of bounds overlaps, are 1,200 as counted, and a limit of 1,000 refuses them (and 400 are 800, and fit).
    #[test]
    fn cubics_that_may_loop_are_counted_for_crossing_themselves() {
        let loops = |n: usize| {
            let mut all = BezPath::new();
            for i in 0..n {
                let x = 100.0 * i as f64;
                all.move_to((x, 0.0));
                all.curve_to((x + 30.0, 30.0), (x - 10.0, 30.0), (x + 20.0, 0.0));
            }
            all
        };
        let low = Limits { real_crossings: 1_000, ..Limits::default() };
        work::reset();
        assert!(!with_limits(low, || sweep_fits([&loops(600)])));
        assert_eq!(work::take(Work::Crossings), 1_200);
        assert!(with_limits(low, || sweep_fits([&loops(400)])));
    }

    /// Lines nearly parallel were counted as not crossing at all (a pair under 1e-10 radians was taken for parallel): 450 lines
    /// through one point, at angles of 1e-13 to 1e-16 (lines 2e7 to 1e10 long, their ends 1.6 or 10 tolerances apart) counted
    /// no crossings and no work, and took 25 to 41 s. The pairs that cross are counted, at one point (the crossing is not known to
    /// better than the rounding over the sine of the angle, which is more than the whole length for the most nearly parallel), and
    /// their work, the square of the others a line crosses at a point for each line, is held to its limit.
    #[test]
    fn lines_through_one_point_at_angles_under_the_rounding_are_held_to_their_work() {
        use crate::testutil::nearly_parallel_lines;
        let lifted = Limits { crossing_work: usize::MAX, ..Limits::default() };
        for (l, dfac) in [(2e7, 1.6), (1e8, 1.6), (1e9, 1.6), (1e10, 1.6), (2e7, 10.0), (1e9, 10.0), (1e10, 10.0)] {
            let path = nearly_parallel_lines(450, l, dfac, false, true);
            work::reset();
            let start = std::time::Instant::now();
            assert!(!sweep_fits([&path]), "{l:e} long, {dfac} tolerances apart");
            assert!(start.elapsed() < std::time::Duration::from_secs(30), "{:?}", start.elapsed());
            // Only the work of the lines through a point refuses them.
            assert!(with_limits(lifted, || sweep_fits([&path])), "{l:e} long, {dfac} tolerances apart: nothing else refuses it");
        }
    }

    /// The same lines spread (their ends jittered, so that the crossings are along them): every pair crosses, 1,500 lines 1e8 long
    /// (1.12 million crossings) took 6.4 s and 1.6 GB, and 2,040 of them 1e10 long (2.08 million, 4,083 elements) 14 s and 3.2 GB
    /// and were counted as 408 thousand. All the pairs are counted as crossing now.
    #[test]
    fn nearly_parallel_lines_that_cross_along_their_length_are_counted() {
        use crate::testutil::nearly_parallel_lines;
        let lifted = Limits { real_crossings: usize::MAX, crossed_work: usize::MAX, crossing_work: usize::MAX, ..Limits::default() };
        for (n, l) in [(1500usize, 1e8), (2040, 1e10)] {
            let path = nearly_parallel_lines(n, l, 1.6, true, true);
            work::reset();
            assert!(!sweep_fits([&path]), "{n} lines {l:e} long");
            let counted = work::take(Work::Crossings) as u64;
            let pairs = (n * (n - 1) / 2) as u64;
            assert!(counted > MAX_REAL_CROSSINGS as u64 && counted >= pairs * 98 / 100, "{counted} counted of {pairs} pairs");
            assert!(with_limits(lifted, || sweep_fits([&path])), "{n} lines: nothing else refuses it");
        }
    }

    /// Lines that are nearly parallel and cross nowhere (no two have both ends of one on the line of the other, their ends
    /// 1.6 tolerances apart) are no more than they are: not counted, cheap, and made. The pairs that were taken for parallel were
    /// all of them, so this is what the count must not turn into.
    #[test]
    fn nearly_parallel_lines_that_do_not_cross_are_not_counted() {
        use crate::testutil::nearly_parallel_lines;
        for (n, l, spread) in [(2040usize, 1e10, true), (450, 1e10, false), (450, 2e7, false)] {
            let path = nearly_parallel_lines(n, l, 1.6, spread, false);
            work::reset();
            let start = std::time::Instant::now();
            assert!(sweep_fits([&path]), "{n} lines {l:e} long, apart");
            assert_eq!(work::take(Work::Crossings), 0, "{n} lines {l:e} long, apart");
            assert!(start.elapsed() < std::time::Duration::from_secs(30), "{:?}", start.elapsed());
        }
    }

    /// Two lines are told apart by the rounding, not by an angle: crossing, nearly parallel with the ends of each across the
    /// other, nearly parallel and apart, meeting at an end they share exactly, and a pair that is a point.
    #[test]
    fn two_lines_are_crossing_apart_or_unknown_by_the_rounding() {
        let line = |a: (f64, f64), b: (f64, f64)| Line::new(a, b);
        let slack = 1e-6;
        let at = |a: &Line, b: &Line| match cross_of(a, b, slack) {
            Cross::No => "no",
            Cross::At { .. } => "at",
            Cross::Somewhere => "somewhere",
        };
        // Across, and the point where: (5, 5) at 5 along each.
        let (x, y) = (line((0.0, 0.0), (10.0, 10.0)), line((0.0, 10.0), (10.0, 0.0)));
        assert!(matches!(cross_of(&x, &y, slack), Cross::At { s, r, .. } if (s - 50f64.sqrt()).abs() < 1e-9 && (r - 50f64.sqrt()).abs() < 1e-9));
        // Away from the other's length, and a crossing at an end, and the end of one on the other (a T): none.
        assert_eq!(at(&x, &line((20.0, 0.0), (30.0, -5.0))), "no");
        assert_eq!(at(&x, &line((5.0, 5.0), (9.0, 1.0))), "no", "the end of one is on the other");
        // Ends shared exactly meet there and nowhere else, whatever the angle.
        assert_eq!(at(&line((0.0, 0.0), (1e10, 1.0)), &line((1e10, 1.0), (0.0, 1e-3))), "no");
        // Nearly parallel, 1e10 long, with the ends across: somewhere or at, not none. Apart by 1e-3: none.
        let far = line((0.0, 0.0), (1e10, 0.0));
        assert_ne!(at(&far, &line((0.0, 1e-4), (1e10, -1e-4))), "no");
        assert_eq!(at(&far, &line((0.0, 1.0), (1e10, 1.0 + 1e-3))), "no");
        assert_eq!(at(&far, &line((1.0, 1.0), (1e10, 1.0))), "no", "level and apart");
        // On one line, overlapping: not told apart, and so counted. A line no longer than two tolerances is an end.
        let along = line((0.0, 0.0), (1e10, 1e10));
        assert_eq!(at(&along, &line((1e5, 1e5), (2e5, 2e5))), "somewhere");
        // Lines on one line that touch end to end (a hair of overlap, under the slack) meet at an end; more is not told apart.
        let chain = line((0.0, 0.0), (1e3, 1e3));
        assert_eq!(at(&chain, &line((1e3 - 1e-7, 1e3 - 1e-7), (2e3, 2e3))), "no");
        assert_eq!(at(&chain, &line((1e3 - 1e-4, 1e3 - 1e-4), (2e3, 2e3))), "somewhere");
        assert_eq!(at(&far, &line((1e5, 0.0), (2e5, 0.0))), "no", "level lines are not counted");
        assert_eq!(at(&x, &line((5.0, 0.0), (5.0 + 1e-7, 10.0))), "at", "across, nearly at a right angle");
        assert_eq!(at(&x, &line((5.0, 5.0 - 1e-7), (5.0, 5.0 + 1e-7))), "no", "no longer than two tolerances");
    }

    /// Where two lines nearly parallel cross is known to the rounding over the sine of the angle, and no better: two lines 1e10
    /// long at 1e-12 radians whose crossing is 20 beyond the end of one of them are told to cross (the place found is the end to
    /// within tens of millions, and the end counts as the whole of the doubt), and one 1e9 beyond is not.
    #[test]
    fn the_place_nearly_parallel_lines_cross_at_is_known_to_the_rounding_over_the_sine() {
        let slack = 1e-6;
        let level = Line::new((0.0, 0.0), (1e10, 0.0));
        let just_past = Line::new((5e9 + 20.0, -5e-3), (1.5e10 + 20.0, 5e-3));
        assert!(matches!(cross_of(&level, &just_past, slack), Cross::At { err, .. } if err > 1e6 && err < 1e9));
        let far_past = Line::new((6e9, -5e-3), (1.6e10, 5e-3));
        assert!(matches!(cross_of(&level, &far_past, slack), Cross::No));
        // At a right angle the place is known to the rounding of the coordinates: a hair.
        let (x, y) = (Line::new((0.0, 0.0), (10.0, 10.0)), Line::new((0.0, 10.0), (10.0, 0.0)));
        assert!(matches!(cross_of(&x, &y, slack), Cross::At { err, .. } if err < 1e-12));
    }

    /// Lines nearly parallel that are rotated so that the arithmetic is noisy (the rounding of a cross product is a part in 1e5
    /// of the product of the directions at 1e-11 radians, the crossings of one line are found scattered along it by far more than
    /// the tolerance): 450 through one point, at angles of 1e-13 to 1e-11, are 450 at one point all the same, grouped by the
    /// uncertainty of each place.
    #[test]
    fn rotated_lines_through_one_point_are_grouped_by_the_uncertainty_of_their_places() {
        use crate::testutil::nearly_parallel_lines;
        let lifted = Limits { crossing_work: usize::MAX, ..Limits::default() };
        for turn in [0.3, 0.6, 1.1] {
            let mut path = nearly_parallel_lines(450, 1e10, 1.6, false, true);
            path.apply_affine(kurbo::Affine::rotate(turn));
            assert!(!sweep_fits([&path]), "turned by {turn}");
            assert!(with_limits(lifted, || sweep_fits([&path])), "turned by {turn}: nothing else refuses it");
        }
    }

    /// Lines that lie on one line and overlap cannot be told from lines that cross anywhere along them, and are counted so: 300
    /// of them (each from `(i, i)` to `(i + 1,000, i + 1,000)`, on a diagonal) are 300 at one place of unknown position, which is
    /// 300 x 299² of work and more than the limit of 2^24; 250 are not. They are not a cost that was seen: 250 such lines, with the
    /// segments that close their subpaths, took 13 s in the sweep, and 500 over a minute and a half, and nothing counted them.
    #[test]
    fn lines_on_one_line_that_overlap_count_as_crossing_somewhere() {
        let on_the_diagonal = |n: usize| {
            let mut path = BezPath::new();
            for i in 0..n {
                path.move_to((i as f64, i as f64));
                path.line_to((i as f64 + 1000.0, i as f64 + 1000.0));
            }
            path
        };
        assert!(sweep_fits([&on_the_diagonal(250)]));
        assert!(!sweep_fits([&on_the_diagonal(300)]));
        let lifted = Limits { crossing_work: usize::MAX, ..Limits::default() };
        assert!(with_limits(lifted, || sweep_fits([&on_the_diagonal(300)])), "nothing else refuses it");
    }

    /// The same on a level line is not counted: pieces of no height are not on the sweep line (the sweep takes 30 ms for 250
    /// of them and 0.4 s for 1,000), and the stacks of bars and tiles that share a baseline are made as they were.
    #[test]
    fn lines_on_one_level_that_overlap_are_not_counted() {
        let on_the_level = |n: usize| {
            let mut path = BezPath::new();
            for i in 0..n {
                path.move_to((i as f64, 7.0));
                path.line_to((i as f64 + 1000.0, 7.0));
            }
            path
        };
        assert!(sweep_fits([&on_the_level(1000)]));
        // They are no work at one point, whatever the limit on it: a limit of a thousand holds none of them.
        let tight = Limits { crossing_work: 1000, ..Limits::default() };
        assert!(with_limits(tight, || sweep_fits([&on_the_level(1000)])));
        // On a line that rises a thousandth (as many, exactly on one line), they are counted.
        let rising = {
            let mut path = BezPath::new();
            for i in 0..300 {
                path.move_to((i as f64, 7.0 + i as f64 / 1024.0));
                path.line_to((i as f64 + 1000.0, 7.0 + (i as f64 + 1000.0) / 1024.0));
            }
            path
        };
        assert!(!sweep_fits([&rising]));
    }

    /// An end two lines share exactly is where they meet only if they are not on one line: a fan of segments from one point along
    /// one line shares its start exactly and overlaps itself, and 250 of them took 11 s (a vertical one 9 s) with nothing counting
    /// them. Lines that are the same line, lines that touch end to end from the same point, and lines that leave a point at an angle
    /// the arithmetic can tell (1e-13 radians) meet there only.
    #[test]
    fn lines_from_one_point_along_one_line_are_counted_as_the_lines_that_overlap_they_are() {
        let line = |a: (f64, f64), b: (f64, f64)| Line::new(a, b);
        let slack = 1e-6;
        let at = |a: &Line, b: &Line| match cross_of(a, b, slack) {
            Cross::No => "no",
            Cross::At { .. } => "at",
            Cross::Somewhere => "somewhere",
        };
        let out = line((0.0, 0.0), (10.0, 10.0));
        // The same direction from the same point, one longer: they overlap. Down the vertical as well.
        assert_eq!(at(&out, &line((0.0, 0.0), (20.0, 20.0))), "somewhere");
        assert_eq!(at(&line((0.0, 0.0), (0.0, 7.0)), &line((0.0, 0.0), (0.0, 30.0))), "somewhere");
        // The ends shared at the far end: the same.
        assert_eq!(at(&out, &line((10.0, 10.0), (4.0, 4.0))), "somewhere");
        // The opposite direction from the same point, or end to end: they touch at the end only.
        assert_eq!(at(&out, &line((0.0, 0.0), (-5.0, -5.0))), "no");
        assert_eq!(at(&out, &line((10.0, 10.0), (20.0, 20.0))), "no");
        // The same line, either way round (a subpath's closing segment comes back over its line): no.
        assert_eq!(at(&out, &line((10.0, 10.0), (0.0, 0.0))), "no");
        assert_eq!(at(&out, &line((0.0, 0.0), (10.0, 10.0))), "no");
        // Not on one line: they meet at the end they share, whatever the angle the arithmetic can tell.
        assert_eq!(at(&out, &line((0.0, 0.0), (10.0, 0.0))), "no");
        let far = line((0.0, 0.0), (1e10, 1e10));
        assert_eq!(at(&far, &line((0.0, 0.0), (1e10, 1e10 + 1e-3))), "no", "1e-13 radians");
        // Level lines are not counted, shared end or not.
        assert_eq!(at(&line((0.0, 3.0), (10.0, 3.0)), &line((0.0, 3.0), (20.0, 3.0))), "no");
    }

    /// The fans, as segments of a sweep: from the point (0, 0), the k-th to `(k, k)` (down the diagonal), `(0, k)` (down the vertical)
    /// or `(k, 0)` (level). 300 of the first two are 300 x 299² of work, over 2^24, and refused; 250 are not; the level ones are
    /// not counted. Nothing else refuses them: they have 300 ends at the point, under the 512.
    #[test]
    fn a_fan_of_segments_along_one_line_is_held_to_its_work() {
        // `kind` 0 goes down the diagonal, 1 down the vertical, 2 along a level.
        let fan = |n: usize, kind: usize| {
            let mut path = BezPath::new();
            for k in (1..=n).map(|k| k as f64) {
                path.move_to((0.0, 0.0));
                path.line_to([(k, k), (0.0, k), (k, 0.0)][kind]);
            }
            path
        };
        let lifted = Limits { crossing_work: usize::MAX, ..Limits::default() };
        for (kind, name) in [(0, "diagonal"), (1, "vertical")] {
            assert!(sweep_fits([&fan(250, kind)]), "250 on the {name}");
            assert!(!sweep_fits([&fan(300, kind)]), "300 on the {name}");
            assert!(with_limits(lifted, || sweep_fits([&fan(300, kind)])), "300 on the {name}: nothing else refuses them");
        }
        assert!(sweep_fits([&fan(500, 2)]), "500 level");
    }

    /// Copies of one shape one on another share both ends of every edge, and are priced by the ends at their corners (200 squares
    /// are 4 corners of 400 ends, 2e9 nanoseconds, and took 4 s), not as lines that overlap: 200 of them are admitted by the count of
    /// lines, as they were.
    #[test]
    fn copies_of_a_shape_one_on_another_are_priced_by_their_ends() {
        let mut copies = BezPath::new();
        for _ in 0..200 {
            copies.extend(crate::testutil::polyline(&[(0.0, 0.0), (10.0, 0.0), (10.0, 10.0), (0.0, 10.0)], true).iter());
        }
        assert!(sweep_fits([&copies]));
        let tight = Limits { crossing_work: 1000, ..Limits::default() };
        assert!(with_limits(tight, || sweep_fits([&copies])), "not one of the pairs is work");
    }

    /// The class of lines too near parallel to be placed is the rounding of the product of their directions and no more: 300
    /// lines 1e6 long at angles of 1e-8 to 1e-10 to one another (slopes evenly spread over 2e-8), through the neighbourhood of
    /// one place (their heights there differ by up to 2e-5), cross one another in 44,850 separate places a few thousand apart
    /// along each line, and are no work at one point; counted as unplaceable they would be 300 x 299².
    #[test]
    fn lines_at_a_small_angle_that_cross_in_separate_places_are_not_work_at_one_point() {
        use crate::testutil::Rng;
        let mut rng = Rng(0x00c0_ffee_1234_5678);
        let mut path = BezPath::new();
        for i in 0..300 {
            let (height, slope) = (rng.range(-1e-5, 1e-5), (i as f64 - 149.5) / 150.0 * 1e-8);
            path.move_to((0.0, height - slope * 5e5));
            path.line_to((1e6, height + slope * 5e5));
        }
        work::reset();
        assert!(sweep_fits([&path]));
        // Every pair was walked and found crossing somewhere, which the counts of the gate show: the pairs are all of them.
        assert_eq!(work::take(Work::CrossingPairs), 300 * 299 / 2);
    }

    /// The work of the ends of segments at the points where they meet is added up over the points (the most at any one point
    /// was all that was held): pies of 256 slices, each with 512 ends at its centre and 8,192 elements for 8 of them, stacked on a
    /// diagonal so that their heights do not overlap, took 1.3, 3.3 (three), 4.6 (four), 15 (eight) and 66 s (30, 2.7 GB). One pie
    /// is 1.31 billion nanoseconds as priced, three 3.93, four are over the 4 billion of the limit.
    #[test]
    fn stacked_pies_are_held_to_the_work_of_their_ends() {
        use crate::testutil::stacked_pies;
        let work_of = |count: usize| {
            let path = stacked_pies(count, 256);
            work::reset();
            let start = std::time::Instant::now();
            let fits = sweep_fits([&path]);
            (fits, work::take(Work::PricedEnds) as u64, start.elapsed())
        };
        let (fits, one, _) = work_of(1);
        assert!(fits && (1_300_000_000..1_320_000_000).contains(&one), "{one}");
        let (fits, three, _) = work_of(3);
        assert!(fits && (3 * one..3 * one + 1_000_000).contains(&three), "{three}");
        for count in [4, 8, 30] {
            let (fits, charged, took) = work_of(count);
            assert!(!fits && charged > MAX_ENDS_WORK as u64, "{count} pies: {charged}");
            assert!(took < std::time::Duration::from_secs(10), "{took:?}");
        }
        // The limit is what refuses them: lifted, 30 pies are not refused by anything else, and at the work of three they fit and
        // under it by one they do not.
        let lifted = Limits { ends_work: usize::MAX, ..Limits::default() };
        assert!(with_limits(lifted, || sweep_fits([&stacked_pies(30, 256)])));
        for (limit, fits) in [(three, true), (three - 1, false)] {
            let at = Limits { ends_work: limit as usize, ..Limits::default() };
            assert_eq!(with_limits(at, || sweep_fits([&stacked_pies(3, 256)])), fits, "{limit}");
        }
        // Many small ones: 100 copies of a polygon of 20 sides one on another (200 ends at each of its 20 corners, 0.9 s) fit, and
        // so do small pies in numbers that take as long.
        let copies = {
            let mut all = BezPath::new();
            for _ in 0..100 {
                all.extend(
                    crate::testutil::polyline(
                        &(0..20)
                            .map(|i| (50.0 + 40.0 * (i as f64 * 0.3141592653589793).cos(), 50.0 + 40.0 * (i as f64 * 0.3141592653589793).sin()))
                            .collect::<Vec<_>>(),
                        true,
                    )
                    .iter(),
                );
            }
            all
        };
        work::reset();
        assert!(sweep_fits([&copies]));
        let charged = work::take(Work::PricedEnds) as u64;
        assert!((700_000_000..1_200_000_000).contains(&charged), "{charged}");
    }

    /// A circle cut into 100,000 cubics costs its neighbours nothing, wherever it is and however small: at (1e12, 1e12) with a
    /// radius of 1,000 the handles (0.021) are about the slack (0.014), and the rays from a joint that were dropped for being
    /// shorter than that left the cone of each every direction, so that all 100,000 joints cost 18 each (1.8 million, over the
    /// limit) and the circle was refused; so were circles with handles under 2e-6 at ordinary coordinates.
    #[test]
    fn a_circle_of_many_cubics_costs_nothing_wherever_it_is() {
        use crate::testutil::cubic_circle;
        for (r, c) in [(1000.0, 1e12), (1000.0, 1e9), (1000.0, 0.0), (0.09, 0.0)] {
            let circle = cubic_circle(100_000, r, c, c);
            work::reset();
            assert!(sweep_fits([&circle]), "radius {r} at {c:e}");
            assert_eq!(work::take(Work::Crossings), 0, "radius {r} at {c:e}");
        }
    }

    /// Wiggles between the samples must not take a stack past the limit: 20,000 pairs of a cubic along the x axis and one
    /// whose height is `400 (t - .30)(t - .33)(t - .36)`, which cross three times in the length of a piece of eight (where the
    /// chords of a piece cross once), each pair on a diagonal of its own. The crossings as counted by the chords
    /// of eight pieces, twice each, are 40 thousand: under a limit of 100 thousand, with 120 thousand there are as counted
    /// twice. The bound is nine for each, and it is refused.
    #[test]
    fn pairs_of_cubics_that_wiggle_between_the_samples_are_counted_at_the_most_they_can_cross_in() {
        let k = 400.0;
        let (a0, a1, a2, a3) = (-0.03564 * k, 0.3258 * k, -0.99 * k, k);
        let y = [a0, a0 + a1 / 3.0, a0 + 2.0 * a1 / 3.0 + a2 / 3.0, a0 + a1 + a2 + a3];
        let mut stack = BezPath::new();
        for i in 0..20_000 {
            let (dx, dy) = (200.0 * i as f64, 2_000.0 * i as f64);
            stack.move_to((dx, dy));
            stack.curve_to((dx + 100.0 / 3.0, dy), (dx + 200.0 / 3.0, dy), (dx + 100.0, dy));
            stack.move_to((dx, dy + y[0]));
            stack.curve_to((dx + 100.0 / 3.0, dy + y[1]), (dx + 200.0 / 3.0, dy + y[2]), (dx + 100.0, dy + y[3]));
        }
        let (_, mixed, curve_pairs, _) = event_counts(&stack);
        assert_eq!((mixed, curve_pairs), (0, 20_000), "each pair of cubics overlaps, and none with another");
        let low = Limits { real_crossings: 100_000, ..Limits::default() };
        work::reset();
        assert!(!with_limits(low, || sweep_fits([&stack])), "20,000 pairs of three crossings are 120 thousand as counted");
        assert_eq!(work::take(Work::Crossings), 2 * 9 * 20_000, "nine for each pair, twice");
        // The most the count can come to, which says whether it is made, is 18 for each pair and 2 for each curve (there are 40,000
        // of them: the gate allows each for a loop whether it may loop or not, and these do not, so the count is 18 for each
        // pair and no more). At nine for each pair and 2 for each curve, 260 thousand, a limit is under the count and not under
        // that, so that the stack would pass unseen if the pairs were priced at nine.
        let nine = Limits { real_crossings: 9 * 20_000 + 2 * 40_000, ..Limits::default() };
        assert!(!with_limits(nine, || sweep_fits([&stack])));
        // As many as the limit allows, at nine for each (counted twice), are made.
        let some = Limits { real_crossings: 2 * 9 * 20_000, ..Limits::default() };
        assert!(with_limits(some, || sweep_fits([&stack])));
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
