// Carved from VectorCraft's crates/pathops (src/planar/graph.rs) at storytold/vectorcraft ff9deee46d1c59dc9ecef0b668c25e57cd80b55e.
// Copyright (c) 2026 ArtCraft Team and the VectorCraft contributors. MIT OR Apache-2.0.

use super::index::{RectIndex, UNSHEAR, segment_footprint};
use super::*;
use crate::path::subs;

/// The inputs' segments (in the sweep's frame), to tell which inputs an edge lies on.
pub(super) struct Outlines {
    segs: RectIndex<(PathSeg, usize)>,
    tol: f64,
}

impl Outlines {
    pub(super) fn new(inputs: &[(BezPath, usize)], eps: f64) -> Self {
        // A zero-length segment (a point) is the source of no edge, even one running through it.
        let segs = inputs
            .iter()
            .flat_map(|(bp, i)| bp.segments().map(move |s| (s.bounding_box(), (s, *i))))
            .filter(|(r, ..)| r.width() > 0.0 || r.height() > 0.0)
            .collect();
        // The sweep moves points by a few `eps` at most.
        Self { segs: RectIndex::with_footprint(segs, |r, (s, _), cell| segment_footprint(r, s, cell)), tol: 16.0 * eps }
    }

    /// The inputs whose outlines pass through `p`, back to front (none when no outline does).
    pub(super) fn sources_at(&self, p: Point) -> Vec<usize> {
        use kurbo::ParamCurveNearest;
        let tol = self.tol;
        let mut out: Vec<usize> = self.segs.at(p, tol).filter(|(s, _)| s.nearest(p, 1e-9).distance_sq <= tol * tol).map(|(_, i)| *i).collect();
        out.sort_unstable();
        out.dedup();
        out
    }
}

/// The area an input's fill covers: its closed subpaths (open ones bound no face).
pub(super) struct Fill {
    path: BezPath,
    bounds: kurbo::Rect,
    rule: FillRule,
}

impl Fill {
    pub(super) fn new(s: &Shape) -> Self {
        let mut path = BezPath::new();
        for sp in subs(&s.path).iter().filter(|sp| sp.closed && sp.anchors() >= 2) {
            sp.push_closed(&mut path);
        }
        Self { bounds: path.bounding_box(), path, rule: s.rule }
    }

    pub(super) fn contains(&self, p: Point) -> bool {
        !self.path.elements().is_empty() && self.bounds.contains(p) && crate::boolean::inside(self.rule, self.path.winding(p))
    }
}

/// One output segment of the sweep: an edge of the planar graph between two vertices.
pub(super) struct Edge {
    pub(super) segs: Vec<Seg>,
    pub(super) ends: [usize; 2],
    /// The input it came from (the front-most, where inputs share it).
    pub(super) source: usize,
    /// Every input it lies on, back to front (`source` last).
    pub(super) sources: Vec<usize>,
}

/// `segs` (a chain) moved to start at `a` and end at `b`, the handles next to each moving with it.
pub(crate) fn pin_ends(segs: &mut [Seg], a: Point, b: Point) {
    if let Some(s) = segs.first_mut() {
        let d = a - s.c.p0;
        s.c.p0 = a;
        s.c.p1 += d;
    }
    if let Some(s) = segs.last_mut() {
        let d = b - s.c.p3;
        s.c.p3 = b;
        s.c.p2 += d;
    }
}

/// Half-edge `2e` runs along edge `e`, `2e + 1` back along it.
pub(super) fn twin(h: usize) -> usize {
    h ^ 1
}

/// The segments of half-edge `h`, in its direction.
pub(super) fn half_segs(edges: &[Edge], h: usize) -> Vec<Seg> {
    half_seg_iter(edges, h).collect()
}

/// The segments of half-edge `h` in its direction, without copying the edge first (an edge can
/// hold thousands of segments, and the angle it leaves a vertex at needs the first few).
fn half_seg_iter(edges: &[Edge], h: usize) -> impl Iterator<Item = Seg> + '_ {
    let segs = edges.get(h / 2).map(|e| e.segs.as_slice()).unwrap_or_default();
    let forward = h.is_multiple_of(2);
    (0..segs.len()).filter_map(move |k| {
        let s = segs.get(if forward { k } else { segs.len() - 1 - k })?;
        Some(if forward { *s } else { Seg { c: CubicBez::new(s.c.p3, s.c.p2, s.c.p1, s.c.p0), line: s.line } })
    })
}

/// Where half-edge `h` ends (the vertex at the other end of its edge).
pub(super) fn far_end(edges: &[Edge], h: usize) -> Option<Point> {
    let e = edges.get(h / 2)?;
    if h.is_multiple_of(2) { e.segs.last().map(|s| s.c.p3) } else { e.segs.first().map(|s| s.c.p0) }
}

