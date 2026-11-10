// Carved from VectorCraft's crates/pathops (src/boolean.rs: `comes_close` and what it calls, 4060f14) at storytold/vectorcraft ef81d550e47623a6922fc14574eee66cc5fed083, on `BezPath`.
// Copyright (c) 2026 ArtCraft Team and the VectorCraft contributors. MIT OR Apache-2.0.

//! Do two segments of a sweep's contours lie within a distance of each other? A union of one input is not swept a second time when its
//! own sweep left its segments as they are, unless two of them come this close ([`comes_close`]): a sweep joins segments that lie within
//! its tolerance of each other or leaves them apart depending on the order it meets them in, and the second sweep can answer differently
//! from the first.
//!
//! The segments are sorted along the longer side of the whole and each is looked at with those that follow it as far as they reach: at most
//! [`MAX_CLOSE_PAIRS`] pairs for each segment, and of those only the pairs whose extents also meet across are compared, at most
//! [`MAX_CLOSE_COMPARED`] of them. Past either the answer is yes (the union is swept again, which costs only that sweep). Every pair is
//! counted ([`Work::ClosePairs`], [`Work::CloseCompared`]) before it is looked at or compared.

use kurbo::{CubicBez, Point};
use linesweeper::topology::Contours;

use crate::path::Seg;
use crate::work::{self, Work};

/// The pairs of segments [`comes_close`] compares for each segment, as VectorCraft does. Past it the answer is yes.
const MAX_CLOSE_PAIRS: usize = 512;

/// The pairs [`comes_close`] compares in all (those whose extents meet both ways, the others being turned away at once): a comparison takes 54
/// nanoseconds at the most (measured, in a release build, for the pair that needs every direction of [`hulls_apart`]), so the bound is a
/// quarter of a second. VectorCraft has none beyond its 512 pairs for each segment, which for the 524,288 segments the element limit admits
/// would be 268 million pairs, up to 14 s of comparisons. The pairs looked at and turned away take a nanosecond (the 512 for each segment
/// are 0.3 s at the most), and are not bounded further.
pub(crate) const MAX_CLOSE_COMPARED: usize = 1 << 22;

/// One segment of a contour for [`comes_close`]: its control points' extents along and across the side the segments are sorted on, and its
/// place in its contour.
struct CloseSeg {
    c: CubicBez,
    line: bool,
    lo: f64,
    hi: f64,
    across_lo: f64,
    across_hi: f64,
    contour: usize,
    index: usize,
    len: usize,
}

impl CloseSeg {
    fn new(seg: Seg, contour: usize, index: usize, len: usize) -> Self {
        let pts = [seg.c.p0, seg.c.p1, seg.c.p2, seg.c.p3];
        let (lo, hi) = pts.iter().fold((f64::INFINITY, f64::NEG_INFINITY), |(l, h), p| (l.min(p.x), h.max(p.x)));
        let (across_lo, across_hi) = pts.iter().fold((f64::INFINITY, f64::NEG_INFINITY), |(l, h), p| (l.min(p.y), h.max(p.y)));
        Self { c: seg.c, line: seg.line, lo, hi, across_lo, across_hi, contour, index, len }
    }

    /// Do the segments `self` and `other` follow each other along one contour (the last of a contour is followed by its first)?
    fn follows_or_is_followed_by(&self, other: &CloseSeg) -> bool {
        self.contour == other.contour && ((self.index + 1) % self.len.max(1) == other.index || (other.index + 1) % other.len.max(1) == self.index)
    }
}

/// Does a segment of the contours `c` lie within `touch` of another? Segments whose control points can't be told apart from `touch` count as
/// close, which costs only the second sweep. Past [`MAX_CLOSE_PAIRS`] pairs for each segment, or [`MAX_CLOSE_COMPARED`] comparisons in all, the answer is yes.
pub(crate) fn comes_close(c: &Contours, touch: f64) -> bool {
    comes_close_within(c, touch, MAX_CLOSE_COMPARED)
}

