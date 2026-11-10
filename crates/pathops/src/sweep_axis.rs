// Carved from VectorCraft's crates/pathops (src/sweep_axis.rs) at storytold/vectorcraft 8a7011a9d35974510227ebaaba66cb4e9178b217, on `BezPath`.
// Copyright (c) 2026 ArtCraft Team and the VectorCraft contributors. MIT OR Apache-2.0.

//! Which way the overlap sweep runs. `linesweeper` moves a horizontal line down the page, and the
//! time it spends at each event grows with the number of segments on that line at the time. A long,
//! low path (rows of a zigzag, a stroked line of type) can put thousands of segments on the line at
//! once, where the same path turned a quarter turn puts far fewer on it. [`turned_outlines`] and
//! [`turned_fills`] estimate both directions before the sweep; when the turned one is clearly
//! cheaper they return the input turned, the caller runs its operation on that, and [`unturn`]
//! turns the result back. A quarter turn swaps and negates coordinates, so no point moves by
//! rounding.
//!
//! The operation runs on the turned input from its first step, so everything after the choice sees
//! it turned: the counts of its limits, the metered dry sweep ([`crate::Limits::sweep_work`]) that
//! sweeps what the sweep will, and the sweep itself. The estimate only chooses the direction; the
//! meter is what holds the work, in whichever direction it runs.
//!
//! The estimate reads the input once (the segments are counted, and the boxes of at most
//! [`MAX_SAMPLE`] of them are made), so it is made after the size of the input has been held to
//! [`crate::MAX_ELEMENTS`], and what it reads is counted ([`Work::TurnReads`], [`Work::TurnBoxes`]).

use kurbo::{Affine, BezPath, PathSeg, Rect};

use crate::FillRule;
use crate::path::fill_bezpath;
use crate::work::{self, Work};

/// Inputs with fewer segments than this are always swept as they are.
const MIN_SEGMENTS: usize = 2048;

/// The turned direction is used when its estimate is below the upright one divided by this.
const MIN_GAIN: f64 = 1.5;

/// The most segments the estimate measures.
const MAX_SAMPLE: usize = 4096;

/// (x, y) to (−y, x).
const TURN: Affine = Affine::new([0.0, 1.0, -1.0, 0.0, 0.0, 0.0]);

/// (x, y) to (y, −x): the inverse of [`TURN`].
const UNTURN: Affine = Affine::new([0.0, -1.0, 1.0, 0.0, 0.0, 0.0]);

/// For each of the spans from `starts[i]` to `ends[i]`, the number of spans that overlap it
/// (touching counts, itself included), summed: the segments a sweep across these extents has on
/// its line while each one is there. Sorts both lists.
fn load(starts: &mut [f64], ends: &mut [f64]) -> u64 {
    starts.sort_unstable_by(f64::total_cmp);
    ends.sort_unstable_by(f64::total_cmp);
    // Spans that start at or before each end, less spans that end before each start: every span
    // that ends before a start also starts before it, so it was counted in the first sum.
    let (mut begun, mut i) = (0u64, 0usize);
    for &e in ends.iter() {
        while starts.get(i).is_some_and(|&s| s <= e) {
            i += 1;
        }
        begun += i as u64;
    }
    let (mut over, mut j) = (0u64, 0usize);
    for &s in starts.iter() {
        while ends.get(j).is_some_and(|&e| e < s) {
            j += 1;
        }
        over += j as u64;
    }
    begun.saturating_sub(over)
}

/// The segments of `outlines`.
fn outline_segments(outlines: &[BezPath]) -> impl Iterator<Item = PathSeg> + '_ {
    outlines.iter().flat_map(BezPath::segments)
}

/// The box of a segment's control points.
fn segment_box(s: PathSeg) -> Rect {
    match s {
        PathSeg::Line(l) => Rect::from_points(l.p0, l.p1),
        PathSeg::Quad(q) => Rect::from_points(q.p0, q.p2).union_pt(q.p1),
        PathSeg::Cubic(c) => Rect::from_points(c.p0, c.p3).union_pt(c.p1).union_pt(c.p2),
    }
}