/// The vertex half-edge `h` leaves from.
pub(super) fn origin(edges: &[Edge], h: usize) -> Option<usize> {
    edges.get(h / 2).map(|e| e.ends[h % 2])
}

/// The start of `c`, as a cubic of its own and the parameter of `c` it ends at, that is enough to
/// find where `c` first gets `r` from `v`.
///
/// `c` itself when its control polygon stays within 4 `r` of `v`, or when flattening all of it is
/// cheap: the same lines as ever, so that edges leaving a vertex along one tangent, whose order
/// the flattening decides, are put in the order they always were. The sweep's stubs make `r` a
/// millionth of the curve, and a million lines is what that takes. Else (the bound on the lines,
/// `sqrt(0.75 m / tol)` for second differences of at most `m`, is over [`WHOLE_LINES`], or over
/// what is left of `whole` for the vertices of one map) the part of `c` up to the parameter, a power
/// of two, where the control polygon of the part first lies within 4 `r` of `v`, and one doubling
/// more, so that the part still reaches past `r`. A curve lies inside its control polygon, so
/// what is cut is flattened in a few hundred lines at most, however long `c` is.
pub(super) fn start_within_reach(c: &CubicBez, v: Point, r: f64, whole: &mut f64) -> (CubicBez, f64) {
    use kurbo::ParamCurve;
    let within = |c: &CubicBez| [c.p0, c.p1, c.p2, c.p3].iter().all(|p| p.distance(v) <= 4.0 * r);
    if within(c) {
        return (*c, 1.0);
    }
    let second = |a: Point, b: Point, d: Point| (a.to_vec2() - 2.0 * b.to_vec2() + d.to_vec2()).hypot();
    let m = second(c.p0, c.p1, c.p2).max(second(c.p1, c.p2, c.p3));
    let lines = (0.75 * m / (r * 1e-4).max(1e-12)).sqrt();
    if lines <= WHOLE_LINES && lines <= *whole {
        *whole -= lines;
        return (*c, 1.0);
    }
    let mut t = 1.0;
    // Sixty halvings take a segment of 1e18 r down to r.
    for _ in 0..60 {
        t *= 0.5;
        if within(&c.subsegment(0.0..t)) {
            break;
        }
    }
    let end = (2.0 * t).min(1.0);
    (c.subsegment(0.0..end), end)
}

/// Most lines (by the bound on them) one segment is flattened into whole to find where it leaves a
/// vertex: a million, some tens of milliseconds.
pub(super) const WHOLE_LINES: f64 = 1_048_576.0;

/// Most lines (by the bound) the segments of one planar map are flattened into whole: past it each
/// is cut to its start, so a map of many vertices where a tiny edge meets huge ones costs a few
/// hundred lines a vertex and not a million.
pub(super) const WHOLE_LINES_IN_ALL: f64 = 67_108_864.0;

/// Most parts of one segment read for the way it leaves a vertex (the first nearly always has it).
const MAX_PARTS: usize = 64;

/// Where half-edge `h` (leaving `v`) first gets `r` away from it, as an angle around `v`.
///
/// Edges that only meet at `v` cross a circle around it in the order they leave `v` in, however
/// they start out (tangent curves, or the sweep's short straight stubs at a vertex). `whole` is
/// what is left of [`WHOLE_LINES_IN_ALL`].
pub(super) fn leaving_angle(edges: &[Edge], h: usize, v: Point, r: f64, whole: &mut f64) -> f64 {
    let first = half_seg_iter(edges, h).next();
    let mut prev = first.map_or(v, |s| s.c.p0);
    let mut hit: Option<Point> = None;
    // One segment at a time, stopping at the first that gets r away: an edge can run on for
    // thousands of segments (a traced outline between two crossings), all of which flattening
    // the whole edge to this tolerance would visit, at every vertex. And one part of a segment at
    // a time where it is long beside `r`: flattening a segment that is ten billion times `r` to a
    // tolerance of `r` over ten thousand makes up to a billion lines.
    'edge: for s in half_seg_iter(edges, h) {
        let mut rest = s.c;
        for _ in 0..MAX_PARTS {
            let (c, end) = start_within_reach(&rest, v, r, whole);
            let path = [kurbo::PathEl::MoveTo(c.p0), kurbo::PathEl::CurveTo(c.p1, c.p2, c.p3)];
            kurbo::flatten(path, (r * 1e-4).max(1e-12), |el| {
                if matches!(el, kurbo::PathEl::LineTo(_)) {
                    crate::work::add(crate::work::Work::FlattenLines, 1);
                }
                if let (None, kurbo::PathEl::LineTo(p)) = (hit, el) {
                    if p.distance(v) >= r {
                        // The point on prev → p at distance r (prev is closer).
                        let (d, f) = (p - prev, prev - v);
                        let (a, b, c) = (d.hypot2(), 2.0 * f.dot(d), f.hypot2() - r * r);
                        let t = if a > 0.0 { ((-b + (b * b - 4.0 * a * c).max(0.0).sqrt()) / (2.0 * a)).clamp(0.0, 1.0) } else { 1.0 };
                        hit = Some(prev + d * t);
                    }
                    prev = p;
                }
            });
            if hit.is_some() {
                break 'edge;
            }
            if end >= 1.0 {
                break;
            }
            // The part did not get r away (the curve turned back inside it): on to the rest.
            rest = kurbo::ParamCurve::subsegment(&rest, end..1.0);
        }
    }
    // An edge too short to reach r (it can't, r is at most half of every chord) leaves along its
    // tangent.
    let far = hit.unwrap_or_else(|| first.map_or(v, |s| [s.c.p1, s.c.p2, s.c.p3].into_iter().find(|p| p.distance(s.c.p0) > 0.0).unwrap_or(s.c.p3)));
    (far - v).atan2()
}