/// [`comes_close`] with the bound on the comparisons given (a test lowers it).
fn comes_close_within(c: &Contours, touch: f64, compared: usize) -> bool {
    let mut segs = Vec::new();
    for (contour, ct) in c.contours().enumerate() {
        let len = ct.path.segments().count();
        for (index, s) in ct.path.segments().enumerate() {
            segs.push(CloseSeg::new(Seg::straight(s), contour, index, len));
        }
    }
    work::add(Work::CloseSegments, segs.len());
    any_close(segs, touch, compared)
}

/// Do two of the segments `segs` lie within `touch` of each other, looking at most 512 pairs for each segment and comparing at most
/// `max_compared` in all (past either: yes)?
fn any_close(mut segs: Vec<CloseSeg>, touch: f64, max_compared: usize) -> bool {
    if segs.is_empty() {
        return false;
    }
    // Sort along the longer side of the whole, where fewer segments overlap.
    let extent = |lo: fn(&CloseSeg) -> f64, hi: fn(&CloseSeg) -> f64| {
        segs.iter().map(hi).fold(f64::NEG_INFINITY, f64::max) - segs.iter().map(lo).fold(f64::INFINITY, f64::min)
    };
    if extent(|s| s.across_lo, |s| s.across_hi) > extent(|s| s.lo, |s| s.hi) {
        for s in &mut segs {
            std::mem::swap(&mut s.lo, &mut s.across_lo);
            std::mem::swap(&mut s.hi, &mut s.across_hi);
        }
    }
    segs.sort_by(|a, b| a.lo.total_cmp(&b.lo));
    let (mut pairs, mut compared, budget) = (0usize, 0usize, segs.len().saturating_mul(MAX_CLOSE_PAIRS));
    for (i, a) in segs.iter().enumerate() {
        for b in segs.iter().skip(i + 1) {
            if b.lo > a.hi + touch {
                break;
            }
            // Counted before the pair is compared.
            pairs += 1;
            work::add(Work::ClosePairs, 1);
            if pairs > budget {
                return true;
            }
            if a.across_lo > b.across_hi + touch || b.across_lo > a.across_hi + touch {
                continue;
            }
            // Counted before the pair is compared.
            compared += 1;
            work::add(Work::CloseCompared, 1);
            if compared > max_compared {
                return true;
            }
            let close = if a.follows_or_is_followed_by(b) { neighbors_run_together(a, b, touch) } else { !hulls_apart(&a.c, &b.c, touch) };
            if close {
                return true;
            }
        }
    }
    false
}

/// The sine of the angle below which two segments leaving a shared end count as parallel.
const PARALLEL_SINE: f64 = 1e-3;

/// Do segments `a` and `b`, which share an end, run along each other within `touch`? Straight segments do when the far end of either lies
/// within `touch` of the other. A curve does when the two leave their shared end in the same direction, or when their extents overlap by more
/// than `touch` both ways.
fn neighbors_run_together(a: &CloseSeg, b: &CloseSeg, touch: f64) -> bool {
    use kurbo::ParamCurveNearest;
    if a.line && b.line {
        let (shared, p, q) = if a.c.p3 == b.c.p0 { (a.c.p3, a.c.p0, b.c.p3) } else { (a.c.p0, a.c.p3, b.c.p0) };
        let near = |p: Point, q: Point| kurbo::Line::new(shared, q).nearest(p, 0.0).distance_sq.sqrt() <= touch;
        return near(q, p) || near(p, q);
    }
    let (from_a, from_b) = if a.c.p3 == b.c.p0 {
        ([a.c.p3, a.c.p2, a.c.p1, a.c.p0], [b.c.p0, b.c.p1, b.c.p2, b.c.p3])
    } else {
        ([a.c.p0, a.c.p1, a.c.p2, a.c.p3], [b.c.p3, b.c.p2, b.c.p1, b.c.p0])
    };
    let away = |pts: [Point; 4]| pts.iter().skip(1).map(|p| *p - pts[0]).find(|v| v.hypot2() > 0.0);
    if let (Some(u), Some(v)) = (away(from_a), away(from_b))
        && u.dot(v) > 0.0
        && u.cross(v).abs() <= PARALLEL_SINE * u.hypot() * v.hypot()
    {
        return true;
    }
    a.hi.min(b.hi) - a.lo.max(b.lo) > touch && a.across_hi.min(b.across_hi) - a.across_lo.max(b.across_lo) > touch
}

