// Carved from VectorCraft's crates/pathops (the anchor-based `PathData` model its operations used, replaced here by
// `kurbo::BezPath`) and crates/geom (`flatten`) at storytold/vectorcraft ff9deee46d1c59dc9ecef0b668c25e57cd80b55e.
// Copyright (c) 2026 ArtCraft Team and the VectorCraft contributors. MIT OR Apache-2.0.

//! Reading a [`BezPath`] the way the operations need it: as subpaths of segments, each open or
//! closed, and writing a chain of segments back out.
//!
//! VectorCraft's operations worked on an anchor model (a point with two handles) whose conversions
//! to and from `BezPath` fixed a few rules the operations rely on, kept here:
//!
//! - a subpath with no segment (a lone `MoveTo`) draws nothing and is dropped;
//! - a closed subpath whose last point lies within [`JOIN_TOLERANCE`] of its start ends on the start
//!   exactly, and its closing segment is written out (so a closed subpath ends with a segment that
//!   arrives at its start, then `ClosePath`);
//! - a cubic is a straight line when neither of its handles leaves its end ([`HANDLE_EPS`]).

use kurbo::{BezPath, CubicBez, PathEl, PathSeg, Point};

use crate::fit::is_straight;
use crate::work::{self, Work};

/// A handle closer than this to its anchor is no handle (the segment is a line).
pub(crate) const HANDLE_EPS: f64 = 1e-9;

/// The last point of a closed subpath this close to its start is the start (the subpath's closing
/// segment arrives there, and the duplicate point is merged into it).
pub(crate) const JOIN_TOLERANCE: f64 = 1e-7;

/// One subpath of a path: where it starts, the segments that follow, and whether it was closed.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Sub {
    pub(crate) start: Point,
    /// The `LineTo`, `QuadTo` and `CurveTo` elements, in order. Never empty.
    pub(crate) draws: Vec<PathEl>,
    pub(crate) closed: bool,
}

/// The subpaths of `bp` that have a segment. A lone `MoveTo` and a `MoveTo` followed only by
/// `ClosePath` draw nothing and are dropped; segments after a `ClosePath` with no `MoveTo` start
/// from the closed subpath's start, as kurbo reads them. A quadratic is raised to a cubic, as an anchor model has
/// only cubics, so everything the operations sweep is lines and cubics.
pub(crate) fn subs(bp: &BezPath) -> Vec<Sub> {
    work::add(Work::Visited, bp.elements().len());
    let mut out = Vec::new();
    let (mut start, mut cur) = (Point::ZERO, Point::ZERO);
    let mut draws: Vec<PathEl> = Vec::new();
    let finish = |start: Point, draws: &mut Vec<PathEl>, closed: bool, out: &mut Vec<Sub>| {
        if !draws.is_empty() {
            out.push(Sub { start, draws: std::mem::take(draws), closed });
        }
    };
    for &el in bp.elements() {
        match el {
            PathEl::MoveTo(p) => {
                finish(start, &mut draws, false, &mut out);
                (start, cur) = (p, p);
            }
            PathEl::ClosePath => {
                finish(start, &mut draws, true, &mut out);
                cur = start;
            }
            PathEl::LineTo(p) => {
                draws.push(el);
                cur = p;
            }
            PathEl::QuadTo(c, p) => {
                let raised = kurbo::QuadBez::new(cur, c, p).raise();
                draws.push(PathEl::CurveTo(raised.p1, raised.p2, p));
                cur = p;
            }
            PathEl::CurveTo(_, _, p) => {
                draws.push(el);
                cur = p;
            }
        }
    }
    finish(start, &mut draws, false, &mut out);
    out
}

impl Sub {
    /// The end point of the last segment.
    pub(crate) fn end(&self) -> Point {
        self.draws.last().and_then(PathEl::end_point).unwrap_or(self.start)
    }

    /// The number of anchors (points the subpath passes through): one more than its segments, less
    /// the duplicate when it is closed and ends on its start.
    pub(crate) fn anchors(&self) -> usize {
        let n = self.draws.len() + 1;
        if self.closed && self.end().distance(self.start) < JOIN_TOLERANCE { n - 1 } else { n }
    }