/// Path of a closed run of half-edges, tidied (split pieces of one input curve joined again).
pub(super) fn cycle_bezpath(edges: &[Edge], cycle: &[usize]) -> BezPath {
    let mut bp = BezPath::new();
    for (k, s) in cycle.iter().flat_map(|&h| half_segs(edges, h)).enumerate() {
        if k == 0 {
            bp.move_to(s.c.p0);
        }
        if s.line {
            bp.line_to(s.c.p3);
        } else {
            bp.curve_to(s.c.p1, s.c.p2, s.c.p3);
        }
    }
    bp.close_path();
    bp
}

/// `cycle` without the spurs it runs out and back along (edges with the same face on both sides,
/// such as a line's loose end).
pub(super) fn without_spurs(cycle: &[usize]) -> Vec<usize> {
    let mut out: Vec<usize> = Vec::with_capacity(cycle.len());
    for &h in cycle {
        if out.last() == Some(&twin(h)) {
            out.pop();
        } else {
            out.push(h);
        }
    }
    // A spur where the walk started wraps around the ends.
    while out.len() >= 2 && out.first().map(|&h| twin(h)) == out.last().copied() {
        out.pop();
        out.remove(0);
    }
    out
}

/// Append `open` traced there and back (a closed, zero-area path along it) to `out`.
pub(super) fn there_and_back(open: &BezPath, out: &mut BezPath) {
    let segs: Vec<PathSeg> = open.segments().collect();
    let Some(first) = segs.first() else { return };
    out.move_to(first.start());
    for s in &segs {
        out.push(s.as_path_el());
    }
    for s in segs.iter().rev() {
        out.push(s.reverse().as_path_el());
    }
    out.close_path();
}

/// `path` back from the sweep's frame, its on-curve points within `tol` of an input anchor (`sorted`
/// by x) put exactly on it, the control points next to it moving along.
pub(super) fn unshear(path: BezPath, sorted: &[Point], tol: f64) -> BezPath {
    let snap = |p: Point| -> Point {
        let lo = sorted.partition_point(|q| q.x < p.x - tol);
        sorted.get(lo..).unwrap_or_default().iter().take_while(|q| q.x <= p.x + tol).find(|q| (q.y - p.y).abs() <= tol).copied().unwrap_or(p)
    };
    let mut out = BezPath::new();
    // How far the point the next segment leaves from moved, and how far the subpath's start did.
    let (mut shift, mut start_shift) = (kurbo::Vec2::ZERO, kurbo::Vec2::ZERO);
    for el in path.elements() {
        match *el {
            kurbo::PathEl::MoveTo(p) => {
                let p = UNSHEAR * p;
                let q = snap(p);
                (shift, start_shift) = (q - p, q - p);
                out.move_to(q);
            }
            kurbo::PathEl::LineTo(p) => {
                let p = UNSHEAR * p;
                let q = snap(p);
                shift = q - p;
                out.line_to(q);
            }
            kurbo::PathEl::QuadTo(c, p) => {
                let (c, p) = (UNSHEAR * c, UNSHEAR * p);
                let q = snap(p);
                out.quad_to(c + (shift + (q - p)) / 2.0, q);
                shift = q - p;
            }
            kurbo::PathEl::CurveTo(c1, c2, p) => {
                let (c1, c2, p) = (UNSHEAR * c1, UNSHEAR * c2, UNSHEAR * p);
                let q = snap(p);
                out.curve_to(c1 + shift, c2 + (q - p), q);
                shift = q - p;
            }
            kurbo::PathEl::ClosePath => {
                out.close_path();
                shift = start_shift;
            }
        }
    }
    out
}

pub(super) fn find(parent: &mut [usize], mut v: usize) -> usize {
    while let Some(&p) = parent.get(v) {
        if p == v {
            break;
        }
        let gp = parent.get(p).copied().unwrap_or(p);
        if let Some(slot) = parent.get_mut(v) {
            *slot = gp;
        }
        v = p;
    }
    v
}