/// Is the sweep over the `count` `segments`, each with the box [`segment_box`] gives it, clearly cheaper
/// turned a quarter turn? Past [`MAX_SAMPLE`] segments, one segment from each run of
/// `count / MAX_SAMPLE` (rounded up) stands for the run, at a position within it that changes from
/// run to run, and only those segments' boxes are built.
fn turn_pays(count: usize, segments: impl Iterator<Item = PathSeg>) -> bool {
    if count < MIN_SEGMENTS {
        return false;
    }
    let stride = count.div_ceil(MAX_SAMPLE);
    let mut next = picks(count, stride).peekable();
    let boxes: Vec<Rect> = segments.enumerate().filter(|(i, _)| next.next_if_eq(i).is_some()).map(|(_, s)| segment_box(s)).collect();
    work::add(Work::TurnReads, count);
    work::add(Work::TurnBoxes, boxes.len());
    if !boxes.iter().all(Rect::is_finite) {
        return false;
    }
    let (mut x0, mut x1): (Vec<f64>, Vec<f64>) = boxes.iter().map(|b| (b.x0, b.x1)).unzip();
    let (mut y0, mut y1): (Vec<f64>, Vec<f64>) = boxes.iter().map(|b| (b.y0, b.y1)).unzip();
    let pays = gain_pays(load(&mut x0, &mut x1), load(&mut y0, &mut y1));
    if pays {
        work::add(Work::Turns, 1);
    }
    pays
}

/// The indices, ascending, of the segments whose boxes are measured among `count`: one from each run of `stride` of them, at a position
/// within the run that changes from run to run ([`sample_offset`]) (none from the last run when its position is past the end).
fn picks(count: usize, stride: usize) -> impl Iterator<Item = usize> {
    (0..count.div_ceil(stride)).filter_map(move |run| {
        let i = run.checked_mul(stride)?.checked_add(sample_offset(run, stride))?;
        (i < count).then_some(i)
    })
}

/// Does the sweep pay to turn: is the load turned (the segments held at once, summed) less than the upright one divided by [`MIN_GAIN`]?
fn gain_pays(turned: u64, upright: u64) -> bool {
    turned as f64 * MIN_GAIN < upright as f64
}

/// The position, below `stride`, of the segment measured in run `run`: a hash of `run` (the
/// SplitMix64 finalizer), so input that repeats every few segments is measured at every position.
fn sample_offset(run: usize, stride: usize) -> usize {
    let mut z = (run as u64).wrapping_add(0x9e37_79b9_7f4a_7c15);
    z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    ((z ^ (z >> 31)) % stride as u64) as usize
}

/// The elements of `paths`, which the estimate reads (it is only made past [`MIN_SEGMENTS`] of them).
fn elements<'a>(paths: impl IntoIterator<Item = &'a BezPath>) -> usize {
    paths.into_iter().fold(0usize, |n, p| n.saturating_add(p.elements().len()))
}

/// `outlines` turned a quarter turn, when sweeping them that way is clearly cheaper. The size of
/// `outlines` has been held to the element limit, and none begins with a `ClosePath` (kurbo's `segments` panics on one;
/// [`crate::path::without_leading_closes`]).
pub(crate) fn turned_outlines(outlines: &[BezPath]) -> Option<Vec<BezPath>> {
    // A segment is at least an element: fewer elements than the gate are fewer segments.
    if elements(outlines) < MIN_SEGMENTS {
        return None;
    }
    let count = outlines.iter().map(|o| o.segments().count()).sum();
    turn_pays(count, outline_segments(outlines)).then(|| outlines.iter().map(|o| TURN * o).collect())
}

/// `paths` turned a quarter turn, when sweeping their fills that way is clearly cheaper. The size
/// of `paths` has been held to the element limit.
pub(crate) fn turned_fills(paths: &[(&BezPath, FillRule)]) -> Option<Vec<(BezPath, FillRule)>> {
    // The fill of a subpath has as many segments as the subpath has elements, at most.
    if elements(paths.iter().map(|(p, _)| *p)) < MIN_SEGMENTS {
        return None;
    }
    // The estimate is made on the paths as they are swept: closed, as [`fill_bezpath`] makes them.
    let fills: Vec<BezPath> = paths.iter().map(|(p, _)| fill_bezpath(p)).collect();
    work::add(Work::Visited, elements(&fills));
    let count = fills.iter().map(|f| f.segments().count()).sum();
    turn_pays(count, outline_segments(&fills)).then(|| paths.iter().map(|(p, r)| (TURN * *p, *r)).collect())
}

/// A result computed on turned input, turned back.
pub(crate) fn unturn(path: BezPath) -> BezPath {
    UNTURN * path
}

#[cfg(test)]
mod tests {
    use kurbo::{Join, Point, Shape as _, Stroke, StrokeOpts};