    /// An open subpath of three or more anchors whose last anchor lies within 1e-6 of its first (the
    /// test [`join`](crate::join) applies to one path) as a closed subpath: its last segment ends on
    /// the first anchor. Any other subpath is returned unchanged. (A drawing program writes an ellipse
    /// as a path that comes back to where it started without closing.)
    pub(crate) fn closed_if_ends_meet(mut self) -> Self {
        if !self.closed && self.anchors() > 2 && self.end().distance(self.start) < 1e-6 {
            let start = self.start;
            if let Some(last) = self.draws.last_mut() {
                *last = with_end(*last, start);
            }
            self.closed = true;
        }
        self
    }

    /// Does the subpath draw anything? A closed one that is a single segment from its start back to
    /// it (one anchor) draws a point or a line out and back, unless that segment is a cubic whose
    /// handles take it round an area (`M0,0 C100,0 0,100 0,0 Z` is a loop with an inside).
    pub(crate) fn draws_something(&self) -> bool {
        if self.anchors() >= 2 {
            return true;
        }
        let [PathEl::CurveTo(..)] = self.draws[..] else { return false };
        let mut bp = BezPath::new();
        self.push_closed(&mut bp);
        let size = kurbo::Shape::bounding_box(&bp).size();
        kurbo::Shape::area(&bp).abs() > 1e-9 * (size.width * size.width + size.height * size.height)
    }

    /// The segments of the subpath as it is written ([`Sub::push_open`] or [`Sub::push_closed`]),
    /// without the closing segment of a closed one that ends on its start already.
    pub(crate) fn segments(&self) -> impl Iterator<Item = PathSeg> + '_ {
        let mut cur = self.start;
        self.draws.iter().filter_map(move |&el| {
            let seg = match el {
                PathEl::LineTo(p) => PathSeg::Line(kurbo::Line::new(cur, p)),
                PathEl::QuadTo(c, p) => PathSeg::Quad(kurbo::QuadBez::new(cur, c, p)),
                PathEl::CurveTo(a, b, p) => PathSeg::Cubic(CubicBez::new(cur, a, b, p)),
                PathEl::MoveTo(_) | PathEl::ClosePath => return None,
            };
            cur = el.end_point().unwrap_or(cur);
            Some(seg)
        })
    }

    /// The segments of the subpath as a chain, as an anchor model saw it: a closed subpath has a
    /// closing segment back to its start (a line), unless it ends on its start already, when the last
    /// segment is moved to end exactly there. A quadratic is raised to a cubic.
    pub(crate) fn chain(&self) -> Vec<Seg> {
        let mut segs: Vec<Seg> = self.segments().map(Seg::exact).collect();
        if self.closed {
            if self.end().distance(self.start) < JOIN_TOLERANCE {
                if let Some(last) = segs.last_mut() {
                    last.set_end(self.start);
                }
            } else {
                segs.push(Seg::line(self.end(), self.start));
            }
        }
        segs
    }

    /// Appends the subpath as an open one.
    pub(crate) fn push_open(&self, out: &mut BezPath) {
        out.move_to(self.start);
        for &el in &self.draws {
            out.push(el);
        }
    }

    /// Appends the subpath as a closed one, in the form VectorCraft's conversion wrote: a last
    /// point within [`JOIN_TOLERANCE`] of the start is moved onto it, otherwise a line back to the
    /// start is written out, and `ClosePath` follows.
    pub(crate) fn push_closed(&self, out: &mut BezPath) {
        out.move_to(self.start);
        let n = self.draws.len();
        let snap = self.end().distance(self.start) < JOIN_TOLERANCE;
        for (i, &el) in self.draws.iter().enumerate() {
            if snap && i + 1 == n {
                out.push(with_end(el, self.start));
            } else {
                out.push(el);
            }
        }
        if !snap {
            out.line_to(self.start);
        }
        out.close_path();
    }
}

/// `el` ending at `p` instead (its control points stay).
pub(crate) fn with_end(el: PathEl, p: Point) -> PathEl {
    match el {
        PathEl::LineTo(_) => PathEl::LineTo(p),
        PathEl::QuadTo(c, _) => PathEl::QuadTo(c, p),
        PathEl::CurveTo(a, b, _) => PathEl::CurveTo(a, b, p),
        other => other,
    }
}

