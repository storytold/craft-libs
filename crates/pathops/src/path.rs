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

use std::borrow::Cow;

use kurbo::{BezPath, CubicBez, PathEl, PathSeg, Point};

use crate::fit::is_straight;
use crate::work::{self, Work};

/// A handle closer than this to its anchor is no handle (the segment is a line).
pub(crate) const HANDLE_EPS: f64 = 1e-9;

/// The signed area of `path` as [`kurbo::Shape::area`] gives it, each subpath's taken about its own
/// first point. The shoelace multiplies coordinates, and far from the origin their products cancel to
/// nothing for a small shape (a square half a unit across at 1e8 read 0, and its contour was dropped
/// as a sliver). Relative to a point of its own subpath a coordinate is exact (the difference of two
/// numbers within a factor of two of each other), and the area a closed subpath encloses does not
/// depend on the point it is taken about.
pub(crate) fn area_about_starts(path: &BezPath) -> f64 {
    use kurbo::{Line, ParamCurveArea, QuadBez};
    let (mut total, mut origin, mut start, mut cur) = (0.0, Point::ZERO, Point::ZERO, Point::ZERO);
    let at = |p: Point, o: Point| Point::new(p.x - o.x, p.y - o.y);
    for el in path.elements() {
        match *el {
            PathEl::MoveTo(p) => (origin, start, cur) = (p, p, p),
            PathEl::LineTo(p) => {
                total += Line::new(at(cur, origin), at(p, origin)).signed_area();
                cur = p;
            }
            PathEl::QuadTo(a, p) => {
                total += QuadBez::new(at(cur, origin), at(a, origin), at(p, origin)).signed_area();
                cur = p;
            }
            PathEl::CurveTo(a, b, p) => {
                total += CubicBez::new(at(cur, origin), at(a, origin), at(b, origin), at(p, origin)).signed_area();
                cur = p;
            }
            PathEl::ClosePath => {
                if cur != start {
                    total += Line::new(at(cur, origin), at(start, origin)).signed_area();
                }
                cur = start;
            }
        }
    }
    total
}

/// The last point of a closed subpath this close to its start is the start (the subpath's closing
/// segment arrives there, and the duplicate point is merged into it).
pub(crate) const JOIN_TOLERANCE: f64 = 1e-7;

/// How many times its own rounding the cross product of the two handles of a loop of one cubic must
/// pass for the loop to have an inside ([`handles_loop`]): each handle (a point less the start), the
/// two products and their difference are each rounded once, a few units in the last place of the
/// two terms in all.
const LOOP_ROUNDING: f64 = 64.0;

/// Do the handles `a` and `b` of a cubic from `start` that comes back to it take it round an area?
/// Such a loop encloses 3/20 of the cross product of its handles, which is nothing when they lie
/// along a line.
///
/// Told by the cross product against its own rounding, not by the area against the size of the loop,
/// nor by the angle between the handles: a loop of 75 billion square units that is a trillion long and
/// half a unit wide has handles 5e-13 of a radian apart, and is as much a loop as a round one. The cross
/// product `x1 y2 - y1 x2` is rounded to a few units in the last place of its two terms, so that of
/// handles along one line (also turned, each of them rounded) is under [`LOOP_ROUNDING`] machine epsilons
/// of `|x1 y2| + |y1 x2|`: that much is nothing.
pub(crate) fn handles_loop(start: Point, a: Point, b: Point) -> bool {
    let (h1, h2) = (a - start, b - start);
    let terms = (h1.x * h2.y).abs() + (h1.y * h2.x).abs();
    h1.cross(h2).abs() > LOOP_ROUNDING * f64::EPSILON * terms
}

/// One subpath of a path: where it starts, the segments that follow, and whether it was closed.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Sub {
    pub(crate) start: Point,
    /// The `LineTo`, `QuadTo` and `CurveTo` elements, in order. Never empty.
    pub(crate) draws: Vec<PathEl>,
    pub(crate) closed: bool,
}