    use super::*;
    use crate::boolean::{normalize_bez_within, try_unite_all_upright, unite_all_upright};
    use crate::offset::{stroke_region_upright, try_stroke_region_upright};
    use crate::testutil::subpath_count;
    use crate::{
        BoolOp, DEFAULT_PRECISION, Limits, PathOpsError, stroke_region, try_area, try_boolean, try_stroke_region, try_unite_all_with, unite_all_with,
        with_limits,
    };

    const NZ: FillRule = FillRule::NonZero;

    /// A small xorshift generator, so the random cases are the same on every run.
    struct Rng(u64);

    impl Rng {
        fn unit(&mut self) -> f64 {
            self.0 ^= self.0 << 13;
            self.0 ^= self.0 >> 7;
            self.0 ^= self.0 << 17;
            (self.0 >> 11) as f64 / (1u64 << 53) as f64
        }
    }

    /// `rows` rows of a zigzag `cols` anchors long (smooth cubics between them), each anchor moved by up to `jitter`: a long, low path.
    fn zigzag(rows: usize, cols: usize, jitter: f64, rng: &mut Rng, closed: bool) -> BezPath {
        let mut anchors = Vec::new();
        for row in 0..rows {
            for col in 0..cols {
                let c = if row % 2 == 0 { col } else { cols - 1 - col };
                let x = c as f64 * 1.6 + (rng.unit() - 0.5) * jitter;
                let y = row as f64 * 6.0 + if col % 2 == 0 { 0.0 } else { 2.0 } + (rng.unit() - 0.5) * jitter;
                let dir = if row % 2 == 0 { 0.5 } else { -0.5 };
                anchors.push((Point::new(x, y), kurbo::Vec2::new(dir, 0.0)));
            }
        }
        let mut bp = BezPath::new();
        let Some(&(first, _)) = anchors.first() else { return bp };
        bp.move_to(first);
        let mut pairs: Vec<_> = anchors.windows(2).collect();
        let closing = [anchors[anchors.len() - 1], anchors[0]];
        if closed {
            pairs.push(&closing);
        }
        for w in pairs {
            let ((p, h), (q, g)) = (w[0], w[1]);
            bp.curve_to(p + h, q - g, q);
        }
        if closed {
            bp.close_path();
        }
        bp
    }

    /// The same path stood on end: a tall, narrow path.
    fn upright(p: &BezPath) -> BezPath {
        Affine::new([0.0, 1.0, 1.0, 0.0, 0.0, 0.0]) * p
    }

    /// A unit square with its lower left corner at (`x`, `y`).
    fn square(x: f64, y: f64) -> BezPath {
        crate::testutil::rect(x, y, x + 1.0, y + 1.0)
    }

    fn outline(p: &BezPath) -> BezPath {
        kurbo::stroke(p.iter(), &Stroke::new(1.0).with_join(Join::Round), &StrokeOpts::default(), 1e-3)
    }

    fn segments(p: &BezPath) -> usize {
        p.segments().count()
    }

    /// How many operations ran turned during `f`.
    fn turns_during(f: impl FnOnce()) -> usize {
        work::reset();
        f();
        work::take(Work::Turns)
    }

    /// `a` and `b` have the same pieces and cover the same area to within the refit precision: the
    /// area in one and not the other is less than a band [`DEFAULT_PRECISION`] wide along the outline.
    fn assert_same_region(a: &BezPath, b: &BezPath, what: &str) {
        assert_eq!(subpath_count(a), subpath_count(b), "{what}: subpaths");
        let band = DEFAULT_PRECISION * a.perimeter(1e-3);
        let area_a = try_area(a, NZ).unwrap();
        let area_b = try_area(b, NZ).unwrap();
        assert!((area_a - area_b).abs() <= band, "{what}: area {area_a} against {area_b}, band {band}");
        let apart = try_boolean(a, NZ, b, NZ, BoolOp::Xor, DEFAULT_PRECISION).unwrap();
        let apart = try_area(&apart, NZ).unwrap();
        assert!(apart <= band, "{what}: {apart} of {area_a} differs, band {band}");
    }

