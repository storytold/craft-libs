//! Tangent Circle: the circles that touch two shapes and have a given radius, or that touch three
//! shapes. A circle touches a shape when it meets the shape's outline without crossing it: the
//! whole outline lies outside the circle (its nearest point is on the circle) or inside it (its
//! farthest point is on the circle). At a smooth point of the outline the circle is tangent there;
//! at a corner or an open end it passes through that point.
//!
//! When every shape is a four-segment Bézier circle the answer is closed-form (the problem of
//! Apollonius for three), then moved onto the drawn outlines.
//! Otherwise the centers are isolated on a grid that is split where a root can be: the distance
//! from a point to an outline changes by at most the distance the point moves, so a cell whose
//! center is too far from every condition holds no center and is dropped. The cells left are
//! refined with Newton's method on the exact curves.

use craft_geom::PathData;
use kurbo::{CubicBez, ParamCurve, Point, Vec2};

#[cfg(test)]
mod tests;

/// Most segments the shapes may hold together, counted by [`tangent_circle_segments`].
pub const MAX_TANGENT_CIRCLE_SEGMENTS: usize = 2_000;

/// The segments of `p`'s subpaths, a lone anchor counting as one.
pub fn tangent_circle_segments(p: &PathData) -> usize {
    p.subpaths.iter().map(|sp| sp.segment_count().max(usize::from(!sp.anchors.is_empty()))).sum()
}
/// Most circles one call returns.
pub const MAX_TANGENT_CIRCLES: usize = 64;
/// With three shapes, the largest radius looked for, as a multiple of the diagonal of the box
/// around the shapes' anchors and handles.
const MAX_RADIUS_FACTOR: f64 = 64.0;

/// A circle by its center and radius.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Circle {
    pub center: Point,
    pub radius: f64,
}

/// Why [`tangent_circles`] gives no answer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TangentCircleError {
    /// Not two shapes with a radius above 0, nor three shapes without one; or a shape with no
    /// outline, or values that are not finite.
    BadInput,
    /// More than [`MAX_TANGENT_CIRCLE_SEGMENTS`] segments, or a search that would take too long.
    TooComplex,
    /// More than [`MAX_TANGENT_CIRCLES`] circles: the shapes may be touched along a whole curve
    /// (two concentric circles, for example).
    TooMany,
}

/// The circles that touch every one of `shapes` (document coordinates): two shapes and `radius`,
/// or three shapes and no radius. They are listed by radius, then top to bottom, then left to
/// right; an empty list when no circle touches them all.
pub fn tangent_circles(shapes: &[&PathData], radius: Option<f64>) -> Result<Vec<Circle>, TangentCircleError> {
    use TangentCircleError::*;
    match (shapes.len(), radius) {
        (2, Some(r)) if r.is_finite() && r > 0.0 => {}
        (3, None) => {}
        _ => return Err(BadInput),
    }
    if shapes.iter().map(|p| tangent_circle_segments(p)).sum::<usize>() > MAX_TANGENT_CIRCLE_SEGMENTS {
        return Err(TooComplex);
    }
    let mut bounds = None::<kurbo::Rect>;
    // The smallest shape's diagonal.
    let mut smallest = f64::INFINITY;
    for p in shapes {
        if p.subpaths.iter().all(|sp| sp.anchors.is_empty()) {
            return Err(BadInput);
        }
        let b = p.control_bounds().ok_or(BadInput)?;
        smallest = smallest.min(b.width().hypot(b.height()));
        bounds = Some(bounds.map_or(b, |a| a.union(b)));
    }
    let bounds = bounds.ok_or(BadInput)?;
    let finite = [bounds.x0, bounds.y0, bounds.x1, bounds.y1].iter().all(|v| v.is_finite());
    // The work is done about the bounds' center and in units of their diagonal.
    let origin = bounds.center();
    let scale = bounds.width().hypot(bounds.height());
    if !finite || !scale.is_finite() || scale <= 0.0 {
        return Err(BadInput);
    }
    let local = |p: Point| Point::new((p.x - origin.x) / scale, (p.y - origin.y) / scale);
    let r = radius.map(|r| r / scale);
    if r.is_some_and(|r| !r.is_finite()) {
        // Shapes so small that the radius is past any number in their units.
        return Err(BadInput);
    }
    let circles: Option<Vec<Circle>> =
        shapes.iter().map(|p| as_circle(p).map(|c| Circle { center: local(c.center), radius: c.radius / scale })).collect();
    // Two of three shapes the same circle up to rounding, or two copies of the same path: every
    // circle touching it and the third touches all three.
    let same = |a: &Circle, b: &Circle| (a.center - b.center).hypot() <= COPY && (a.radius - b.radius).abs() <= COPY;
    if shapes.len() == 3
        && (0..3).any(|i| match &circles {
            Some(c) => same(&c[i], &c[(i + 1) % 3]),
            None => shapes[i] == shapes[(i + 1) % 3],
        })
    {
        return Err(TooMany);
    }
    let outlines: Vec<Outline> = shapes.iter().map(|p| Outline::new(p, &local)).collect();
    let found = match circles {
        Some(c) => {
            let exact = match r {
                Some(r) => two_circles(c[0], c[1], r)?,
                None => apollonius(c[0], c[1], c[2]).into_iter().filter(|c| c.radius <= MAX_RADIUS_FACTOR).collect(),
            };
            // The paths are Bézier circles: each answer is polished on their outlines.
            exact.into_iter().map(|found| on_outlines(&outlines, &c, r, found)).collect()
        }
        None => numeric(&outlines, r)?,
    };
    // Answers about a shape far smaller than the gaps between the shapes can be closer together than
    // `SAME`: the tolerance shrinks with that shape.
    let tolerance = SAME.min(smallest / scale * 1e-3).max(COPY);
    let mut out = dedup(found, tolerance);
    if out.len() > MAX_TANGENT_CIRCLES {
        return Err(TooMany);
    }
    sort(&mut out, tolerance);
    Ok(out
        .into_iter()
        .map(|c| Circle { center: Point::new(origin.x + c.center.x * scale, origin.y + c.center.y * scale), radius: c.radius * scale })
        .filter(|c| c.center.x.is_finite() && c.center.y.is_finite() && c.radius.is_finite())
        .collect())
}