/// Is there a direction, along or across a side of either segment's control polygon or chord, in which the control points of `a` and `b` lie
/// more than `touch` apart?
fn hulls_apart(a: &CubicBez, b: &CubicBez, touch: f64) -> bool {
    let (pa, pb) = ([a.p0, a.p1, a.p2, a.p3], [b.p0, b.p1, b.p2, b.p3]);
    let apart = |d: kurbo::Vec2| {
        let span =
            |pts: &[Point; 4]| pts.iter().map(|p| p.to_vec2().dot(d)).fold((f64::INFINITY, f64::NEG_INFINITY), |(l, h), v| (l.min(v), h.max(v)));
        let ((la, ha), (lb, hb)) = (span(&pa), span(&pb));
        let gap = touch * d.hypot();
        gap > 0.0 && (lb - ha > gap || la - hb > gap)
    };
    let sides = |p: [Point; 4]| [p[3] - p[0], p[1] - p[0], p[2] - p[1], p[3] - p[2]];
    sides(pa).into_iter().chain(sides(pb)).any(|d| apart(d) || apart(kurbo::Vec2::new(-d.y, d.x)))
}

#[cfg(test)]
mod tests {
    use kurbo::{BezPath, Shape as _};

    use super::*;
    use crate::FillRule;
    use crate::boolean::normalize_bez;
    use crate::testutil::polyline;

    /// The contours a sweep makes of `path`.
    fn contours_of(path: &BezPath) -> Contours {
        normalize_bez(path, FillRule::NonZero).expect("a sweep")
    }

    /// `n` squares 10 wide, `gap` apart, in a row.
    fn row(n: usize, gap: f64) -> BezPath {
        let mut bp = BezPath::new();
        for i in 0..n {
            bp.extend(kurbo::Rect::new(i as f64 * (10.0 + gap), 0.0, i as f64 * (10.0 + gap) + 10.0, 10.0).path_elements(0.1));
        }
        bp
    }

    /// `n` thin bars, each a parallelogram running 10,000 along the diagonal, 5 apart (3.5 across): the bounds of every two overlap both
    /// ways, and no two come near.
    fn bars(n: usize) -> BezPath {
        let mut bp = BezPath::new();
        for i in 0..n {
            let y = i as f64 * 5.0;
            bp.extend(polyline(&[(0.0, y), (10_000.0, y + 10_000.0), (10_000.0, y + 10_001.0), (0.0, y + 1.0)], true).iter());
        }
        bp
    }

    fn line(a: (f64, f64), b: (f64, f64)) -> CloseSeg {
        CloseSeg::new(Seg::line(a.into(), b.into()), 0, 0, 2)
    }

    fn curve(p: [(f64, f64); 4]) -> CloseSeg {
        CloseSeg::new(Seg { c: CubicBez::new(p[0], p[1], p[2], p[3]), line: false }, 0, 0, 2)
    }

    /// VectorCraft's `neighboring_segments_run_together_when_one_lies_along_the_other`.
    #[test]
    fn neighboring_segments_run_together_when_one_lies_along_the_other() {
        let touch = 2e-6;
        // The two sides of a needle's tip, and an ordinary corner.
        assert!(neighbors_run_together(&line((10.0, 10.0), (30.0, 30.0)), &line((30.0, 30.0), (10.0, 10.000001)), touch));
        assert!(!neighbors_run_together(&line((0.0, 0.0), (10.0, 0.0)), &line((10.0, 0.0), (10.0, 10.0)), touch));
        // Curves that fold back over each other, and two quarters of a circle.
        let out = curve([(0.0, 0.0), (5.0, 8.0), (10.0, 8.0), (20.0, 0.0)]);
        assert!(neighbors_run_together(&out, &curve([(20.0, 0.0), (10.0, 6.0), (5.0, 6.0), (0.0, 1.0)]), touch));
        // A curved hairpin that lies within the tolerance along its length.
        let there = curve([(0.0, 0.0), (5.0, 1e-7), (15.0, 1e-7), (20.0, 0.0)]);
        assert!(neighbors_run_together(&there, &curve([(20.0, 0.0), (15.0, 2e-7), (5.0, 2e-7), (0.0, 1e-6)]), touch));
        let quarter = curve([(10.0, 0.0), (10.0, 5.5), (5.5, 10.0), (0.0, 10.0)]);
        assert!(!neighbors_run_together(&quarter, &curve([(0.0, 10.0), (-5.5, 10.0), (-10.0, 5.5), (-10.0, 0.0)]), touch));
    }