    #[test]
    fn a_long_low_path_is_swept_turned_and_a_tall_or_small_one_as_it_is() {
        let mut rng = Rng(7);
        let low = zigzag(3, 300, 0.0, &mut rng, false);
        let stroke = outline(&low);
        // More segments than the estimate measures, so it decides from a sample.
        assert!(segments(&stroke) > MAX_SAMPLE, "{} segments", segments(&stroke));
        assert_eq!(turns_during(|| drop(stroke_region(std::slice::from_ref(&stroke)))), 1);
        assert_eq!(turns_during(|| drop(try_stroke_region(std::slice::from_ref(&stroke)))), 1);
        let fill = zigzag(3, 800, 0.0, &mut rng, true);
        assert_eq!(turns_during(|| drop(unite_all_with(&[(&fill, NZ)], DEFAULT_PRECISION))), 1);
        assert_eq!(turns_during(|| drop(try_unite_all_with(&[(&fill, NZ)], DEFAULT_PRECISION))), 1);
        // Stood on end, the same paths are already narrow across the sweep.
        let tall = outline(&upright(&low));
        assert_eq!(turns_during(|| drop(stroke_region(std::slice::from_ref(&tall)))), 0);
        assert_eq!(turns_during(|| drop(unite_all_with(&[(&upright(&fill), NZ)], DEFAULT_PRECISION))), 0);
        // A square grid costs the same either way.
        let grid: Vec<BezPath> = (0..900).map(|i| square((i % 30) as f64 * 3.0, (i / 30) as f64 * 3.0)).collect();
        let grid: Vec<(&BezPath, FillRule)> = grid.iter().map(|p| (p, NZ)).collect();
        assert_eq!(turns_during(|| drop(unite_all_with(&grid, DEFAULT_PRECISION))), 0);
        // Below the size where the sweep costs much, nothing is turned (and nothing is read for the estimate).
        let short = outline(&zigzag(1, 40, 0.0, &mut rng, false));
        assert!(segments(&short) < MIN_SEGMENTS);
        work::reset();
        assert_eq!(turns_during(|| drop(stroke_region(std::slice::from_ref(&short)))), 0);
        assert_eq!((work::take(Work::TurnReads), work::take(Work::TurnBoxes)), (0, 0));
    }

    #[test]
    fn a_repeating_path_is_measured_at_every_position() {
        // Short segments in a row along the bottom, with a long one stacked above the row after
        // every `period - 1` of them. The short ones alone are cheaper to sweep turned; with the
        // long ones, the path is cheaper upright.
        let add = |path: &mut BezPath, a: (f64, f64), b: (f64, f64)| {
            path.move_to(a);
            path.line_to(b);
        };
        for period in [2, 4] {
            let n = MAX_SAMPLE;
            let width = (n * period) as f64 * 3.0;
            let mut both = BezPath::new();
            let mut shorts = BezPath::new();
            for i in 0..n {
                for k in 0..period - 1 {
                    let x = (i * period + k) as f64 * 3.0;
                    add(&mut both, (x, 0.0), (x + 1.0, 0.0));
                    add(&mut shorts, (x, 0.0), (x + 1.0, 0.0));
                }
                add(&mut both, (0.0, 10.0 + i as f64), (width, 10.0 + i as f64));
            }
            assert_eq!(segments(&both), period * MAX_SAMPLE, "period {period}");
            assert!(turned_outlines(&[shorts]).is_some(), "period {period}: shorts");
            assert!(turned_outlines(&[both]).is_none(), "period {period}: both");
        }
    }

    #[test]
    fn the_load_counts_every_overlapping_pair() {
        let mut rng = Rng(5);
        for n in [0, 1, 2, 7, 60] {
            // Coarse values, so many spans share ends and touch.
            let spans: Vec<(f64, f64)> = (0..n)
                .map(|_| {
                    let a = (rng.unit() * 10.0).round();
                    let b = (rng.unit() * 10.0).round();
                    (a.min(b), a.max(b))
                })
                .collect();
            let by_hand = spans.iter().map(|&(lo, hi)| spans.iter().filter(|&&(s, e)| s <= hi && e >= lo).count() as u64).sum::<u64>();
            let (mut starts, mut ends): (Vec<f64>, Vec<f64>) = spans.iter().copied().unzip();
            assert_eq!(load(&mut starts, &mut ends), by_hand, "{n} spans");
        }
    }