/// `found`, an answer for the ideal circles `ideal`, moved by Newton's method to touch the
/// `outlines` of the paths themselves in the same way: each circle's outline lies outside the answer
/// (nearest point), or inside it when the answer holds that circle (farthest point). `found` as it
/// is when the method does not settle nearby: where the answer is within the Bézier circles' own
/// error (0.0273% of the radius) of touching them in two places or none, it follows the true
/// circles.
fn on_outlines(outlines: &[Outline], ideal: &[Circle], r: Option<f64>, found: Circle) -> Circle {
    let mut combo = 0;
    for (i, c) in ideal.iter().enumerate() {
        let d = (found.center - c.center).hypot();
        let holds = (d + c.radius - found.radius).abs();
        if holds < (d - c.radius - found.radius).abs() && holds < (c.radius - d - found.radius).abs() {
            combo |= 1 << i;
        }
    }
    newton(outlines, r, combo, found.center, 1e-4, &mut 0.0)
        .filter(|c| (c.center - found.center).hypot() <= 1e-3 && (c.radius - found.radius).abs() <= 1e-3)
        .unwrap_or(found)
}

/// `path` as a circle when it is a four-segment Bézier circle: one closed subpath of four curved
/// segments with a square bounding box, lying within 0.03% of the radius of a circle (its largest
/// error is 0.0273%).
fn as_circle(path: &PathData) -> Option<Circle> {
    let [sp] = path.subpaths.as_slice() else { return None };
    if !sp.closed || sp.segment_count() != 4 || (0..4).any(|i| sp.segment_is_line(i)) {
        return None;
    }
    let b = path.bounds()?;
    let radius = (b.width() + b.height()) / 4.0;
    if !radius.is_finite() || radius <= 0.0 || (b.width() - b.height()).abs() > 1e-6 * radius {
        return None;
    }
    let center = b.center();
    let near = |p: Point| ((p - center).hypot() - radius).abs() <= 3e-4 * radius;
    let round = (0..sp.segment_count()).all(|i| {
        let seg = sp.segment(i);
        (0..=16).all(|k| near(seg.eval(k as f64 / 16.0)))
    });
    round.then_some(Circle { center, radius })
}

/// Below this (in units of the bounds' diagonal) two values are the same.
const EPS: f64 = 1e-9;