    /// Contours that lie 4e-6 apart do not come close at twice a tolerance of 1e-6 (the segments of the squares are 4 apart in each
    /// pair), 1.9e-6 apart do, and a square's own corner (two segments sharing an end) does not.
    #[test]
    fn contours_come_close_within_twice_the_tolerance_and_not_beyond() {
        let touch = 2e-6;
        assert!(!comes_close(&contours_of(&row(2, 4e-6)), touch), "4e-6 apart is not close at 2e-6");
        assert!(comes_close(&contours_of(&row(2, 1.9e-6)), touch), "1.9e-6 apart is close");
        assert!(!comes_close(&contours_of(&row(1, 0.0)), touch), "one square: its corners are not close");
        assert!(!comes_close(&contours_of(&BezPath::new()), touch), "nothing is not close");
    }

    /// Every pair is counted before it is looked at, and no more are looked at than 512 for each segment: past that the
    /// answer is yes, and for no other reason (no two bars are near). Chained at 100, 200, 400 and 700 bars (4 segments each; the most the sweep
    /// admits) that overlap both ways, so that most pairs of segments are ones to compare: the pairs grow with the square of the segments
    /// until they pass the budget, and are the budget and one after it.
    #[test]
    fn the_pairs_compared_are_counted_and_held_to_a_budget() {
        let mut ran_out = Vec::new();
        for n in [100usize, 200, 400, 700] {
            let contours = contours_of(&bars(n));
            let segments = 4 * n;
            work::reset();
            let close = comes_close(&contours, 2e-6);
            assert_eq!(work::take(Work::CloseSegments), segments, "{n} bars: the segments read");
            let (pairs, compared) = (work::take(Work::ClosePairs), work::take(Work::CloseCompared));
            let budget = segments * MAX_CLOSE_PAIRS;
            assert!(pairs <= budget + 1, "{n} bars: {pairs} pairs for a budget of {budget}");
            assert!(pairs > segments * segments / 8, "{n} bars: {pairs} pairs is not most of the pairs of {segments} segments");
            assert_eq!(close, pairs == budget + 1, "{n} bars: close when the budget ran out and for no other reason");
            // Every pair of bars meets across too: each pair looked at is compared, the last one not when the budget ran out at it.
            assert_eq!(compared, pairs - usize::from(close), "{n} bars: the pairs compared");
            ran_out.push(close);
        }
        assert_eq!(ran_out, [false, false, false, true], "the budget runs out at the largest of these and at no other");
    }

    /// The bound on the comparisons is a bound of its own: lowered to 500, the answer is yes at the 501st; and with it lifted the check ends at
    /// the last pair, of which none is close.
    #[test]
    fn the_comparisons_are_bounded() {
        let contours = contours_of(&bars(100));
        let counts = || (work::take(Work::ClosePairs), work::take(Work::CloseCompared));
        work::reset();
        assert!(comes_close_within(&contours, 2e-6, 500));
        assert_eq!(counts(), (501, 501));
        assert!(!comes_close_within(&contours, 2e-6, usize::MAX), "no pair of bars is close");
        let (pairs, compared) = counts();
        assert!(pairs > 501 && pairs == compared, "{pairs} pairs, {compared} compared: all of them");
        assert_eq!(MAX_CLOSE_COMPARED, 1 << 22);
    }

    /// `rows` rows of 20 squares from `(x0, 0)`, 15 apart, transposed when `wide`: taller than wide (or wider than tall), so that the segments are
    /// sorted along the long side, and the segments of one row overlap there without meeting across (a square's neighbours are 5 away).
    fn grid(rows: usize, x0: f64, wide: bool) -> BezPath {
        let mut bp = BezPath::new();
        for i in 0..rows * 20 {
            let (x, y) = ((i % 20) as f64 * 15.0, (i / 20) as f64 * 15.0);
            let (x, y) = if wide { (y, x) } else { (x, y) };
            bp.extend(kurbo::Rect::new(x0 + x, y, x0 + x + 10.0, y + 10.0).path_elements(0.1));
        }
        bp
    }