/// `path` less the `ClosePath` elements it begins with. A close with no subpath before it closes nothing (it is no subpath, as [`subs`]
/// reads it), and kurbo's `segments`, with its `bounding_box` and everything else that walks the segments, panics on a path that begins
/// with one. Borrowed when there is none.
pub(crate) fn without_leading_closes(path: &BezPath) -> Cow<'_, BezPath> {
    let closes = path.elements().iter().take_while(|el| matches!(el, PathEl::ClosePath)).count();
    if closes == 0 {
        return Cow::Borrowed(path);
    }
    work::add(Work::Visited, path.elements().len());
    // `extend` takes what `push` refuses in a build for tests: a path that does not begin with a move.
    let mut out = BezPath::new();
    out.extend(path.elements().iter().skip(closes).copied());
    Cow::Owned(out)
}

/// `outlines` with each one [`without_leading_closes`]: borrowed when none begins with one.
pub(crate) fn outlines_without_leading_closes(outlines: &[BezPath]) -> Cow<'_, [BezPath]> {
    if !outlines.iter().any(|o| matches!(o.elements().first(), Some(PathEl::ClosePath))) {
        return Cow::Borrowed(outlines);
    }
    Cow::Owned(outlines.iter().map(|o| without_leading_closes(o).into_owned()).collect())
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

    /// An open subpath of three or more anchors, or of one cubic that loops round an area
    /// ([`Sub::draws_something`]), whose last anchor lies within 1e-6 of its first (the test
    /// [`join`](crate::join) applies to one path) as a closed subpath: its last segment ends on the
    /// first anchor. Any other subpath is returned unchanged. (A drawing program writes an ellipse
    /// as a path that comes back to where it started without closing.)
    pub(crate) fn closed_if_ends_meet(mut self) -> Self {
        if !self.closed && self.end().distance(self.start) < 1e-6 && (self.anchors() > 2 || self.loops_round_area()) {
            self.close_on_start();
        }
        self
    }

    /// An open subpath of one cubic that loops round an area and whose end lies within 1e-6 of its
    /// start, as a closed subpath (as [`Sub::closed_if_ends_meet`] closes it); any other subpath
    /// unchanged. Such a cubic draws the outline of its loop whether it is closed or not, and one that
    /// is open is no cutting edge of anything but that loop.
    pub(crate) fn closed_if_one_cubic_loop(mut self) -> Self {
        if !self.closed && self.end().distance(self.start) < 1e-6 && self.loops_round_area() {
            self.close_on_start();
        }
        self
    }

    /// The subpath closed, its last segment moved to end on its start.
    fn close_on_start(&mut self) {
        let start = self.start;
        if let Some(last) = self.draws.last_mut() {
            *last = with_end(*last, start);
        }
        self.closed = true;
    }

    /// Does the subpath draw anything? A closed one that is a single segment from its start back to
    /// it (one anchor) draws a point or a line out and back, unless that segment is a cubic whose
    /// handles take it round an area (`M0,0 C100,0 0,100 0,0 Z` is a loop with an inside).
    pub(crate) fn draws_something(&self) -> bool {
        self.anchors() >= 2 || self.loops_round_area()
    }

    /// Is the subpath one cubic whose handles, taken from its start, are not along one line? Such a
    /// cubic that ends where it starts is a loop with an inside ([`handles_loop`]).
    fn loops_round_area(&self) -> bool {
        let [PathEl::CurveTo(a, b, _)] = self.draws[..] else { return false };
        handles_loop(self.start, a, b)
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

/// Whether some subpath fills: one with a segment that is closed, or an open one of one cubic that
/// comes back to its start round an area ([`Sub::closed_if_one_cubic_loop`]), which a planar map fills
/// as a closed one. A path with none is a line there, whose pieces are cutting edges.
pub(crate) fn has_filled_subpath(path: &BezPath) -> bool {
    has_closed_subpath(path) || subs(path).into_iter().any(|sp| !sp.closed && sp.closed_if_one_cubic_loop().closed)
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
pub(crate) fn flatten(path: impl IntoIterator<Item = PathEl>, tolerance: f64, callback: impl FnMut(PathEl)) {
    flatten_charged(path, tolerance, &|_| true, callback);
}

/// The most lines `kurbo::flatten` makes of a curve, as a multiple of Wang's count of them (measured on random cubics and quadratics, 4,000 of them
/// at tolerances from a thousandth to a million times what they bow: 2 at the most, in the curves that make a line or two).
const FLATTEN_OVER_WANG: f64 = 2.0;

/// An upper bound of the lines [`flatten`] makes of the curve `el` that starts at `from`, to within `tolerance` (the tolerance it is flattened to,
/// raised as `flatten` raises it): Wang's count (`sqrt(n (n - 1) / 8 * M / tol)` for a curve of degree `n` whose second differences are at most
/// `M`) times [`FLATTEN_OVER_WANG`] and two more, at most the most a curve makes. A line is one.
fn lines_bound(el: PathEl, from: Point, tolerance: f64) -> usize {
    let (wang_squared_per_tol, length) = match el {
        PathEl::QuadTo(a, b) => (0.25 * (from.to_vec2() - a.to_vec2() * 2.0 + b.to_vec2()).hypot(), from.distance(a) + a.distance(b)),
        PathEl::CurveTo(a, b, c) => {
            let (d0, d1) = ((from.to_vec2() - a.to_vec2() * 2.0 + b.to_vec2()).hypot(), (a.to_vec2() - b.to_vec2() * 2.0 + c.to_vec2()).hypot());
            (0.75 * d0.max(d1), from.distance(a) + a.distance(b) + b.distance(c))
        }
        _ => return 1,
    };
    let tol = tolerance.max(length / (MAX_FLATTEN_PER_CURVE * MAX_FLATTEN_PER_CURVE));
    let wang = (wang_squared_per_tol / tol).sqrt().ceil();
    // A bound that is not a number (a second difference past what a float holds) is the most.
    let lines = if wang.is_finite() { wang * FLATTEN_OVER_WANG + 2.0 } else { MAX_FLATTEN_PER_CURVE };
    lines.min(MAX_FLATTEN_PER_CURVE * FLATTEN_OVER_WANG) as usize
}

/// [`flatten`] that asks `charge` for the lines of each curve ([`lines_bound`]) before it flattens it, and stops where `charge` says no. False
/// then, with what was flattened before given to `callback` already; true when the whole path was.
pub(crate) fn flatten_charged(
    path: impl IntoIterator<Item = PathEl>,
    tolerance: f64,
    charge: &dyn Fn(usize) -> bool,
    mut callback: impl FnMut(PathEl),
) -> bool {
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
        if !charge(lines_bound(el, from, tolerance)) {
            return false;
        }
        let tol = tolerance.max(length / (MAX_FLATTEN_PER_CURVE * MAX_FLATTEN_PER_CURVE));
        kurbo::flatten([PathEl::MoveTo(from), el], tol, |e| {
            if !matches!(e, PathEl::MoveTo(_)) {
                callback(e);
            }
        });
    }
    true
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

/// A point as the bits of its coordinates, to hash or order it by: the same bits for the same point,
/// with `-0.0` taken for `0.0` (they compare equal, and their bits do not).
pub(crate) fn point_bits(p: Point) -> (u64, u64) {
    let bits = |v: f64| if v == 0.0 { 0 } else { v.to_bits() };
    (bits(p.x), bits(p.y))
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

    /// The closes a path begins with are taken off it, and nothing else: a path with none is borrowed (no copy), what follows is as it was
    /// (a later close stays), and a path of closes only is empty. The work is counted for a copy only.
    #[test]
    fn the_closes_a_path_begins_with_are_taken_off_it() {
        let mut odd = BezPath::new();
        odd.extend([
            PathEl::ClosePath,
            PathEl::ClosePath,
            PathEl::MoveTo(Point::new(1.0, 1.0)),
            PathEl::LineTo(Point::new(5.0, 1.0)),
            PathEl::ClosePath,
            PathEl::ClosePath,
        ]);
        let clean =
            BezPath::from_vec(vec![PathEl::MoveTo(Point::new(1.0, 1.0)), PathEl::LineTo(Point::new(5.0, 1.0)), PathEl::ClosePath, PathEl::ClosePath]);
        work::reset();
        assert_eq!(without_leading_closes(&odd).into_owned(), clean);
        assert_eq!(work::take(Work::Visited), odd.elements().len(), "a copy reads the path once");
        assert!(matches!(without_leading_closes(&clean), Cow::Borrowed(_)), "a path with none is borrowed");
        assert_eq!(work::take(Work::Visited), 0, "and costs nothing");
        let mut closes = BezPath::new();
        closes.extend([PathEl::ClosePath, PathEl::ClosePath]);
        assert!(without_leading_closes(&closes).elements().is_empty());
        assert!(without_leading_closes(&BezPath::new()).elements().is_empty());
        // Each outline of a list is taken off its own, and a list with none is borrowed.
        let list = [odd.clone(), clean.clone(), closes.clone()];
        let out = outlines_without_leading_closes(&list);
        assert!(matches!(out, Cow::Owned(_)));
        assert_eq!((out[0].clone(), out[1].clone(), out[2].elements().len()), (clean.clone(), clean.clone(), 0));
        assert!(matches!(outlines_without_leading_closes(&[clean.clone(), clean]), Cow::Borrowed(_)));
        assert!(matches!(outlines_without_leading_closes(&[]), Cow::Borrowed(_)));
    }

    /// The lines [`flatten`] makes of a curve are never more than the bound it is charged ([`lines_bound`]): random cubics and quadratics, of every
    /// bow and size, at tolerances from a thousandth to a thousand times what they bow. (A bound is for the worst curve of its control points: a
    /// cubic that overshoots its ends with handles along its chord is flattened to far fewer lines than it is charged; see the arcs below.)
    #[test]
    fn the_lines_a_curve_is_flattened_to_are_within_the_bound_it_is_charged() {
        let mut rng = crate::testutil::Rng(0x9e37_79b9_7f4a_7c15);
        let mut worst = 0.0f64;
        for case in 0..4000 {
            let size = 10f64.powf(rng.unit() * 6.0 - 1.0);
            let mut p = || Point::new((rng.unit() - 0.5) * size, (rng.unit() - 0.5) * size);
            let from = p();
            let el = if case % 2 == 0 { PathEl::CurveTo(p(), p(), p()) } else { PathEl::QuadTo(p(), p()) };
            let tol = size * 10f64.powf(-rng.unit() * 6.0);
            let mut lines = 0usize;
            flatten([PathEl::MoveTo(from), el], tol, |e| lines += usize::from(matches!(e, PathEl::LineTo(_))));
            let bound = lines_bound(el, from, tol);
            assert!(lines < bound, "case {case}: {lines} lines against a bound of {bound} ({el:?} from {from:?} at {tol})");
            worst = worst.max(lines as f64 / (bound as f64 - 2.0).max(1.0) * FLATTEN_OVER_WANG);
        }
        // The factor was chosen on this: the most lines as a multiple of Wang's count is the factor of it, and no more.
        assert!(worst <= FLATTEN_OVER_WANG, "{worst}");
    }

    /// A bound that is not wasteful where it matters: for the arcs of a circle (what the thin faces of the map are bounded by), at radii of a
    /// hundred to a million and tolerances of a ten-thousandth to a hundredth of the radius, it is at most 2.5 times the lines made, and at
    /// least those.
    #[test]
    fn the_bound_on_the_lines_of_an_arc_is_within_two_and_a_half_times_the_lines() {
        let mut worst = 0.0f64;
        for r in [1e2, 1e4, 1e6] {
            for tol_per_r in [1e-4, 1e-5, 1e-7, 1e-9] {
                let tol = r * tol_per_r;
                let arcs = crate::testutil::cubic_circle(4, r, 0.0, 0.0);
                let (mut lines, mut bound) = (0usize, 0usize);
                let mut from = Point::new(r, 0.0);
                for el in arcs.elements().iter().skip(1) {
                    if let PathEl::CurveTo(_, _, c) = *el {
                        bound += lines_bound(*el, from, tol);
                        flatten([PathEl::MoveTo(from), *el], tol, |e| lines += usize::from(matches!(e, PathEl::LineTo(_))));
                        from = c;
                    }
                }
                assert!(lines <= bound, "radius {r}, tolerance {tol}: {lines} lines against {bound}");
                worst = worst.max(bound as f64 / lines as f64);
            }
        }
        assert!(worst <= 2.5, "{worst}");
    }

    /// A path flattened with a charge is charged for each curve, before it is flattened (a move, a line and a close are not charged), and where the
    /// charge says no the flattening stops with what was made before: the answer is false, and the curves after it are not asked for.
    #[test]
    fn a_flattening_stops_where_it_is_not_charged() {
        let arc = |x: f64| [PathEl::CurveTo(Point::new(x + 30.0, 40.0), Point::new(x + 70.0, 40.0), Point::new(x + 100.0, 0.0))];
        let mut path = vec![PathEl::MoveTo(Point::ZERO)];
        path.extend(arc(0.0));
        path.push(PathEl::LineTo(Point::new(150.0, 10.0)));
        path.extend(arc(150.0));
        path.extend(arc(250.0));
        path.push(PathEl::ClosePath);
        let lines_of = |upto: usize| {
            let mut n = 0usize;
            flatten(path[..upto].iter().copied(), 0.01, |e| n += usize::from(matches!(e, PathEl::LineTo(_))));
            n
        };
        let asked = std::cell::RefCell::new(Vec::new());
        let mut made = 0usize;
        let charge = |n: usize| {
            asked.borrow_mut().push(n);
            true
        };
        assert!(flatten_charged(path.iter().copied(), 0.01, &charge, |e| made += usize::from(matches!(e, PathEl::LineTo(_)))));
        assert_eq!(made, lines_of(path.len()), "charged and not refused: what flatten makes");
        let asked = asked.into_inner();
        assert_eq!(asked.len(), 3, "the three curves: {asked:?}");
        assert!(asked.iter().all(|&n| n > 10), "each is charged its lines: {asked:?}");
        // Refused at the second curve (the third is never asked for): the first curve and the line are made, and not the rest.
        let count = std::cell::Cell::new(0usize);
        let mut made = 0usize;
        let refusing = |_: usize| {
            count.set(count.get() + 1);
            count.get() < 2
        };
        let done = flatten_charged(path.iter().copied(), 0.01, &refusing, |e| made += usize::from(matches!(e, PathEl::LineTo(_))));
        assert!(!done);
        assert_eq!(count.get(), 2, "nothing is asked for after a no");
        assert_eq!(made, lines_of(3), "the first curve and the line");
    }

    /// The bound is Wang's count times two and two more, from the larger second difference of the control points and the tolerance (raised, for
    /// a curve of a length over `4096^2` tolerances, to its length over that), checked to the line on curves whose numbers are exact: a quadratic
    /// with a second difference of 20 at a tolerance of 0.05 (wang 10: 22), a cubic with one of 20 and one of 10 at 0.15 (wang 10: 22), a
    /// quadratic whose tolerance is raised (3,448), a cubic that reaches the most any curve is charged (5,796), a line, and a curve with a
    /// coordinate that is not a number.
    #[test]
    fn the_bound_on_the_lines_of_a_curve_is_wangs_count_twice_and_two() {
        let p = |x: f64, y: f64| Point::new(x, y);
        assert_eq!(lines_bound(PathEl::QuadTo(p(5.0, 10.0), p(10.0, 0.0)), p(0.0, 0.0), 0.05), 22);
        assert_eq!(lines_bound(PathEl::CurveTo(p(0.0, 10.0), p(0.0, 0.0), p(0.0, 0.0)), p(0.0, 0.0), 0.15), 22);
        // The larger of the two second differences: here the first is 10 and the second 20 (and 22 is the count of 20).
        assert_eq!(lines_bound(PathEl::CurveTo(p(0.0, 0.0), p(0.0, 10.0), p(0.0, 0.0)), p(0.0, 0.0), 0.15), 22, "0.75 x 20 / 0.15 is 100");
        // 2,000,000 of second difference over a length of 2.83 million, at a tolerance of a billionth that is raised to length / 4096^2.
        let raised = lines_bound(PathEl::QuadTo(p(1e6, 1e6), p(2e6, 0.0)), p(0.0, 0.0), 1e-9);
        assert_eq!(raised, 3_448);
        assert_eq!(lines_bound(PathEl::CurveTo(p(0.0, 1000.0), p(0.0, 0.0), p(0.0, 1000.0)), p(0.0, 0.0), 1e-9), 5_796);
        assert_eq!(lines_bound(PathEl::LineTo(p(1.0, 1.0)), p(0.0, 0.0), 0.01), 1);
        assert_eq!(lines_bound(PathEl::MoveTo(p(1.0, 1.0)), p(0.0, 0.0), 0.01), 1);
        // A second difference past what a float holds is the most a curve is charged, twice what it can make.
        assert_eq!(lines_bound(PathEl::QuadTo(p(1e308, 1e308), p(-1e308, 0.0)), p(1e308, 0.0), 1e-3), 4096);
    }
}