/// The circles of radius `r` touching circles `a` and `b`: their centers are `a.radius + r` from
/// `a`'s center (outside it) or `|a.radius - r|` (inside it, or holding it), and the same for `b`.
/// A distance of 0 (`r` equal to that circle's radius) puts the center on that circle's center.
/// Concentric circles at a distance both allow touch along a whole circle: [`TangentCircleError::TooMany`].
fn two_circles(a: Circle, b: Circle, r: f64) -> Result<Vec<Circle>, TangentCircleError> {
    let mut out = vec![];
    let apart = (a.center - b.center).hypot();
    let only = |ok: bool, p: Point| if ok { vec![p] } else { vec![] };
    for da in [a.radius + r, (a.radius - r).abs()] {
        for db in [b.radius + r, (b.radius - r).abs()] {
            let centers = match (da <= EPS, db <= EPS) {
                (true, true) => only(apart <= EPS, a.center),
                (true, false) => only((apart - db).abs() <= EPS, a.center),
                (false, true) => only((apart - da).abs() <= EPS, b.center),
                (false, false) if apart <= EPS && (da - db).abs() <= EPS => return Err(TangentCircleError::TooMany),
                (false, false) => circle_crossings(a.center, da, b.center, db),
            };
            out.extend(centers.into_iter().map(|center| Circle { center, radius: r }));
        }
    }
    Ok(out)
}

/// Where the circle about `c1` of radius `r1` meets the one about `c2` of radius `r2`: none, one
/// where they touch, or two.
fn circle_crossings(c1: Point, r1: f64, c2: Point, r2: f64) -> Vec<Point> {
    let v = c2 - c1;
    let d = v.hypot();
    if d <= EPS || d > r1 + r2 + EPS || d < (r1 - r2).abs() - EPS {
        return vec![];
    }
    let along = (d * d + r1 * r1 - r2 * r2) / (2.0 * d);
    let h2 = r1 * r1 - along * along;
    let u = v / d;
    let foot = c1 + u * along;
    // Only rounding takes a touching pair below 0; two crossings closer than `SAME` become one later.
    if h2 <= 0.0 {
        return vec![foot];
    }
    let n = Vec2::new(-u.y, u.x) * h2.sqrt();
    vec![foot + n, foot - n]
}

/// The circles touching circles `c` (the problem of Apollonius): for each choice of outside (+)
/// or inside (−) per circle, `|center − cᵢ|² = (r ± rᵢ)²`. Subtracting the first equation from the
/// others leaves two linear equations in x, y and r; two of the three unknowns follow from the
/// third, and the first equation gives a quadratic in it.
fn apollonius(a: Circle, b: Circle, c: Circle) -> Vec<Circle> {
    let cs = [a, b, c];
    let mut out = vec![];
    for signs in 0..8u8 {
        let s = [0, 1, 2].map(|i| if signs & (1 << i) == 0 { 1.0 } else { -1.0 });
        let (x1, y1, r1) = (cs[0].center.x, cs[0].center.y, cs[0].radius * s[0]);
        let row = |i: usize| {
            let (x, y, ri) = (cs[i].center.x, cs[i].center.y, cs[i].radius * s[i]);
            ([2.0 * (x - x1), 2.0 * (y - y1), 2.0 * (ri - r1)], (x * x + y * y - ri * ri) - (x1 * x1 + y1 * y1 - r1 * r1))
        };
        let (m1, b1) = row(1);
        let (m2, b2) = row(2);
        // Keep the third unknown free whose 2×2 minor is largest.
        let minor = |i: usize, j: usize| m1[i] * m2[j] - m1[j] * m2[i];
        let Some((free, (i, j))) =
            [(2, (0, 1)), (1, (0, 2)), (0, (1, 2))].into_iter().max_by(|p, q| minor(p.1.0, p.1.1).abs().total_cmp(&minor(q.1.0, q.1.1).abs()))
        else {
            continue;
        };
        let det = minor(i, j);
        if det.abs() <= EPS {
            continue;
        }
        // vᵢ = αᵢ + βᵢ t, vⱼ = αⱼ + βⱼ t, with t the free unknown.
        let alpha_i = (b1 * m2[j] - b2 * m1[j]) / det;
        let beta_i = (-m1[free] * m2[j] + m2[free] * m1[j]) / det;
        let alpha_j = (m1[i] * b2 - m2[i] * b1) / det;
        let beta_j = (-m1[i] * m2[free] + m2[i] * m1[free]) / det;
        let unknowns = |t: f64| {
            let mut v = [0.0; 3];
            v[free] = t;
            v[i] = alpha_i + beta_i * t;
            v[j] = alpha_j + beta_j * t;
            v
        };
        // (x − x1)² + (y − y1)² − (r + r1)² as a quadratic in t.
        let [v0, v1] = [unknowns(0.0), unknowns(1.0)];
        let lin = [v0[0] - x1, v0[1] - y1, v0[2] + r1];
        let dir = [v1[0] - v0[0], v1[1] - v0[1], v1[2] - v0[2]];
        let qa = dir[0] * dir[0] + dir[1] * dir[1] - dir[2] * dir[2];
        let qb = 2.0 * (lin[0] * dir[0] + lin[1] * dir[1] - lin[2] * dir[2]);
        let qc = lin[0] * lin[0] + lin[1] * lin[1] - lin[2] * lin[2];
        for t in quadratic_roots(qa, qb, qc) {
            let [x, y, r] = unknowns(t);
            if r > EPS && [x, y, r].iter().all(|v| v.is_finite()) {
                out.push(Circle { center: Point::new(x, y), radius: r });
            }
        }
    }
    out
}