    /// Pairs that overlap on the side the segments are sorted on and not across are turned away before they are compared, and only counted:
    /// 200 rows of 20 squares have 552,000 pairs looked at and 16,000 compared, and the bound on the comparisons is not the one that they are
    /// held to. The side is the longer one whatever the coordinates (the grid moved 5,000 along x, where the sum of the ends of its x side is
    /// the larger, and the grid on its side give the same).
    #[test]
    fn pairs_whose_extents_do_not_meet_across_are_counted_and_not_compared() {
        for (x0, wide) in [(0.0, false), (5000.0, false), (0.0, true), (5000.0, true)] {
            let contours = contours_of(&grid(200, x0, wide));
            work::reset();
            assert!(!comes_close(&contours, 2e-6), "no two squares come near");
            assert_eq!(work::take(Work::CloseSegments), 16_000, "the segments read");
            let (pairs, compared) = (work::take(Work::ClosePairs), work::take(Work::CloseCompared));
            assert_eq!((pairs, compared), (552_000, 16_000), "grid at {x0}, wide {wide}: pairs looked at, compared");
        }
    }

    /// `bars` thin bars as segments (4 each, in one contour each), each running 10,000 along the diagonal and 5 apart: every pair of them
    /// overlaps both ways, and none is near another.
    fn bar_segments(bars: usize) -> Vec<CloseSeg> {
        let mut segs = Vec::new();
        for i in 0..bars {
            let y = i as f64 * 5.0;
            let corners = [(0.0, y), (10_000.0, y + 10_000.0), (10_000.0, y + 10_001.0), (0.0, y + 1.0)];
            for k in 0..4 {
                segs.push(CloseSeg::new(Seg::line(corners[k].into(), corners[(k + 1) % 4].into()), i, k, 4));
            }
        }
        segs
    }

    /// The worst the comparisons can cost, more segments than a sweep admits to overlap (so made by hand, with no sweep): 2,200 bars of 4
    /// segments make 8,800, every pair one to compare and none near. The answer is yes at the first comparison past the bound, in a fraction
    /// of a second.
    #[test]
    #[ignore = "seconds: run with cargo test --release"]
    fn the_worst_case_of_the_comparisons_is_bounded_in_time() {
        let segs = bar_segments(2200);
        assert_eq!(segs.len(), 8800);
        work::reset();
        let started = std::time::Instant::now();
        assert!(any_close(segs, 2e-6, MAX_CLOSE_COMPARED));
        let took = started.elapsed().as_secs_f64();
        assert_eq!((work::take(Work::ClosePairs), work::take(Work::CloseCompared)), (MAX_CLOSE_COMPARED + 1, MAX_CLOSE_COMPARED + 1));
        eprintln!("{MAX_CLOSE_COMPARED} comparisons took {took:.3} s");
        assert!(took < 1.0, "{took} s");
    }

    /// The worst the looking can cost: 140,000 lines 1,000,000 long and 5 apart, every pair overlapping on the side they are sorted on and none
    /// meeting across: the 512 pairs for each of them are 71.7 million, and the answer is yes at the first pair past that.
    #[test]
    #[ignore = "seconds: run with cargo test --release"]
    fn the_worst_case_of_the_looking_is_bounded_in_time() {
        let segs: Vec<CloseSeg> =
            (0..140_000).map(|i| CloseSeg::new(Seg::line((0.0, i as f64 * 5.0).into(), (1e6, i as f64 * 5.0).into()), i, 0, 1)).collect();
        work::reset();
        let started = std::time::Instant::now();
        assert!(any_close(segs, 2e-6, MAX_CLOSE_COMPARED));
        let took = started.elapsed().as_secs_f64();
        assert_eq!((work::take(Work::ClosePairs), work::take(Work::CloseCompared)), (140_000 * MAX_CLOSE_PAIRS + 1, 0));
        eprintln!("{} pairs looked at took {took:.3} s", 140_000 * MAX_CLOSE_PAIRS);
        assert!(took < 1.0, "{took} s");
    }