/// A path to be filled: every subpath with a segment is closed (an open one implicitly, as a filled
/// open path is painted), in the form of [`Sub::push_closed`]. A closed subpath that is one segment
/// from its start back to it has no area and is left out, unless that segment is a cubic that loops
/// round one (VectorCraft's conversion left out every subpath of fewer than two anchors).
pub(crate) fn fill_bezpath(path: &BezPath) -> BezPath {
    let mut out = BezPath::new();
    for sub in subs(path) {
        if sub.closed {
            if !sub.draws_something() {
                continue;
            }
            sub.push_closed(&mut out);
        } else {
            // An open subpath is closed with a line back to its start, even when it ends there.
            out.move_to(sub.start);
            for &el in &sub.draws {
                out.push(el);
            }
            out.line_to(sub.start);
            out.close_path();
        }
    }
    out
}

/// `path` in the form VectorCraft's anchor model wrote it: closed subpaths by [`Sub::push_closed`],
/// open ones as they are, and subpaths without a segment left out.
pub(crate) fn canonical(path: &BezPath) -> BezPath {
    let mut out = BezPath::new();
    for sub in subs(path) {
        if sub.closed {
            sub.push_closed(&mut out);
        } else {
            sub.push_open(&mut out);
        }
    }
    out
}

/// Runs `f`, turning a panic in the sweep (`linesweeper` can panic on rare near-degenerate input)
/// into `None`. On WebAssembly a panic aborts and nothing is caught.
pub(crate) fn guarded<T>(f: impl FnOnce() -> T) -> Option<T> {
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(f)).ok()
}

/// Whether some subpath with a segment is not closed (reads the elements, copies nothing).
pub(crate) fn has_open_subpath(path: &BezPath) -> bool {
    let mut draws = false;
    for el in path.elements() {
        match el {
            PathEl::MoveTo(_) => {
                if draws {
                    return true;
                }
            }
            PathEl::ClosePath => draws = false,
            _ => draws = true,
        }
    }
    draws
}

/// Whether some subpath with a segment is closed (reads the elements, copies nothing).
pub(crate) fn has_closed_subpath(path: &BezPath) -> bool {
    let mut draws = false;
    for el in path.elements() {
        match el {
            PathEl::MoveTo(_) => draws = false,
            PathEl::ClosePath => {
                if draws {
                    return true;
                }
            }
            _ => draws = true,
        }
    }
    false
}

/// The box of all the points of `path`, control points included.
pub(crate) fn control_bounds(path: &BezPath) -> Option<kurbo::Rect> {
    let mut pts = path.elements().iter().flat_map(|el| match *el {
        PathEl::MoveTo(p) | PathEl::LineTo(p) => [Some(p), None, None],
        PathEl::QuadTo(a, p) => [Some(a), Some(p), None],
        PathEl::CurveTo(a, b, p) => [Some(a), Some(b), Some(p)],
        PathEl::ClosePath => [None, None, None],
    });
    let first = pts.by_ref().flatten().next()?;
    Some(pts.flatten().fold(kurbo::Rect::from_points(first, first), |r, p| r.union_pt(p)))
}

/// Most line segments [`flatten`] makes of one curve, whatever its size and the tolerance.
const MAX_FLATTEN_PER_CURVE: f64 = 4096.0;

/// A curve whose control polygon is longer than this is drawn as a line by [`flatten`]: kurbo
/// squares and cubes coordinates when it sizes its work, and with coordinates this large that
/// overflows to infinity and it reserves a buffer no machine has.
const MAX_FLATTEN_LENGTH: f64 = 1e60;