/// The real roots of `a t² + b t + c`, a double root once; none when every coefficient is 0.
fn quadratic_roots(a: f64, b: f64, c: f64) -> Vec<f64> {
    let size = a.abs().max(b.abs()).max(c.abs());
    if size == 0.0 {
        return vec![];
    }
    let (a, b, c) = (a / size, b / size, c / size);
    if a.abs() <= EPS {
        return if b.abs() <= EPS { vec![] } else { vec![-c / b] };
    }
    let disc = b * b - 4.0 * a * c;
    if disc < -1e-12 {
        return vec![];
    }
    if disc <= 1e-12 {
        return vec![-b / (2.0 * a)];
    }
    // The root that adds magnitudes first; with b = 0 either sign does.
    let q = if b < 0.0 { -0.5 * (b - disc.sqrt()) } else { -0.5 * (b + disc.sqrt()) };
    vec![q / a, c / q]
}

/// Circles closer than this (in units of the bounds' diagonal) are one.
const SAME: f64 = 1e-6;
/// Given circles closer than this (in units of the bounds' diagonal) are copies of one circle.
const COPY: f64 = 1e-12;

/// Sorts `circles` (local units) by radius, then top to bottom, then left to right. Values within
/// `tolerance` of each other (the radii of mirror images, for example) sort as equal.
fn sort(circles: &mut [Circle], tolerance: f64) {
    let key = |v: f64| (v / tolerance).round();
    circles.sort_by(|a, b| {
        key(a.radius).total_cmp(&key(b.radius)).then(key(a.center.y).total_cmp(&key(b.center.y))).then(key(a.center.x).total_cmp(&key(b.center.x)))
    });
}

/// `found` with each circle within `tolerance` of an earlier one left out.
fn dedup(found: Vec<Circle>, tolerance: f64) -> Vec<Circle> {
    let mut out: Vec<Circle> = vec![];
    for c in found {
        if !out.iter().any(|o| (o.center - c.center).hypot() <= tolerance && (o.radius - c.radius).abs() <= tolerance) {
            out.push(c);
        }
    }
    out
}

/// A shape's outline, in local units: its segments, and chords that lie within `slack` of them.
struct Outline {
    segments: Vec<CubicBez>,
    /// Each segment's neighbors along its subpath (before, after), where it has them.
    beside: Vec<(Option<usize>, Option<usize>)>,
    /// (start, end, segment, t at start, t at end).
    chords: Vec<(Point, Point, usize, f64, f64)>,
    /// The farthest the chords are from the segments.
    slack: f64,
}

/// How far chords may stray from their segment, at most (local units); each segment is cut into
/// at most [`MAX_CHORDS`] chords, so a tightly bent one may stray more.
const CHORD_SLACK: f64 = 1e-3;
const MAX_CHORDS: usize = 64;