    /// What one comparison can cost at the most: `hulls_apart` of two nested quarter circles, whose hulls no direction tells apart, reads all 16
    /// directions. The whole bound of comparisons at that cost is a fraction of a second (a check ends at the first such pair, which is close).
    #[test]
    #[ignore = "seconds: run with cargo test --release"]
    fn the_most_one_comparison_costs_keeps_the_check_under_a_second() {
        const K: f64 = 0.552_284_749_830_793_6;
        let arc = |r: f64| CubicBez::new((r, 0.0), (r, K * r), (K * r, r), (0.0, r));
        let (a, b) = (arc(100.0), arc(100.5));
        let started = std::time::Instant::now();
        let mut apart = 0usize;
        for i in 0..MAX_CLOSE_COMPARED {
            // The arguments change a little so that the loop is not folded away.
            apart += usize::from(hulls_apart(&a, &b, 2e-6 + i as f64 * 1e-18));
        }
        let took = started.elapsed().as_secs_f64();
        eprintln!("{MAX_CLOSE_COMPARED} comparisons of nested arcs: {apart} apart, {took:.3} s");
        assert_eq!(apart, 0, "nested hulls are never apart");
        assert!(took < 1.0, "{took} s");
    }

    /// A segment of its own contour (so that it is nobody's neighbour).
    fn alone(a: (f64, f64), b: (f64, f64), contour: usize) -> CloseSeg {
        CloseSeg::new(Seg::line(a.into(), b.into()), contour, 0, 1)
    }

    /// Segments follow each other along a contour, the last being followed by the first, and not along different contours.
    #[test]
    fn segments_follow_each_other_along_a_contour_and_round_it_and_not_across_contours() {
        let at = |contour: usize, index: usize| CloseSeg::new(Seg::line((0.0, 0.0).into(), (1.0, 1.0).into()), contour, index, 4);
        assert!(at(0, 0).follows_or_is_followed_by(&at(0, 1)) && at(0, 1).follows_or_is_followed_by(&at(0, 0)));
        assert!(at(0, 3).follows_or_is_followed_by(&at(0, 0)) && at(0, 0).follows_or_is_followed_by(&at(0, 3)), "the last and the first");
        assert!(!at(0, 0).follows_or_is_followed_by(&at(0, 2)) && !at(0, 1).follows_or_is_followed_by(&at(0, 3)), "two apart");
        assert!(
            !at(0, 0).follows_or_is_followed_by(&at(1, 1)) && !at(1, 3).follows_or_is_followed_by(&at(0, 0)),
            "the next index of another contour"
        );
    }

    /// Segments exactly `touch` apart are close (a pair is looked at at that distance, and its hulls are not apart), and not a hair more apart,
    /// along the side they are sorted on, across it, and with either drawn the other way (the hulls are compared in both directions of a side).
    /// `touch` of a half and coordinates of halves keep every product exact.
    #[test]
    fn segments_exactly_the_touch_apart_are_close_and_a_hair_more_apart_are_not() {
        let touch = 0.5;
        let hair = 0.500_000_1;
        let any = |segs: Vec<CloseSeg>| any_close(segs, touch, usize::MAX);
        for (gap, close) in [(touch, true), (hair, false)] {
            // Along: the second starts `gap` after the first ends, level with it.
            assert_eq!(any(vec![alone((0.0, 0.0), (10.0, 0.0), 0), alone((10.0 + gap, 0.0), (20.0 + gap, 0.0), 1)]), close, "along, {gap}");
            // The second drawn the other way: the first is the one that is further along the direction of a side of the second.
            assert_eq!(any(vec![alone((0.0, 0.0), (10.0, 0.0), 0), alone((20.0 + gap, 0.0), (10.0 + gap, 0.0), 1)]), close, "along, reversed, {gap}");
            // Across: the second `gap` above the first, and the first `gap` above the second.
            assert_eq!(any(vec![alone((0.0, 0.0), (10.0, 0.0), 0), alone((0.0, gap), (10.0, gap), 1)]), close, "across, {gap}");
            assert_eq!(any(vec![alone((0.0, gap), (10.0, gap), 0), alone((0.0, 0.0), (10.0, 0.0), 1)]), close, "across, the other way, {gap}");
        }
    }