    /// The estimate is made of the fill as it is swept: an open subpath is closed, and its closing segment is a segment of the count and
    /// of the sample, so a path of exactly the gate's segments as open is turned like the same path closed.
    #[test]
    fn the_estimate_reads_the_fill_closed() {
        let mut rng = Rng(9);
        let open = zigzag(1, MIN_SEGMENTS + 1, 1.0, &mut rng, false);
        let closed = {
            let mut c = open.clone();
            c.close_path();
            c
        };
        assert_eq!(segments(&open), MIN_SEGMENTS);
        work::reset();
        assert!(turned_fills(&[(&open, NZ)]).is_some());
        assert_eq!(work::take(Work::TurnReads), MIN_SEGMENTS + 1, "the closing line is counted");
        assert!(turned_fills(&[(&closed, NZ)]).is_some());
        assert_eq!(work::take(Work::TurnReads), MIN_SEGMENTS + 1);
        // One element fewer in the whole is under the gate (a segment is at least an element): nothing is read.
        let short = zigzag(1, MIN_SEGMENTS - 3, 1.0, &mut rng, false);
        assert!(turned_fills(&[(&short, NZ)]).is_none());
        assert_eq!(work::take(Work::TurnReads), 0);
    }

    #[test]
    fn turning_and_turning_back_moves_no_point() {
        let mut rng = Rng(11);
        let p = zigzag(2, 50, 3.0, &mut rng, true);
        assert_eq!(unturn(TURN * &p), p);
        let bp = outline(&p);
        assert_eq!(UNTURN * (TURN * &bp), bp);
    }

    #[test]
    fn turned_results_match_the_upright_sweep_on_random_paths() {
        let mut rng = Rng(0x9e37_79b9_7f4a_7c15);
        for case in 0..3 {
            let rows = 2 + case;
            let low = zigzag(rows, 120 + 40 * case, 0.8, &mut rng, false);
            let stroke = [outline(&low)];
            assert!(turned_outlines(&stroke).is_some(), "case {case}: not turned");
            assert_same_region(&stroke_region(&stroke), &stroke_region_upright(&stroke), &format!("case {case}: stroke"));
            assert_same_region(
                &try_stroke_region(&stroke).unwrap(),
                &try_stroke_region_upright(&stroke).unwrap(),
                &format!("case {case}: try_stroke"),
            );
            let fill = zigzag(rows, 1100, 0.8, &mut rng, true);
            let one = [(&fill, NZ)];
            assert!(turned_fills(&one).is_some(), "case {case}: fill not turned");
            assert_same_region(&unite_all_with(&one, DEFAULT_PRECISION), &unite_all_upright(&one, DEFAULT_PRECISION), &format!("case {case}: unite"));
            assert_same_region(
                &try_unite_all_with(&one, DEFAULT_PRECISION).unwrap(),
                &try_unite_all_upright(&one, DEFAULT_PRECISION).unwrap(),
                &format!("case {case}: try_unite"),
            );
        }
    }

    #[test]
    fn tall_and_ordinary_art_give_exactly_the_upright_result() {
        let mut rng = Rng(13);
        let tall = [outline(&upright(&zigzag(3, 300, 0.4, &mut rng, false)))];
        assert_eq!(stroke_region(&tall), stroke_region_upright(&tall), "tall stroke");
        let tall_fill = upright(&zigzag(3, 800, 0.4, &mut rng, true));
        let one = [(&tall_fill, NZ)];
        assert_eq!(unite_all_with(&one, DEFAULT_PRECISION), unite_all_upright(&one, DEFAULT_PRECISION), "tall unite");
        // A square grid of overlapping squares, and a small path below the size that is measured.
        let grid: Vec<BezPath> = (0..900).map(|i| square((i % 30) as f64 * 0.75, (i / 30) as f64 * 0.75)).collect();
        let grid: Vec<(&BezPath, FillRule)> = grid.iter().map(|p| (p, NZ)).collect();
        assert_eq!(unite_all_with(&grid, DEFAULT_PRECISION), unite_all_upright(&grid, DEFAULT_PRECISION), "grid");
        assert_eq!(try_unite_all_with(&grid, DEFAULT_PRECISION).unwrap(), try_unite_all_upright(&grid, DEFAULT_PRECISION).unwrap(), "grid, try");
        let small = [outline(&zigzag(1, 40, 0.4, &mut rng, false))];
        assert_eq!(stroke_region(&small), stroke_region_upright(&small), "small");
    }