impl Outline {
    fn new(path: &PathData, local: &dyn Fn(Point) -> Point) -> Self {
        let mut o = Outline { segments: vec![], beside: vec![], chords: vec![], slack: 0.0 };
        for sp in &path.subpaths {
            let n = sp.segment_count();
            if n == 0 {
                if let Some(a) = sp.anchors.first() {
                    // A lone anchor: a segment that stays on its point.
                    let p = local(a.p);
                    o.segments.push(CubicBez::new(p, p, p, p));
                    o.beside.push((None, None));
                    o.chords.push((p, p, o.segments.len() - 1, 0.0, 1.0));
                }
                continue;
            }
            let first = o.segments.len();
            for i in 0..n {
                let s = sp.segment(i);
                let seg = CubicBez::new(local(s.p0), local(s.p1), local(s.p2), local(s.p3));
                let k = o.segments.len();
                let before = if i > 0 { Some(k - 1) } else { sp.closed.then_some(first + n - 1) };
                let after = if i + 1 < n { Some(k + 1) } else { sp.closed.then_some(first) };
                // |B''| is at most 6 × the larger second difference; a chord over a parameter step
                // h strays at most |B''| h² / 8.
                let bend = 6.0
                    * (seg.p0.to_vec2() - 2.0 * seg.p1.to_vec2() + seg.p2.to_vec2())
                        .hypot()
                        .max((seg.p1.to_vec2() - 2.0 * seg.p2.to_vec2() + seg.p3.to_vec2()).hypot());
                let pieces = ((bend / (8.0 * CHORD_SLACK)).sqrt().ceil() as usize).clamp(1, MAX_CHORDS);
                o.slack = o.slack.max(bend / (8.0 * (pieces * pieces) as f64));
                let mut prev = seg.p0;
                for j in 1..=pieces {
                    let t = j as f64 / pieces as f64;
                    let p = if j == pieces { seg.p3 } else { seg.eval(t) };
                    o.chords.push((prev, p, k, (j - 1) as f64 / pieces as f64, t));
                    prev = p;
                }
                o.segments.push(seg);
                o.beside.push((before, after));
            }
        }
        o
    }

    /// The chords' nearest and farthest distance from `q`.
    fn reach(&self, q: Point) -> Reach {
        let mut r = Reach { near: f64::INFINITY, far: -1.0 };
        for &(a, b, ..) in &self.chords {
            r.near = r.near.min(chord_near(q, a, b).0);
            r.far = r.far.max((q - a).hypot()).max((q - b).hypot());
        }
        r
    }

    /// The point of the curves nearest `q` (or farthest, `far`) and its distance. Every segment
    /// whose chords come within twice the slack of the chords' best answer may hold the true one,
    /// so each is polished from its own best chord point, and the best result is kept.
    fn extreme(&self, q: Point, far: bool) -> (Point, f64) {
        let better = |a: f64, b: f64| if far { a > b } else { a < b };
        let worst = if far { -1.0 } else { f64::INFINITY };
        // Each segment's best chord distance and the parameter where it is reached.
        let mut per = vec![(worst, 0.0); self.segments.len()];
        for &(a, b, seg, t0, t1) in &self.chords {
            let (d, u) = if far {
                let (da, db) = ((q - a).hypot(), (q - b).hypot());
                if db > da { (db, 1.0) } else { (da, 0.0) }
            } else {
                chord_near(q, a, b)
            };
            if let Some(slot) = per.get_mut(seg)
                && better(d, slot.0)
            {
                *slot = (d, t0 + (t1 - t0) * u);
            }
        }
        let best = per.iter().fold(worst, |m, p| if better(p.0, m) { p.0 } else { m });
        let window = 2.0 * self.slack + 1e-12;
        let mut out: Option<(Point, f64)> = None;
        for (seg, &(d, t)) in per.iter().enumerate() {
            if (d - best).abs() > window {
                continue;
            }
            let mut tries = vec![(seg, t)];
            let (before, after) = self.beside.get(seg).copied().unwrap_or((None, None));
            if t <= 0.0
                && let Some(b) = before
            {
                tries.push((b, 1.0));
            }
            if t >= 1.0
                && let Some(a) = after
            {
                tries.push((a, 0.0));
            }
            for (s, t) in tries {
                let Some(curve) = self.segments.get(s) else { continue };
                let (p, d) = polish(*curve, q, t, far);
                if out.is_none_or(|(_, od)| better(d, od)) {
                    out = Some((p, d));
                }
            }
        }
        out.unwrap_or((q, 0.0))
    }
}

/// The distance from `q` to the chord from `a` to `b`, and the parameter along it where it is reached.
fn chord_near(q: Point, a: Point, b: Point) -> (f64, f64) {
    let ab = b - a;
    let len2 = ab.hypot2();
    let u = if len2 > 0.0 { ((q - a).dot(ab) / len2).clamp(0.0, 1.0) } else { 0.0 };
    ((q - (a + ab * u)).hypot(), u)
}