/// [`kurbo::flatten`], each curve's tolerance raised so that it makes at most about
/// [`MAX_FLATTEN_PER_CURVE`] segments (kurbo sizes its work and a buffer from the curve's
/// coordinates before producing anything, so a small curve with far-out handles asks for
/// gigabytes). A curve whose points aren't finite, or are past [`MAX_FLATTEN_LENGTH`], is drawn as a
/// line to its end.
pub(crate) fn flatten(path: impl IntoIterator<Item = PathEl>, tolerance: f64, mut callback: impl FnMut(PathEl)) {
    let mut last = None;
    let mut start = None;
    for el in path {
        let (from, to, length) = match el {
            PathEl::MoveTo(p) => {
                (last, start) = (Some(p), Some(p));
                callback(el);
                continue;
            }
            PathEl::LineTo(p) => {
                last = Some(p);
                callback(el);
                continue;
            }
            PathEl::ClosePath => {
                last = start;
                callback(el);
                continue;
            }
            PathEl::QuadTo(a, b) => (last, b, last.map(|p| p.distance(a) + a.distance(b))),
            PathEl::CurveTo(a, b, c) => (last, c, last.map(|p| p.distance(a) + a.distance(b) + b.distance(c))),
        };
        last = Some(to);
        let (Some(from), Some(length)) = (from, length) else { continue };
        // A short polygon far from the origin still overflows kurbo's `p * (mt * 2.0)` products.
        let far = |p: Point| !(p.x.abs() <= MAX_FLATTEN_LENGTH && p.y.abs() <= MAX_FLATTEN_LENGTH);
        let controls_far = match el {
            PathEl::QuadTo(a, _) => far(a),
            PathEl::CurveTo(a, b, _) => far(a) || far(b),
            _ => false,
        };
        if length.is_nan() || length > MAX_FLATTEN_LENGTH || far(from) || far(to) || controls_far {
            callback(PathEl::LineTo(to));
            continue;
        }
        let tol = tolerance.max(length / (MAX_FLATTEN_PER_CURVE * MAX_FLATTEN_PER_CURVE));
        kurbo::flatten([PathEl::MoveTo(from), el], tol, |e| {
            if !matches!(e, PathEl::MoveTo(_)) {
                callback(e);
            }
        });
    }
}

/// Every control point of `path` is finite.
pub(crate) fn is_finite(path: &BezPath) -> bool {
    work::add(Work::Visited, path.elements().len());
    path.elements().iter().all(|el| match *el {
        PathEl::MoveTo(p) | PathEl::LineTo(p) => p.is_finite(),
        PathEl::QuadTo(a, p) => a.is_finite() && p.is_finite(),
        PathEl::CurveTo(a, b, p) => a.is_finite() && b.is_finite() && p.is_finite(),
        PathEl::ClosePath => true,
    })
}

/// One segment of a chain: a cubic plus whether it is a straight line.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Seg {
    pub(crate) c: CubicBez,
    pub(crate) line: bool,
}

impl Seg {
    pub(crate) fn line(a: Point, b: Point) -> Self {
        Self { c: CubicBez::new(a, a, b, b), line: true }
    }

    /// Moves the start of the segment to `p`; a line stays a line.
    pub(crate) fn set_start(&mut self, p: Point) {
        self.c.p0 = p;
        if self.line {
            self.c.p1 = p;
        }
    }

    /// Moves the end of the segment to `p`; a line stays a line.
    pub(crate) fn set_end(&mut self, p: Point) {
        self.c.p3 = p;
        if self.line {
            self.c.p2 = p;
        }
    }

    /// A segment as it is written: a line stays a line and a curve is a line when neither handle
    /// leaves its end ([`HANDLE_EPS`]); a quadratic is raised to a cubic.
    pub(crate) fn exact(s: PathSeg) -> Self {
        match s {
            PathSeg::Line(l) => Self::line(l.p0, l.p1),
            PathSeg::Quad(q) => Self::exact(PathSeg::Cubic(q.raise())),
            PathSeg::Cubic(c) => Self { c, line: c.p1.distance(c.p0) <= HANDLE_EPS && c.p2.distance(c.p3) <= HANDLE_EPS },
        }
    }

    /// A segment classified by shape: a cubic or quadratic that is straight to within a tolerance
    /// scaled to its length is a line.
    pub(crate) fn straight(s: PathSeg) -> Self {
        match s {
            PathSeg::Line(l) => Self::line(l.p0, l.p1),
            PathSeg::Quad(q) => {
                let c = q.raise();
                Self { c, line: is_straight(&c, 1e-9) }
            }
            PathSeg::Cubic(c) => Self { c, line: is_straight(&c, 1e-9 * (1.0 + c.p0.distance(c.p3))) },
        }
    }
}