    #[test]
    fn edge_cases_give_the_upright_result() {
        // Nothing to sweep.
        assert!(turned_outlines(&[]).is_none());
        assert!(stroke_region(&[]).elements().is_empty());
        assert!(unite_all_with(&[], DEFAULT_PRECISION).elements().is_empty());
        // A single segment.
        let line = BezPath::from_vec(vec![kurbo::PathEl::MoveTo((0.0, 0.0).into()), kurbo::PathEl::LineTo((5_000.0, 0.0).into())]);
        let one = [outline(&line)];
        assert!(turned_outlines(&one).is_none());
        assert_same_region(&stroke_region(&one), &stroke_region_upright(&one), "one segment");

        // No overlaps: a row of separate squares.
        let squares: Vec<BezPath> = (0..600).map(|i| square(i as f64 * 3.0, 0.0)).collect();
        let refs: Vec<(&BezPath, FillRule)> = squares.iter().map(|p| (p, NZ)).collect();
        assert!(turned_fills(&refs).is_some());
        let apart = unite_all_with(&refs, DEFAULT_PRECISION);
        assert_eq!(subpath_count(&apart), 600);
        assert_same_region(&apart, &unite_all_upright(&refs, DEFAULT_PRECISION), "separate squares");

        // All anchors on one line: no area either way.
        let flat: BezPath = {
            let mut p = BezPath::new();
            p.move_to((0.0, 0.0));
            for i in 1..2100 {
                p.line_to((i as f64, 0.0));
            }
            p.close_path();
            p
        };
        let flat = [(&flat, NZ)];
        assert!(turned_fills(&flat).is_some());
        assert!(unite_all_with(&flat, DEFAULT_PRECISION).elements().is_empty());
        assert!(unite_all_upright(&flat, DEFAULT_PRECISION).elements().is_empty());

        // Coincident segments: the same path twice unites to that path.
        let mut rng = Rng(3);
        let fill = zigzag(2, 1100, 0.0, &mut rng, true);
        let twice = [(&fill, NZ), (&fill, NZ)];
        assert!(turned_fills(&twice).is_some());
        assert_same_region(&unite_all_with(&twice, DEFAULT_PRECISION), &unite_all_upright(&[(&fill, NZ)], DEFAULT_PRECISION), "twice");

        // A point that is not a number, or too far out for the sweep, is not measured, and is the upright operation's to refuse.
        let mut nan = zigzag(1, 2100, 0.0, &mut rng, true);
        nan.move_to((f64::NAN, 0.0));
        nan.line_to((1.0, 1.0));
        assert!(turned_fills(&[(&nan, NZ)]).is_none());
        assert_eq!(try_unite_all_with(&[(&nan, NZ)], DEFAULT_PRECISION), Err(PathOpsError::NonFinite));
    }

    /// One zigzag along the top of a long, low polygon: `cols` straight edges up and down, a closing run along the bottom. Every edge is
    /// on the sweep line while the line is at the top, so sweeping it across costs the square of `cols` and sweeping it along its
    /// length, a few edges a step.
    fn low_polygon(cols: usize) -> BezPath {
        let mut bp = BezPath::new();
        bp.move_to((0.0, 0.0));
        for i in 1..cols {
            bp.line_to((i as f64 * 1.6, if i % 2 == 0 { 0.0 } else { 2.0 }));
        }
        bp.line_to((cols as f64 * 1.6, -5.0));
        bp.line_to((0.0, -5.0));
        bp.close_path();
        bp
    }

    /// The nanoseconds of dry sweep counted while `f` ran, the budget out of the way.
    fn metered(f: impl FnOnce()) -> usize {
        let big = Limits { sweep_work: 1 << 50, ..Limits::default() };
        work::reset();
        with_limits(big, f);
        work::take(Work::MeterNanos)
    }