struct Reach {
    near: f64,
    far: f64,
}

/// The nearest (or farthest) point to `q` on `seg` near parameter `t`: Newton's method on
/// (B(t) − q)·B′(t) = 0, kept in 0..1, keeping only steps that improve the distance.
fn polish(seg: CubicBez, q: Point, t: f64, far: bool) -> (Point, f64) {
    let dist = |t: f64| (seg.eval(t) - q).hypot();
    let better = |a: f64, b: f64| if far { a > b } else { a < b };
    let (mut t, mut d) = (t, dist(t));
    let [p0, p1, p2, p3] = [seg.p0, seg.p1, seg.p2, seg.p3].map(Point::to_vec2);
    for _ in 0..16 {
        let mt = 1.0 - t;
        let d1 = 3.0 * (mt * mt * (p1 - p0) + 2.0 * mt * t * (p2 - p1) + t * t * (p3 - p2));
        let d2 = 6.0 * (mt * (p2 - 2.0 * p1 + p0) + t * (p3 - 2.0 * p2 + p1));
        let w = seg.eval(t) - q;
        let f = w.dot(d1);
        let df = d1.hypot2() + w.dot(d2);
        if df == 0.0 || !df.is_finite() {
            break;
        }
        let next = (t - f / df).clamp(0.0, 1.0);
        let nd = dist(next);
        if !better(nd, d) && nd != d {
            break;
        }
        let moved = (next - t).abs();
        (t, d) = (next, nd);
        if moved < 1e-15 {
            break;
        }
    }
    (seg.eval(t), d)
}

/// Most grid cells the search looks at, over all levels.
const MAX_CELLS: usize = 400_000;
/// Most cells × chords the search may measure.
const MAX_WORK: f64 = 1.5e8;
/// Most cells Newton's method starts from.
const MAX_TRIES: usize = 4_000;
/// Most chords Newton's method may measure, over all its starts.
const MAX_NEWTON_WORK: f64 = 1.5e8;

/// Which of a shape's two distances a condition uses: the nearest point (the outline outside the
/// circle) or the farthest (the outline inside it).
fn far_of(combo: usize, shape: usize) -> bool {
    combo & (1 << shape) != 0
}