/// The path of a chain of segments, open or closed: `None` for an empty chain or a closed one of
/// fewer than two segments.
///
/// Each segment ends where the next one starts (a closed chain's last ends where its first starts),
/// as an anchor model does, so a chain with a gap of a rounding error is joined up. A segment is
/// written as a line when it is one, or when neither of its handles leaves its end.
pub(crate) fn segs_to_bezpath(segs: &[Seg], closed: bool) -> Option<BezPath> {
    let n = segs.len();
    if n == 0 {
        return None;
    }
    // A closed chain of one segment is a loop, if that segment is a curve that comes back to where
    // it starts; a line, or a curve that does not come back, is no closed chain.
    if closed && n < 2 {
        let looped = segs.first().is_some_and(|s| {
            let near = 1e-6 * (1.0 + s.c.p0.x.abs().max(s.c.p0.y.abs()));
            !s.line && s.c.p3.distance(s.c.p0) <= near
        });
        if !looped {
            return None;
        }
    }
    let mut out = BezPath::new();
    out.move_to(segs.first()?.c.p0);
    for (i, s) in segs.iter().enumerate() {
        let end = match segs.get(i + 1) {
            Some(next) => next.c.p0,
            None if closed => segs.first()?.c.p0,
            None => s.c.p3,
        };
        if s.line || (s.c.p1.distance(s.c.p0) <= HANDLE_EPS && s.c.p2.distance(end) <= HANDLE_EPS) {
            out.line_to(end);
        } else {
            out.curve_to(s.c.p1, s.c.p2, end);
        }
    }
    if closed {
        out.close_path();
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn svg(s: &str) -> BezPath {
        BezPath::from_svg(s).unwrap()
    }

    #[test]
    fn subpaths_are_split_at_moves_and_closes_and_lone_moves_are_dropped() {
        let subs = subs(&svg("M0,0 L10,0 L10,10 Z M5,5 M20,0 L30,0 L30,10"));
        assert_eq!(subs.len(), 2, "{subs:?}");
        assert!(subs[0].closed && !subs[1].closed);
        assert_eq!((subs[0].start, subs[1].start), (Point::new(0.0, 0.0), Point::new(20.0, 0.0)));
        // A move followed only by a close draws nothing either.
        assert!(super::subs(&svg("M0,0 Z M3,3 L4,4")).len() == 1);
        assert!(super::subs(&BezPath::new()).is_empty());
    }

    #[test]
    fn segments_after_a_close_start_from_the_subpaths_start_as_kurbo_reads_them() {
        let bp = BezPath::from_vec(vec![
            PathEl::MoveTo(Point::new(1.0, 1.0)),
            PathEl::LineTo(Point::new(5.0, 1.0)),
            PathEl::ClosePath,
            PathEl::LineTo(Point::new(1.0, 7.0)),
        ]);
        let subs = subs(&bp);
        assert_eq!(subs.len(), 2);
        assert_eq!(subs[1].start, Point::new(1.0, 1.0));
        assert!(!subs[1].closed);
    }

    #[test]
    #[cfg(panic = "unwind")]
    fn a_panic_in_the_sweep_is_caught_as_none() {
        assert_eq!(guarded(|| 7), Some(7));
        assert_eq!(guarded(|| -> i32 { panic!("simulated panic in the sweep") }), None);
    }

    #[test]
    fn quadratics_are_raised_to_cubics() {
        let q = svg("M0,0 Q10,10 20,0 Z");
        let subs = subs(&q);
        let expect = kurbo::QuadBez::new((0.0, 0.0), (10.0, 10.0), (20.0, 0.0)).raise();
        assert_eq!(subs[0].draws[0], PathEl::CurveTo(expect.p1, expect.p2, expect.p3));
        assert!(canonical(&q).elements().iter().all(|e| !matches!(e, PathEl::QuadTo(..))));
    }

    #[test]
    fn a_closed_subpath_ends_with_a_segment_to_its_start() {
        let implicit = svg("M0,0 L10,0 L10,10 Z");
        let mut out = BezPath::new();
        subs(&implicit)[0].push_closed(&mut out);
        assert_eq!(out, svg("M0,0 L10,0 L10,10 L0,0 Z"));
        // A last point a hair from the start is moved onto it, and no closing line is added.
        let near = svg("M0,0 L10,0 L10,10 L0,0.00000001 Z");
        let mut out = BezPath::new();
        subs(&near)[0].push_closed(&mut out);
        assert_eq!(out, svg("M0,0 L10,0 L10,10 L0,0 Z"));
        assert_eq!(subs(&near)[0].anchors(), 3);
        assert_eq!(subs(&implicit)[0].anchors(), 3);
        assert_eq!(subs(&svg("M0,0 L10,0 L10,10"))[0].anchors(), 3);
    }

    #[test]
    fn a_chain_closes_open_subpaths_it_is_asked_to_and_snaps_a_near_end() {
        let open = subs(&svg("M0,0 L10,0 L10,10"))[0].clone();
        assert_eq!(open.chain().len(), 2);
        let mut closed = open.clone();
        closed.closed = true;
        let chain = closed.chain();
        assert_eq!(chain.len(), 3);
        assert!(chain[2].line && chain[2].c.p3 == Point::new(0.0, 0.0));
        let mut ring = subs(&svg("M0,0 L10,0 L10,10 L0,0"))[0].clone();
        ring.closed = true;
        assert_eq!(ring.chain().len(), 3, "it ends on its start already: no zero-length closing segment");
    }

    #[test]
    fn a_filled_path_closes_every_subpath_and_drops_those_without_area() {
        let f = fill_bezpath(&svg("M0,0 L10,0 L10,10 M20,0 L30,0 L30,10 L20,0 Z M40,40 M50,50 L50,50 Z"));
        assert_eq!(f, svg("M0,0 L10,0 L10,10 L0,0 Z M20,0 L30,0 L30,10 L20,0 Z"));
        assert!(fill_bezpath(&BezPath::new()).elements().is_empty());
    }

    #[test]
    fn open_and_closed_subpaths_are_found_without_copying() {
        assert!(has_open_subpath(&svg("M0,0 L1,1")));
        assert!(has_open_subpath(&svg("M0,0 L1,1 Z M5,5 L6,6")));
        assert!(!has_open_subpath(&svg("M0,0 L1,1 Z M5,5")));
        assert!(!has_open_subpath(&BezPath::new()));
        assert!(has_closed_subpath(&svg("M0,0 L1,1 Z")));
        assert!(!has_closed_subpath(&svg("M0,0 Z M3,3 L4,4")));
        assert!(!has_closed_subpath(&svg("M0,0 L1,1")));
    }

    #[test]
    fn a_chain_is_written_joined_up_and_as_lines_where_it_has_no_handles() {
        let a = Seg::exact(PathSeg::Line(kurbo::Line::new((0.0, 0.0), (10.0, 0.0))));
        let b = Seg::exact(PathSeg::Cubic(CubicBez::new((10.0, 0.0), (12.0, 3.0), (14.0, 3.0), (16.0, 0.0))));
        let flat = Seg::exact(PathSeg::Cubic(CubicBez::new((16.0, 0.0), (16.0, 0.0), (0.0, 0.0), (0.0, 0.0))));
        assert!(a.line && !b.line && flat.line, "a cubic whose handles sit on its ends is a line");
        let open = segs_to_bezpath(&[a, b], false).unwrap();
        assert_eq!(open, svg("M0,0 L10,0 C12,3 14,3 16,0"));
        // A gap of a rounding error is closed by moving the end onto the next start.
        let mut c = b;
        c.c.p0 = Point::new(10.0 + 1e-12, 0.0);
        assert_eq!(segs_to_bezpath(&[a, c], false).unwrap().elements()[1], PathEl::LineTo(Point::new(10.0 + 1e-12, 0.0)));
        assert!(segs_to_bezpath(&[], false).is_none());
        assert!(segs_to_bezpath(&[a], true).is_none(), "a closed chain needs two segments");
    }

    #[test]
    fn flatten_draws_a_curve_with_far_out_points_as_a_line_and_bounds_the_work() {
        let mut lines = 0;
        let far = [PathEl::MoveTo(Point::ZERO), PathEl::CurveTo(Point::new(1e300, 0.0), Point::new(0.0, 1e300), Point::new(1.0, 1.0))];
        flatten(far, 0.01, |e| lines += usize::from(matches!(e, PathEl::LineTo(_))));
        assert_eq!(lines, 1);
        // A tolerance finer than the curve allows makes at most a few thousand lines.
        let mut lines = 0;
        let curve = [PathEl::MoveTo(Point::ZERO), PathEl::CurveTo(Point::new(100.0, 0.0), Point::new(0.0, 100.0), Point::new(100.0, 100.0))];
        flatten(curve, 1e-300, |e| lines += usize::from(matches!(e, PathEl::LineTo(_))));
        assert!(lines > 100 && lines <= 5000, "{lines}");
    }
}