    /// Where an operation turns its input, the dry sweep meters the sweep that really runs: the turned one. An input the turn makes
    /// cheap is admitted at a budget the same work upright is refused at, on the work counted, and the same geometry already
    /// standing the way it is swept counts that work and no other: nothing is under-counted for the turn.
    #[test]
    fn the_dry_sweep_meters_the_sweep_that_runs_whichever_way_it_runs() {
        let low = low_polygon(3000);
        let tall = TURN * &low;
        let paths = [(&low, NZ)];
        // A union sweeps each input, and then the contours of them all: the work of one sweep of the input, upright and standing.
        let one_sweep = |p: &BezPath| metered(|| drop(normalize_bez_within(p, NZ, usize::MAX).unwrap()));
        let (upright_work, standing_work) = (one_sweep(&low), one_sweep(&tall));
        // The turned sweep, made as an input already standing that way (which is not turned again): the same work in all.
        let standing_in_all = metered(|| {
            assert_eq!(turns_during(|| drop(try_unite_all_with(&[(&tall, NZ)], DEFAULT_PRECISION).unwrap())), 0, "a tall input is not turned");
        });
        // The operation turns the low one: it counts the work of the turned sweep, to the nanosecond.
        let mut turned = 0;
        let chosen_work = metered(|| {
            turned = turns_during(|| drop(try_unite_all_with(&paths, DEFAULT_PRECISION).unwrap()));
        });
        assert_eq!(turned, 1, "the low input is turned");
        assert_eq!(chosen_work, standing_in_all, "what is counted is the turned sweep's, to the nanosecond");
        assert!(upright_work > 3 * standing_work, "turned makes it cheap: {standing_work} ns turned against {upright_work} upright");
        // A budget between them: upright, refused on its counted work; turned, admitted on its.
        let between = Limits { sweep_work: (upright_work + standing_work) / 2, ..Limits::default() };
        assert_eq!(with_limits(between, || try_unite_all_upright(&paths, DEFAULT_PRECISION)), Err(PathOpsError::TooComplex));
        assert!(with_limits(between, || try_unite_all_with(&paths, DEFAULT_PRECISION)).is_ok(), "admitted turned");
        assert!(with_limits(between, || unite_all_with(&paths, DEFAULT_PRECISION)).elements().len() > 3);
        assert!(with_limits(between, || unite_all_upright(&paths, DEFAULT_PRECISION)).elements().is_empty(), "refused upright: no union");
        assert!(with_limits(between, || try_unite_all_with(&[(&tall, NZ)], DEFAULT_PRECISION)).is_ok());
        // The turn admits nothing the turned sweep's own work does not: under that, the operation is refused, turned or not.
        let under = Limits { sweep_work: standing_work / 2, ..Limits::default() };
        assert_eq!(with_limits(under, || try_unite_all_with(&paths, DEFAULT_PRECISION)), Err(PathOpsError::TooComplex));
        assert_eq!(with_limits(under, || try_unite_all_with(&[(&tall, NZ)], DEFAULT_PRECISION)), Err(PathOpsError::TooComplex));
    }

    /// The same for a stroke's outlines, `stroke_region` and its `try_` form.
    #[test]
    fn the_dry_sweep_meters_the_stroke_that_runs_whichever_way_it_runs() {
        let spine = {
            let mut p = BezPath::new();
            p.move_to((0.0, 0.0));
            for i in 1..3000 {
                p.line_to((i as f64 * 1.6, if i % 2 == 0 { 0.0 } else { 2.0 }));
            }
            p
        };
        let low = outline(&spine);
        let tall = TURN * &low;
        let low = [low];
        let tall = [tall];
        let upright_work = metered(|| drop(try_stroke_region_upright(&low).unwrap()));
        let standing_work = metered(|| {
            assert_eq!(turns_during(|| drop(try_stroke_region(&tall).unwrap())), 0);
        });
        let mut turned = 0;
        let chosen_work = metered(|| {
            turned = turns_during(|| drop(try_stroke_region(&low).unwrap()));
        });
        assert_eq!(turned, 1);
        assert_eq!(chosen_work, standing_work, "what is counted is the turned sweep's");
        assert!(upright_work > 2 * standing_work, "{standing_work} ns turned against {upright_work} upright");
        let between = Limits { sweep_work: (upright_work + standing_work) / 2, ..Limits::default() };
        assert_eq!(with_limits(between, || try_stroke_region_upright(&low)), Err(PathOpsError::TooComplex));
        assert!(with_limits(between, || try_stroke_region(&low)).is_ok());
        assert!(!with_limits(between, || stroke_region(&low)).elements().is_empty());
        assert!(with_limits(between, || stroke_region_upright(&low)).elements().is_empty());
        let under = Limits { sweep_work: standing_work / 2, ..Limits::default() };
        assert_eq!(with_limits(under, || try_stroke_region(&low)), Err(PathOpsError::TooComplex));
        assert_eq!(with_limits(under, || try_stroke_region(&tall)), Err(PathOpsError::TooComplex));
    }