/// The circles touching `outlines` (local units): two outlines and radius `r`, or three and none.
fn numeric(outlines: &[Outline], r: Option<f64>) -> Result<Vec<Circle>, TangentCircleError> {
    let n = outlines.len();
    let combos = 1usize << n;
    let slack = outlines.iter().map(|o| o.slack).fold(0.0, f64::max);
    let chords: usize = outlines.iter().map(|o| o.chords.len()).sum();
    let reach = match r {
        Some(r) => r,
        None => MAX_RADIUS_FACTOR,
    };
    // A center is within the radius of a point of every outline, so within its bounds grown by it.
    let mut area: Option<kurbo::Rect> = None;
    let mut whole: Option<kurbo::Rect> = None;
    for o in outlines {
        let b = o.chords.iter().fold(None::<kurbo::Rect>, |b, &(p, q, ..)| {
            let c = kurbo::Rect::from_points(p, q);
            Some(b.map_or(c, |b| b.union(c)))
        });
        let Some(b) = b else { return Err(TangentCircleError::BadInput) };
        whole = Some(whole.map_or(b, |w| w.union(b)));
        let b = b.inflate(reach + slack, reach + slack);
        area = Some(area.map_or(b, |a| a.intersect(b)));
    }
    let whole = whole.ok_or(TangentCircleError::BadInput)?.inflate(slack, slack);
    let span = whole.width().hypot(whole.height());
    let Some(area) = area.filter(|a| a.width() >= 0.0 && a.height() >= 0.0) else { return Ok(vec![]) };
    let side = area.width().max(area.height()).max(1e-12);
    let leaf = 2.0 * slack.max(1e-4);
    let mut half = side / 2.0;
    // The cells alive at the current level: (center, combos still possible).
    let all = (1u32 << combos) - 1;
    let mut cells = vec![(area.center(), all)];
    let mut looked = 0usize;
    let test = |c: Point, half: f64, alive: u32| -> u32 {
        let corner = half * std::f64::consts::SQRT_2;
        let e = corner + slack;
        // Seen from far off, the directions to any two points of the outlines differ little, and
        // so do the gradients of their distances: the difference of two distances changes by at
        // most 2 × span / (distance to the outlines) per unit moved, rather than 2.
        let off = {
            let dx = (whole.x0 - c.x).max(c.x - whole.x1).max(0.0);
            let dy = (whole.y0 - c.y).max(c.y - whole.y1).max(0.0);
            dx.hypot(dy) - corner
        };
        let steep = if off > 0.0 { (2.0 * span / off).min(2.0) } else { 2.0 };
        let apart = steep * corner + 2.0 * slack;
        let reach: Vec<Reach> = outlines.iter().map(|o| o.reach(c)).collect();
        let d = |i: usize, combo: usize| if far_of(combo, i) { reach[i].far } else { reach[i].near };
        let mut keep = 0;
        for combo in 0..combos {
            if alive & (1 << combo) == 0 {
                continue;
            }
            let ok = match r {
                Some(r) => (0..n).all(|i| (d(i, combo) - r).abs() <= e),
                None => {
                    let (d0, d1, d2) = (d(0, combo), d(1, combo), d(2, combo));
                    (d0 - d1).abs() <= apart && (d0 - d2).abs() <= apart && (d1 - d2).abs() <= apart && d0 - e <= MAX_RADIUS_FACTOR
                }
            };
            if ok {
                keep |= 1 << combo;
            }
        }
        keep
    };
    loop {
        looked += cells.len();
        if looked > MAX_CELLS || looked as f64 * chords as f64 > MAX_WORK {
            return Err(TangentCircleError::TooComplex);
        }
        cells = cells.into_iter().filter_map(|(c, alive)| Some((c, test(c, half, alive))).filter(|x| x.1 != 0)).collect();
        if half * std::f64::consts::SQRT_2 <= leaf || cells.is_empty() {
            break;
        }
        let q = half / 2.0;
        cells = cells
            .into_iter()
            .flat_map(|(c, alive)| [(-q, -q), (q, -q), (-q, q), (q, q)].map(|(dx, dy)| (Point::new(c.x + dx, c.y + dy), alive)))
            .collect();
        half = q;
    }
    // The leaf cells of each combination, in clusters of touching cells. Newton's method starts
    // from up to [`SEEDS`] cells of each cluster, each the farthest from those before it. A cluster
    // around one center (or two that nearly meet) gives one or two circles; three or more far apart
    // mean the shapes are touched along a whole curve.
    let step = 2.0 * half;
    let mut out = vec![];
    let mut tries = 0usize;
    let mut work = 0.0;
    let same = |a: &Circle, b: &Circle| (a.center - b.center).hypot() <= SAME && (a.radius - b.radius).abs() <= SAME;
    let mut solve = |seed: Point, combo: usize| -> Result<Option<Circle>, TangentCircleError> {
        tries += 1;
        let c = newton(outlines, r, combo, seed, half, &mut work);
        if tries > MAX_TRIES || work > MAX_NEWTON_WORK {
            return Err(TangentCircleError::TooComplex);
        }
        Ok(c)
    };
    for combo in 0..combos {
        let leaves: Vec<Point> = cells.iter().filter(|(_, a)| a & (1 << combo) != 0).map(|(c, _)| *c).collect();
        let mut found: Vec<Circle> = vec![];
        for cluster in clusters(&leaves, step) {
            let mut seeds: Vec<Point> = vec![];
            let mut here: Vec<Circle> = vec![];
            while seeds.len() < SEEDS {
                let next = cluster
                    .iter()
                    .copied()
                    .map(|c| (c, seeds.iter().map(|s| (*s - c).hypot()).fold(f64::INFINITY, f64::min)))
                    .max_by(|a, b| a.1.total_cmp(&b.1));
                let Some((seed, gap)) = next else { break };
                if !seeds.is_empty() && gap <= 2.0 * step {
                    break;
                }
                seeds.push(seed);
                if let Some(c) = solve(seed, combo)?
                    && !here.iter().any(|o| same(o, &c))
                {
                    here.push(c);
                }
            }
            if here.len() > 2 {
                return Err(TangentCircleError::TooMany);
            }
            for c in here {
                if !found.iter().any(|o| same(o, &c)) {
                    found.push(c);
                }
            }
            if found.len() > MAX_TANGENT_CIRCLES {
                return Err(TangentCircleError::TooMany);
            }
        }
        out.extend(found);
    }
    Ok(out)
}

/// Seeds Newton's method starts from in one cluster of cells, at most.
const SEEDS: usize = 4;