    /// A line and a curve that are neighbours are told by the curve's rule, not the line's (which reads the ends only): a line along the bottom
    /// of a cubic that leaves its end upwards, and comes back to the line's other end within the touch, is not along it.
    #[test]
    fn neighbours_that_are_a_line_and_a_curve_are_judged_as_curves() {
        let touch = 2e-6;
        let a = line((0.0, 0.0), (10.0, 0.0));
        let b = curve([(10.0, 0.0), (5.0, 5.0), (3.0, 5.0), (0.0, 1e-7)]);
        assert!(!neighbors_run_together(&a, &b, touch) && !neighbors_run_together(&b, &a, touch));
    }

    /// Of two straight neighbours one need only lie along the other: the short one runs along the long one though the far end of the long one
    /// is nowhere near the short one.
    #[test]
    fn a_short_line_along_a_long_neighbour_runs_together_with_it() {
        let touch = 2e-6;
        assert!(neighbors_run_together(&line((0.0, 0.0), (10.0, 0.0)), &line((10.0, 0.0), (9.0, 1e-7)), touch));
        // And not one that leaves at an angle.
        assert!(!neighbors_run_together(&line((0.0, 0.0), (10.0, 0.0)), &line((10.0, 0.0), (9.0, 1.0)), touch));
    }

    /// Curves with handles on their ends (a first or second control point at an end) leave it along the first handle that is not there, so two
    /// such curves folded back over each other are along each other.
    #[test]
    fn a_handle_at_the_end_is_skipped_for_the_direction_a_curve_leaves_by() {
        let touch = 2e-6;
        let a = curve([(0.0, 0.0), (0.0, 0.0), (10.0, 0.0), (10.0, 0.0)]);
        let b = curve([(10.0, 0.0), (10.0, 0.0), (0.0, 1e-7), (0.0, 1e-7)]);
        assert!(neighbors_run_together(&a, &b, touch));
    }

    /// Curves that leave their shared end at an angle whose sine is 5e-4 are parallel and at 2e-3 are not (the bound is 1e-3), by direction
    /// alone: their extents do not meet across.
    #[test]
    fn curves_leaving_an_end_within_a_thousandth_of_a_sine_are_parallel() {
        let touch = 2e-6;
        let a = curve([(0.0, 0.0), (2.5, 0.0), (7.5, 0.0), (10.0, 0.0)]);
        // From (10, 0) towards (7.5, h): a sine of h / 2.5 about.
        let leaving = |h: f64| curve([(10.0, 0.0), (7.5, h), (5.0, 0.0), (0.0, 0.0)]);
        assert!(neighbors_run_together(&a, &leaving(1.25e-3), touch), "5e-4");
        assert!(!neighbors_run_together(&a, &leaving(5e-3), touch), "2e-3");
    }

    /// The extents of two neighbouring curves must overlap by more than the touch, both ways, to be along each other: by exactly the touch they
    /// are not (the first leaves the shared end straight up, the second a little to the left of up).
    #[test]
    fn neighbouring_curves_overlapping_by_exactly_the_touch_are_not_along_each_other() {
        let a = curve([(0.0, 0.0), (0.0, 5.0), (10.0, 5.0), (10.0, 0.0)]);
        // Along x the second reaches 0.5 back over the first (to 9.5): by the touch of a half, exactly.
        let b = curve([(10.0, 0.0), (9.5, 2.0), (15.0, 5.0), (20.0, 0.0)]);
        assert!(!neighbors_run_together(&a, &b, 0.5));
        assert!(neighbors_run_together(&a, &b, 0.499_999));
        // The same turned on its side: across.
        let turn = |c: &CloseSeg| curve([(c.c.p0.y, c.c.p0.x), (c.c.p1.y, c.c.p1.x), (c.c.p2.y, c.c.p2.x), (c.c.p3.y, c.c.p3.x)]);
        assert!(!neighbors_run_together(&turn(&a), &turn(&b), 0.5));
        assert!(neighbors_run_together(&turn(&a), &turn(&b), 0.499_999));
    }
}