    /// The estimate reads the input once, in proportion to its size, and builds the boxes of at most its sample, at chained sizes.
    #[test]
    fn the_estimate_reads_each_segment_once_and_boxes_no_more_than_its_sample() {
        let mut last = None;
        for cols in [3_000usize, 6_000, 12_000, 24_000] {
            let low = [low_polygon(cols)];
            work::reset();
            assert!(turned_outlines(&low).is_some(), "{cols} edges");
            let (reads, boxes) = (work::take(Work::TurnReads), work::take(Work::TurnBoxes));
            assert_eq!(reads, segments(&low[0]), "{cols} edges: every segment read once");
            // One segment from each run of `stride` of them: the runs, to within the one the last may be short of.
            let stride = reads.div_ceil(MAX_SAMPLE);
            assert!(
                boxes <= MAX_SAMPLE && boxes.abs_diff(reads / stride) <= 1,
                "{cols} edges: {boxes} boxes for {reads} segments in runs of {stride}"
            );
            if let Some((last_reads, _)) = last {
                assert!(reads.abs_diff(2 * last_reads) <= 4, "{cols} edges: {reads} segments read after {last_reads}");
            }
            last = Some((reads, boxes));
        }
    }
    /// The estimate is made from [`MIN_SEGMENTS`] segments, exactly (an open zigzag of that many is turned, and of one fewer it is not), for
    /// outlines and for fills (whose closing line is a segment).
    #[test]
    fn the_estimate_is_made_from_exactly_the_segments_of_the_gate() {
        let zig = |segments: usize| {
            let mut bp = BezPath::new();
            bp.move_to((0.0, 0.0));
            for i in 1..=segments {
                bp.line_to((i as f64 * 1.6, if i % 2 == 0 { 0.0 } else { 2.0 }));
            }
            bp
        };
        assert!(turned_outlines(&[zig(MIN_SEGMENTS)]).is_some(), "{MIN_SEGMENTS} segments");
        assert!(turned_outlines(&[zig(MIN_SEGMENTS - 1)]).is_none(), "{} segments", MIN_SEGMENTS - 1);
        // An open path of one fewer, closed by a line back, is the same number of segments of fill.
        assert!(turned_fills(&[(&zig(MIN_SEGMENTS - 1), NZ)]).is_some());
        assert!(turned_fills(&[(&zig(MIN_SEGMENTS - 2), NZ)]).is_none());
    }

    /// One segment is measured from each run of `stride`, at a place that changes from run to run, whatever the count.
    #[test]
    fn one_segment_is_picked_from_each_run_at_a_place_that_changes() {
        for (count, stride) in [(2_048usize, 1usize), (5_000, 2), (4_097, 2), (10_000, 3), (100_000, 25), (1_000_000, 245)] {
            let picked: Vec<usize> = picks(count, stride).collect();
            assert!(picked.windows(2).all(|w| w[0] < w[1]), "{count} in runs of {stride}: ascending");
            let runs = count.div_ceil(stride);
            assert!(picked.len() == runs || picked.len() + 1 == runs, "{count} in runs of {stride}: {} picked of {runs} runs", picked.len());
            for &i in &picked {
                assert!(i < count);
                assert_eq!(i % stride, sample_offset(i / stride, stride), "{count} in runs of {stride}: index {i}");
            }
            // Every run is represented but a last one that is short of its place; and the places differ (stride 1 has the one place).
            assert!(picked.iter().enumerate().all(|(run, &i)| i / stride == run), "{count} in runs of {stride}: one for each run");
            if stride > 1 {
                let places: std::collections::HashSet<usize> = picked.iter().map(|i| i % stride).collect();
                assert!(places.len() > stride.min(8) / 2, "{count} in runs of {stride}: {} places", places.len());
            }
        }
    }

    /// Turning pays where the upright load is more than one and a half times the turned one, strictly.
    #[test]
    fn the_turn_pays_past_the_gain_and_not_at_it() {
        assert!(!gain_pays(4, 6), "exactly one and a half times does not pay");
        assert!(gain_pays(4, 7));
        assert!(!gain_pays(2, 3));
        assert!(gain_pays(2, 4));
        assert!(!gain_pays(0, 0));
        assert!(!gain_pays(10, 10));
        assert!(!gain_pays(10, 1));
    }
    /// The place measured in a run is the SplitMix64 finalizer of the run's number, as VectorCraft takes it (its test pins nothing: the same
    /// places are measured, so the estimate decides as VectorCraft's does on the same input). Values computed by hand from the finalizer.
    #[test]
    fn the_places_measured_are_those_of_the_splitmix_finalizer() {
        assert_eq!((0..6).map(|run| sample_offset(run, 1000)).collect::<Vec<_>>(), [535, 465, 110, 53, 978, 618]);
        assert_eq!((0..6).map(|run| sample_offset(run, 7)).collect::<Vec<_>>(), [2, 2, 4, 2, 6, 3]);
        assert_eq!(sample_offset(123_456, 245), 102);
        assert_eq!(sample_offset((1 << 40) + 17, 3), 0);
    }
}