/// `cells`, centers on a grid `step` apart, in groups of cells that touch (sides or corners).
fn clusters(cells: &[Point], step: f64) -> Vec<Vec<Point>> {
    use std::collections::HashMap;
    let Some(first) = cells.first() else { return vec![] };
    let key = |p: &Point| (((p.x - first.x) / step).round() as i64, ((p.y - first.y) / step).round() as i64);
    let mut at: HashMap<(i64, i64), usize> = cells.iter().enumerate().map(|(i, p)| (key(p), i)).collect();
    let mut out = vec![];
    for p in cells {
        let Some(start) = at.remove(&key(p)) else { continue };
        let mut group = vec![];
        let mut stack = vec![start];
        while let Some(i) = stack.pop() {
            let Some(c) = cells.get(i) else { continue };
            group.push(*c);
            let (kx, ky) = key(c);
            for dx in -1..=1 {
                for dy in -1..=1 {
                    if let Some(j) = at.remove(&(kx + dx, ky + dy)) {
                        stack.push(j);
                    }
                }
            }
        }
        out.push(group);
    }
    out
}

/// Newton's method from `seed` on the conditions of `combo`: with `r`, distance(i) = r for both
/// outlines; without, the three distances equal (the radius). Adds the chords it measures to `work`.
fn newton(outlines: &[Outline], r: Option<f64>, combo: usize, seed: Point, step: f64, work: &mut f64) -> Option<Circle> {
    let mut c = seed;
    let n = outlines.len();
    let cost: f64 = outlines.iter().map(|o| o.chords.len() as f64).sum();
    let limit = (step * 8.0).max(1e-3);
    // A radius the search sets itself (three shapes) is kept within its limit.
    let fits = |radius: f64| radius > EPS && (r.is_some() || radius <= MAX_RADIUS_FACTOR);
    for _ in 0..60 {
        *work += cost;
        let mut d = [0.0; 3];
        let mut u = [Vec2::ZERO; 3];
        for i in 0..n {
            let (p, di) = outlines[i].extreme(c, far_of(combo, i));
            if di <= EPS {
                return None;
            }
            d[i] = di;
            u[i] = (c - p) / di;
        }
        let (f, rows, radius) = match r {
            Some(r) => ([d[0] - r, d[1] - r], [u[0], u[1]], r),
            None => ([d[0] - d[1], d[0] - d[2]], [u[0] - u[1], u[0] - u[2]], d[0]),
        };
        if f[0].abs().max(f[1].abs()) <= 1e-12 {
            return fits(radius).then_some(Circle { center: c, radius });
        }
        let det = rows[0].cross(rows[1]);
        let mut delta = if det.abs() > 1e-9 {
            // J Δ = −f, with J's rows the gradients.
            Vec2::new(-(f[0] * rows[1].y - f[1] * rows[0].y) / det, -(rows[0].x * f[1] - rows[1].x * f[0]) / det)
        } else {
            // Rows (nearly) parallel: the least-squares step (JᵀJ + λI) Δ = −Jᵀf, which still
            // closes the residual along the rows' direction.
            let g = rows[0] * f[0] + rows[1] * f[1];
            let lambda = 1e-12;
            let (a, b, d) = (
                rows[0].x * rows[0].x + rows[1].x * rows[1].x + lambda,
                rows[0].x * rows[0].y + rows[1].x * rows[1].y,
                rows[0].y * rows[0].y + rows[1].y * rows[1].y + lambda,
            );
            let m = a * d - b * b;
            if !m.is_finite() || m == 0.0 {
                break;
            }
            Vec2::new(-(d * g.x - b * g.y) / m, -(a * g.y - b * g.x) / m)
        };
        let len = delta.hypot();
        if !len.is_finite() {
            return None;
        }
        if len > limit {
            delta *= limit / len;
        }
        c += delta;
        if delta.hypot() <= 1e-15 {
            break;
        }
    }
    // Stopped short of 1e-12: keep it only when it is close (a double root converges slowly).
    *work += cost;
    let mut d = [0.0; 3];
    for i in 0..n {
        d[i] = outlines[i].extreme(c, far_of(combo, i)).1;
    }
    let (worst, radius) = match r {
        Some(r) => ((d[0] - r).abs().max((d[1] - r).abs()), r),
        None => ((d[0] - d[1]).abs().max((d[0] - d[2]).abs()), d[0]),
    };
    (worst <= 1e-8 && fits(radius)).then_some(Circle { center: c, radius })
}
