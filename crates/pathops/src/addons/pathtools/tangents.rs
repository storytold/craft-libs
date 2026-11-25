//! Common Tangents: the straight lines that touch two convex closed outlines (ellipses at any
//! rotation and scale, and convex Bézier paths) without crossing into either. An external tangent
//! has both outlines on the same side of it; an internal one runs between them.
//!
//! A line with unit normal `n` touches an outline from outside where `p · n` reaches the
//! outline's support value `h(n)`, the largest `p · n` over its points. External tangents are the
//! normals where both outlines have the same support value, internal ones where `h₁(n) = -h₂(-n)`.
//! Both are found as roots over the normal's angle: sampled, then refined by bisection, with
//! sampled minima refined too so that a double root (outlines that touch) or two roots between
//! neighboring samples are kept.
//!
//! One of the two may be a point instead ([`Outline::point`]): its support value is `p · n`, and
//! the tangents are the lines through it that touch the other outline.

use std::f64::consts::TAU;

use craft_geom::SubPath;
use kurbo::{Affine, CubicBez, ParamCurve, Point, Vec2};

/// Most segments one outline may have.
pub const MAX_TANGENT_SEGMENTS: usize = 10_000;
/// Normal directions sampled around the circle before refining.
const SAMPLES: usize = 720;
/// Steps of bisection or golden-section search for each root.
const STEPS: usize = 64;
/// Straight pieces each segment is cut into to check that a path is convex and to find its center.
const FLATTEN: usize = 32;

/// Which side of a tangent the two outlines lie on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TangentKind {
    /// Both on the same side.
    External,
    /// One on each side.
    Internal,
}

/// One common tangent: the stretch of the line from where it touches the first outline to where
/// it touches the second. Where the outlines touch each other at one point, the stretch is
/// centered on that point and as long as the narrower outline is wide along the line, and `from`
/// is the end with the lower position along the line.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Tangent {
    pub kind: TangentKind,
    pub from: Point,
    pub to: Point,
}

/// Why [`Outline::convex`] or [`Outline::ellipse`] cannot take a shape.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OutlineError {
    /// The path is open.
    Open,
    /// The path bends inward somewhere, or winds around more than once.
    NotConvex,
    /// No area, or not finite.
    Empty,
    /// More than [`MAX_TANGENT_SEGMENTS`] segments.
    TooComplex,
}

/// The two outlines are the same, so every line touching one touches the other.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Coincide;

/// A convex closed outline, or a point, that [`common_tangents`] takes.
#[derive(Clone, Debug)]
pub enum Outline {
    /// The unit circle under an affine map: the center is the map's translation, `m` its linear
    /// part (`[a, b, c, d]`, as kurbo orders them).
    Ellipse { center: Point, m: [f64; 4] },
    /// A convex closed path's segments, its center of area and the diagonal of its bounds. `ccw`
    /// when the segments run counter-clockwise (y up); the segment where `p · n` is largest then
    /// comes later in the list as the angle of `n` grows.
    Convex { segments: Vec<CubicBez>, center: Point, size: f64, ccw: bool },
    /// A single point.
    Point { at: Point },
}

impl Outline {
    /// The point `at`, whose tangents are the lines through it.
    pub fn point(at: Point) -> Result<Self, OutlineError> {
        if !at.x.is_finite() || !at.y.is_finite() {
            return Err(OutlineError::Empty);
        }
        Ok(Outline::Point { at })
    }

    /// The ellipse that `unit` maps the unit circle onto.
    pub fn ellipse(unit: Affine) -> Result<Self, OutlineError> {
        let [a, b, c, d, e, f] = unit.as_coeffs();
        let norm = a.abs().max(b.abs()).max(c.abs()).max(d.abs());
        // The determinant is rounded by about `f64::EPSILON * norm²`; one no larger than that may
        // belong to a map with no area. An ellipse whose determinant is larger is measured
        // exactly.
        if !unit.as_coeffs().iter().all(|v| v.is_finite()) || norm <= 0.0 || (a * d - b * c).abs() <= f64::EPSILON * norm * norm {
            return Err(OutlineError::Empty);
        }
        Ok(Outline::Ellipse { center: Point::new(e, f), m: [a, b, c, d] })
    }

    /// A closed subpath that bends one way all around and winds around once.
    pub fn convex(sp: &SubPath) -> Result<Self, OutlineError> {
        if !sp.closed {
            return Err(OutlineError::Open);
        }
        let n = sp.segment_count();
        if n > MAX_TANGENT_SEGMENTS {
            return Err(OutlineError::TooComplex);
        }
        let segments: Vec<CubicBez> = (0..n).map(|k| sp.segment(k)).collect();
        let finite = |p: Point| p.x.is_finite() && p.y.is_finite();
        if segments.iter().any(|s| ![s.p0, s.p1, s.p2, s.p3].into_iter().all(finite)) {
            return Err(OutlineError::Empty);
        }
        let mut pts: Vec<Point> = Vec::with_capacity(n * FLATTEN);
        for s in &segments {
            pts.extend((0..FLATTEN).map(|k| s.eval(k as f64 / FLATTEN as f64)));
        }
        let (lo, hi) = pts.iter().fold((Point::new(f64::MAX, f64::MAX), Point::new(f64::MIN, f64::MIN)), |(lo, hi), p| {
            (Point::new(lo.x.min(p.x), lo.y.min(p.y)), Point::new(hi.x.max(p.x), hi.y.max(p.y)))
        });
        let size = (hi - lo).hypot();
        if !size.is_finite() || size <= 0.0 {
            return Err(OutlineError::Empty);
        }
        // Points closer than this to the one before are dropped: a retracted handle or a segment
        // of no length gives no direction.
        pts.dedup_by(|b, a| (*b - *a).hypot() <= 1e-9 * size);
        while pts.len() > 1 && pts.first().zip(pts.last()).is_some_and(|(f, l)| (*f - *l).hypot() <= 1e-9 * size) {
            pts.pop();
        }
        let m = pts.len();
        if m < 3 {
            return Err(OutlineError::Empty);
        }
        // Area, center and turns are worked out on the points taken about the first one and
        // divided by the size.
        let origin = pts[0].to_vec2();
        let unit: Vec<Vec2> = pts.iter().map(|p| (p.to_vec2() - origin) / size).collect();
        let (mut area2, mut cx, mut cy) = (0.0, 0.0, 0.0);
        for i in 0..m {
            let (p, q) = (unit[i], unit[(i + 1) % m]);
            let cross = p.cross(q);
            area2 += cross;
            cx += (p.x + q.x) * cross;
            cy += (p.y + q.y) * cross;
        }
        if area2.abs() <= 1e-12 {
            return Err(OutlineError::Empty);
        }
        let sign = area2.signum();
        // Every turn the same way (straight runs allowed), and one turn in all.
        let mut turning = 0.0;
        for i in 0..m {
            let (p, q, r) = (unit[(i + m - 1) % m], unit[i], unit[(i + 1) % m]);
            let (u, v) = (q - p, r - q);
            let cross = u.cross(v) * sign;
            if cross < -1e-12 {
                return Err(OutlineError::NotConvex);
            }
            turning += cross.atan2(u.dot(v));
        }
        if (turning - TAU).abs() > 1e-6 {
            return Err(OutlineError::NotConvex);
        }
        // A short bend the other way can lie between two of the points: each segment is checked
        // whole.
        if segments.iter().any(|s| bends_against(s, sign)) {
            return Err(OutlineError::NotConvex);
        }
        // A corner can also turn inward between two of the points: each join is checked by the
        // directions the segments leave and reach it in. Segments of no length are passed over.
        let nonzero = |vs: [Vec2; 3]| vs.into_iter().find(|v| v.hypot() > 1e-9 * size).map(|v| v / v.hypot());
        let ends: Vec<(Vec2, Vec2)> = segments
            .iter()
            .filter_map(|s| nonzero([s.p1 - s.p0, s.p2 - s.p0, s.p3 - s.p0]).zip(nonzero([s.p3 - s.p2, s.p3 - s.p1, s.p3 - s.p0])))
            .collect();
        if ends.iter().enumerate().any(|(i, (_, end))| ends[(i + 1) % ends.len()].0.cross(*end) * sign > 1e-9) {
            return Err(OutlineError::NotConvex);
        }
        let center = (Vec2::new(cx / (3.0 * area2), cy / (3.0 * area2)) * size + origin).to_point();
        if !center.x.is_finite() || !center.y.is_finite() {
            return Err(OutlineError::Empty);
        }
        Ok(Outline::Convex { segments, center, size, ccw: sign > 0.0 })
    }

    /// The outline moved by `d`.
    fn shifted(&self, d: Vec2) -> Self {
        match self {
            Outline::Ellipse { center, m } => Outline::Ellipse { center: *center + d, m: *m },
            Outline::Convex { segments, center, size, ccw } => Outline::Convex {
                segments: segments.iter().map(|s| CubicBez::new(s.p0 + d, s.p1 + d, s.p2 + d, s.p3 + d)).collect(),
                center: *center + d,
                size: *size,
                ccw: *ccw,
            },
            Outline::Point { at } => Outline::Point { at: *at + d },
        }
    }

    /// Adds to `out` the normal angles where the point of largest `p · n` can pass from one
    /// segment to the next: square to each segment's direction at its ends, on both sides.
    fn breaks(&self, out: &mut Vec<f64>) {
        let Outline::Convex { segments, .. } = self else { return };
        let nonzero = |vs: [Vec2; 3]| vs.into_iter().find(|v| v.hypot2() > 0.0);
        for s in segments {
            let ends = [nonzero([s.p1 - s.p0, s.p2 - s.p0, s.p3 - s.p0]), nonzero([s.p3 - s.p2, s.p3 - s.p1, s.p3 - s.p0])];
            for d in ends.into_iter().flatten() {
                let a = d.y.atan2(d.x);
                out.extend([a + TAU / 4.0, a - TAU / 4.0]);
            }
        }
    }

    /// The support value at `n` and the segment it is reached on, from every segment.
    fn support_scan(&self, n: Vec2) -> (f64, usize) {
        match self {
            Outline::Ellipse { .. } | Outline::Point { .. } => (self.support(n), 0),
            Outline::Convex { segments, .. } => {
                let mut best = (f64::MIN, 0);
                for (i, s) in segments.iter().enumerate() {
                    each_extreme(s, n, |_, v| {
                        if v > best.0 {
                            best = (v, i);
                        }
                    });
                }
                best
            }
        }
    }

    /// The support value at `n` and the segment it is reached on, walking on from segment `from`,
    /// where the support value was reached at a normal angle a little less than `n`'s.
    fn support_from(&self, n: Vec2, from: usize) -> (f64, usize) {
        match self {
            Outline::Ellipse { .. } | Outline::Point { .. } => (self.support(n), 0),
            Outline::Convex { segments, ccw, .. } => {
                let len = segments.len();
                if len == 0 {
                    return (f64::MIN, 0);
                }
                let step = if *ccw { 1 } else { len - 1 };
                let value = |i: usize| {
                    let mut best = f64::MIN;
                    if let Some(s) = segments.get(i) {
                        each_extreme(s, n, |_, v| best = best.max(v));
                    }
                    best
                };
                let mut i = from % len;
                let mut v = value(i);
                for _ in 0..len {
                    let j = (i + step) % len;
                    let w = value(j);
                    if w < v {
                        break;
                    }
                    (i, v) = (j, w);
                }
                (v, i)
            }
        }
    }

    /// The center: an ellipse's, a path's center of area, or the point itself.
    pub fn center(&self) -> Point {
        match self {
            Outline::Ellipse { center, .. } | Outline::Convex { center, .. } => *center,
            Outline::Point { at } => *at,
        }
    }

    /// Whether this is a point.
    pub fn is_point(&self) -> bool {
        matches!(self, Outline::Point { .. })
    }

    /// The diagonal of the bounds (an ellipse's: of the box its axes span; a point's: 0).
    pub fn size(&self) -> f64 {
        match self {
            Outline::Ellipse { m, .. } => {
                let [a, b, c, d] = *m;
                2.0 * (a.hypot(c).powi(2) + b.hypot(d).powi(2)).sqrt()
            }
            Outline::Convex { size, .. } => *size,
            Outline::Point { .. } => 0.0,
        }
    }

    /// The support value: the largest `p · n` over the outline's points.
    fn support(&self, n: Vec2) -> f64 {
        match self {
            Outline::Ellipse { center, m: [a, b, c, d] } => center.to_vec2().dot(n) + Vec2::new(a * n.x + b * n.y, c * n.x + d * n.y).hypot(),
            Outline::Convex { segments, .. } => {
                let mut best = f64::MIN;
                for s in segments {
                    each_extreme(s, n, |_, v| best = best.max(v));
                }
                best
            }
            Outline::Point { at } => at.to_vec2().dot(n),
        }
    }

    /// The points of the outline on the line `p · n = h`, its support value, which is reached on
    /// segment `at`: the point where `p · n` is largest, and the ends of any straight side along
    /// the line. Another point counts when it lies within `tol` of the line plus `slope` times its
    /// distance from that largest point, where `slope` is how far off the normal's angle may be.
    /// On a convex outline the points that count form one run around the largest one, so the
    /// segments are measured from `at` outward both ways until a segment's end does not count.
    fn touching(&self, n: Vec2, h: f64, tol: f64, slope: f64, at: usize) -> Vec<Point> {
        match self {
            Outline::Ellipse { center, m: [a, b, c, d] } => {
                let w = Vec2::new(a * n.x + b * n.y, c * n.x + d * n.y);
                let w = w / w.hypot();
                vec![*center + Vec2::new(a * w.x + c * w.y, b * w.x + d * w.y)]
            }
            Outline::Convex { segments, .. } => {
                let len = segments.len();
                let mut all = vec![];
                let Some(first) = segments.get(at % len.max(1)) else { return vec![] };
                each_extreme(first, n, |p, v| all.push((p, v)));
                let Some(&(peak, _)) = all.iter().max_by(|x, y| x.1.total_cmp(&y.1)) else { return vec![] };
                let counts = |p: Point| h - p.to_vec2().dot(n) <= tol + slope * (p - peak).hypot();
                let mut ahead = 0;
                for j in 1..len {
                    match segments.get((at + j) % len) {
                        Some(s) if counts(s.p0) => each_extreme(s, n, |p, v| all.push((p, v))),
                        _ => break,
                    }
                    ahead = j;
                }
                for j in 1..len - ahead {
                    match segments.get((at + len - j) % len) {
                        Some(s) if counts(s.p3) => each_extreme(s, n, |p, v| all.push((p, v))),
                        _ => break,
                    }
                }
                let Some(&(peak, _)) = all.iter().max_by(|x, y| x.1.total_cmp(&y.1)) else { return vec![] };
                all.into_iter().filter(|(p, v)| h - v <= tol + slope * (*p - peak).hypot()).map(|(p, _)| p).collect()
            }
            Outline::Point { at } => vec![*at],
        }
    }
}

/// Whether cubic `s` bends against `sign` (the way its outline turns) anywhere. The cross product
/// of its first and second derivatives is a quadratic in t; its lowest value on [0, 1] is compared
/// with the segment's control polygon length squared.
fn bends_against(s: &CubicBez, sign: f64) -> bool {
    let (a, b, c) = (s.p1 - s.p0, s.p2 - s.p1, s.p3 - s.p2);
    let q = |t: f64| {
        let u = 1.0 - t;
        let d1 = a * (u * u) + b * (2.0 * u * t) + c * (t * t);
        let d2 = (b - a) * u + (c - b) * t;
        d1.cross(d2) * sign
    };
    let (q0, half, q1) = (q(0.0), q(0.5), q(1.0));
    // q(t) = q0 + k1 t + k2 t².
    let k2 = 2.0 * (q0 + q1 - 2.0 * half);
    let k1 = q1 - q0 - k2;
    let mut low = q0.min(q1);
    if k2 > 0.0 {
        let t = -k1 / (2.0 * k2);
        if t > 0.0 && t < 1.0 {
            low = low.min(q(t));
        }
    }
    let scale = (a.hypot() + b.hypot() + c.hypot()).powi(2);
    low < -1e-9 * scale
}

#[cfg(test)]
thread_local! {
    /// How many times [`each_extreme`] has measured a segment on this thread, for the tests.
    static EXTREMES: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

/// Calls `f` with each point of `s` where `p · n` can be largest (its ends and the places where
/// the curve runs square to `n`) and the value there.
fn each_extreme(s: &CubicBez, n: Vec2, mut f: impl FnMut(Point, f64)) {
    #[cfg(test)]
    EXTREMES.with(|n| n.set(n.get() + 1));
    let [v0, v1, v2, v3] = [s.p0, s.p1, s.p2, s.p3].map(|p| p.to_vec2().dot(n));
    f(s.p0, v0);
    f(s.p3, v3);
    // The derivative of p(t) · n, divided by 3: a quadratic in t.
    let (d0, d1, d2) = (v1 - v0, v2 - v1, v3 - v2);
    let (qa, qb, qc) = (d0 - 2.0 * d1 + d2, 2.0 * (d1 - d0), d0);
    let mut root = |t: f64| {
        if t > 0.0 && t < 1.0 {
            let p = s.eval(t);
            f(p, p.to_vec2().dot(n));
        }
    };
    let scale = qa.abs().max(qb.abs()).max(qc.abs());
    if !scale.is_finite() || scale <= 0.0 {
        return;
    }
    if qa.abs() <= 1e-12 * scale {
        if qb != 0.0 {
            root(-qc / qb);
        }
        return;
    }
    let disc = qb * qb - 4.0 * qa * qc;
    if disc < 0.0 {
        return;
    }
    // The stable form of the two roots; with `qb` zero the sign is taken as positive, so `q` is
    // not zero.
    let sign = if qb < 0.0 { -1.0 } else { 1.0 };
    let q = -0.5 * (qb + sign * disc.sqrt());
    root(q / qa);
    if q != 0.0 {
        root(qc / q);
    }
}

/// The unit vector at `angle` radians.
fn dir(angle: f64) -> Vec2 {
    Vec2::new(angle.cos(), angle.sin())
}

/// The common tangents of `a` and `b`: the external ones, then the internal ones, each in order of
/// the angle of its normal. Separate outlines have two of each; overlapping ones only external
/// ones (two for ellipses, possibly more for paths whose outlines cross more often); one inside the
/// other none; outlines that touch share one tangent where they touch.
///
/// When one of them is a point, its external and internal tangents are the same lines, and they
/// come back as external ones: two when the point lies outside the other outline, one when it lies
/// on it, none inside. Two points have none.
pub fn common_tangents(a: &Outline, b: &Outline) -> Result<Vec<Tangent>, Coincide> {
    if a.is_point() && b.is_point() {
        return Ok(vec![]);
    }
    // Worked out about the point between the two centers.
    let origin = (a.center().to_vec2() + b.center().to_vec2()) / 2.0;
    let (a, b) = (&a.shifted(-origin), &b.shifted(-origin));
    let size = a.size().max(b.size()).max((a.center() - b.center()).hypot());
    let tol = 1e-9 * size;
    // The normal angles sampled: evenly spaced ones, and every angle where the support value of
    // either outline passes to another segment. Between two neighboring samples each outline is
    // then touched on one segment.
    let mut at: Vec<f64> = (0..SAMPLES).map(|k| k as f64 * TAU / SAMPLES as f64).collect();
    a.breaks(&mut at);
    b.breaks(&mut at);
    let mut at: Vec<f64> = at.into_iter().map(|t| t.rem_euclid(TAU)).map(|t| if t < TAU { t } else { 0.0 }).collect();
    at.sort_by(f64::total_cmp);
    at.dedup_by(|y, x| *y - *x <= 1e-12);
    // The segment each outline is touched on at each sample, found by walking around once.
    let hints = |o: &Outline, sign: f64| -> Vec<usize> {
        let mut out = Vec::with_capacity(at.len());
        let mut i = at.first().map_or(0, |t| o.support_scan(dir(*t) * sign).1);
        for t in &at {
            i = o.support_from(dir(*t) * sign, i).1;
            out.push(i);
        }
        out
    };
    let (ha, hb, hb_back) = (hints(a, 1.0), hints(b, 1.0), hints(b, -1.0));
    let hint = |h: &[usize], k: usize| h.get(k).copied().unwrap_or(0);
    let external = roots(&at, &|t, k| a.support_from(dir(t), hint(&ha, k)).0 - b.support_from(dir(t), hint(&hb, k)).0, tol)?;
    let internal = if a.is_point() || b.is_point() {
        vec![]
    } else {
        roots(&at, &|t, k| a.support_from(dir(t), hint(&ha, k)).0 + b.support_from(-dir(t), hint(&hb_back, k)).0, tol)?
    };
    let (a, b) = (Hinted { o: a, at: &at, hints: &ha }, Hinted { o: b, at: &at, hints: &hb });
    let mut out = Vec::with_capacity(external.len() + internal.len());
    for (kind, list) in [(TangentKind::External, external), (TangentKind::Internal, internal)] {
        for root in list {
            let t = tangent_at(&a, &b, kind, root.angle, root.double, size);
            out.push(Tangent { from: t.from + origin, to: t.to + origin, ..t });
        }
    }
    Ok(out)
}

/// A zero of a support-value difference: the normal's angle, and whether the difference only
/// touches zero there (outlines that touch) rather than crossing it.
#[derive(Clone, Copy, Debug)]
struct Root {
    angle: f64,
    double: bool,
}

/// An outline with the segment it is touched on at each sampled normal angle.
struct Hinted<'a> {
    o: &'a Outline,
    at: &'a [f64],
    hints: &'a [usize],
}

impl Hinted<'_> {
    /// The support value at the normal angle `angle` and the segment it is reached on, walking on
    /// from the segment of the sample at or before `angle`.
    fn support(&self, angle: f64) -> (f64, usize) {
        let angle = angle.rem_euclid(TAU);
        // Before the first sample, the last one (taken 2π back) comes before `angle`.
        let k = self.at.partition_point(|t| *t <= angle).checked_sub(1).unwrap_or(self.at.len().saturating_sub(1));
        self.o.support_from(dir(angle), self.hints.get(k).copied().unwrap_or(0))
    }
}

/// The tangent of `kind` whose normal is at `angle`: `a` lies where `p · n` is at most the line's
/// value, and `b` on the same side (external) or the other side (internal). `double` when the
/// outlines touch there.
fn tangent_at(a: &Hinted, b: &Hinted, kind: TangentKind, angle: f64, double: bool, size: f64) -> Tangent {
    let n = dir(angle);
    // The rounding allowed in `p · n` grows with each outline's own size and its distance from the
    // origin.
    let rounding = |o: &Outline| 1e-12 * o.size() + 1e-14 * (o.center().to_vec2().hypot() + o.size());
    let (tol_a, tol_b) = (rounding(a.o), rounding(b.o));
    // Bisection finds a simple root's angle to about the rounding of the support values; the
    // golden-section search finds a double root's to about 1e-8 radians.
    let slope = if double { 1e-7 } else { 1e-10 };
    let (ha, sa) = a.support(angle);
    let on_a = a.o.touching(n, ha, tol_a, slope, sa);
    let on_b = match kind {
        TangentKind::External => {
            let (hb, sb) = b.support(angle);
            b.o.touching(n, hb, tol_b, slope, sb)
        }
        TangentKind::Internal => {
            let (hb, sb) = b.support(angle + TAU / 2.0);
            b.o.touching(-n, hb, tol_b, slope, sb)
        }
    };
    // Positions along the line, which runs square to `n` through `n * ha`.
    let t = Vec2::new(-n.y, n.x);
    let along = |p: &Point| p.to_vec2().dot(t);
    let mean = |v: &[Point]| v.iter().map(along).sum::<f64>() / v.len().max(1) as f64;
    let all = on_a.iter().chain(&on_b).map(along);
    let (lo, hi) = all.fold((f64::MAX, f64::MIN), |(lo, hi), s| (lo.min(s), hi.max(s)));
    let at = |s: f64| (n * ha + t * s).to_point();
    // A double root's angle is found less exactly than a simple one's, so touching points may
    // stand a little apart.
    let (from, to) = if double && hi - lo <= 1e-6 * size {
        // Touching at one point: the narrower outline's width along the line, centered there. A
        // point on the other outline takes that outline's width.
        let width = [a, b]
            .into_iter()
            .filter(|o| !o.o.is_point())
            .map(|o| o.support(angle + TAU / 4.0).0 + o.support(angle - TAU / 4.0).0)
            .fold(f64::MAX, f64::min);
        let mid = (lo + hi) / 2.0;
        (mid - width / 2.0, mid + width / 2.0)
    } else if mean(&on_a) <= mean(&on_b) {
        (lo, hi)
    } else {
        (hi, lo)
    };
    Tangent { kind, from: at(from), to: at(to) }
}

/// The angles in `[0, 2π)` where `f` (periodic) is zero, within `tol`. An interval where it stays
/// zero (outlines that meet at a corner or along a shared stretch) gives the angle at its middle,
/// as a double root.
///
/// `at` holds the sampled angles in increasing order, in `[0, 2π)`; `f(t, k)` is `f` at an angle
/// `t` at or after sample `k` and before the sample two on.
fn roots(at: &[f64], f: &dyn Fn(f64, usize) -> f64, tol: f64) -> Result<Vec<Root>, Coincide> {
    let n = at.len();
    if n < 3 {
        return Err(Coincide);
    }
    // The angle of sample `k + j`, counted on from sample `k` past 2π.
    let ang = |k: usize, j: usize| at.get((k + j) % n).copied().unwrap_or(0.0) + TAU * ((k + j) / n) as f64;
    let v: Vec<f64> = (0..n).map(|k| f(ang(k, 0), k)).collect();
    let val = |k: usize| v.get(k % n).copied().unwrap_or(0.0);
    let zero = |x: f64| x.abs() <= tol;
    let Some(start) = v.iter().position(|x| !zero(*x)) else { return Err(Coincide) };
    let simple = |angle: f64| Root { angle, double: false };
    let mut out = vec![];
    // Between `lo` (sample `k`) and `hi`, where `s * f` is positive at both ends: the least of
    // `s * f`, a double root where it reaches zero, two roots where it goes past it, none where it
    // stays above.
    let dip = |out: &mut Vec<Root>, s: f64, k: usize, lo: f64, hi: f64| {
        let g = |t: f64| f(t, k);
        let (t, least) = golden(&|t| s * g(t), lo, hi);
        if least < -tol {
            out.push(simple(bisect(&g, lo, t)));
            out.push(simple(bisect(&g, t, hi)));
        } else if least <= tol {
            out.push(Root { angle: t, double: true });
        }
    };
    for i in 0..n {
        let k = (start + i) % n;
        let (prev, next) = ((k + n - 1) % n, k + 1);
        let theta = ang(k, 0);
        if zero(val(k)) {
            continue;
        }
        if zero(val(next)) {
            let len = (1..n).take_while(|j| zero(val(k + j))).count();
            if len == 1 {
                // One sample on zero: a crossing when the samples beside it differ in sign, else a
                // dip between them.
                let (lo, hi) = (theta, ang(k, 2));
                let s = val(k).signum();
                if val(k + 2).signum() == s {
                    dip(&mut out, s, k, lo, hi);
                } else {
                    out.push(simple(bisect(&|t| f(t, k), lo, hi)));
                }
            } else {
                out.push(Root { angle: (ang(k, 1) + ang(k, len)) / 2.0, double: true });
            }
            continue;
        }
        if val(k).signum() != val(next).signum() {
            out.push(simple(bisect(&|t| f(t, k), theta, ang(k, 1))));
            continue;
        }
        // A sampled minimum of |f|: the curve may dip to zero (a double root), or across it and
        // back (two roots), between the neighboring samples.
        let (vp, vk, vn) = (val(prev), val(k), val(next));
        if !zero(vp) && vp.signum() == vk.signum() && vk.abs() < vp.abs() && vk.abs() <= vn.abs() {
            let lo = if k == 0 { ang(prev, 0) - TAU } else { ang(prev, 0) };
            dip(&mut out, vk.signum(), prev, lo, ang(k, 1));
        }
    }
    let mut out: Vec<Root> = out.into_iter().map(|r| Root { angle: r.angle.rem_euclid(TAU), ..r }).collect();
    out.sort_by(|a, b| a.angle.total_cmp(&b.angle));
    out.dedup_by(|b, a| {
        let same = (b.angle - a.angle).abs() < 1e-7;
        a.double |= same && b.double;
        same
    });
    if out.len() > 1
        && out.first().zip(out.last()).is_some_and(|(f, l)| f.angle + TAU - l.angle < 1e-7)
        && let Some(last) = out.pop()
        && let Some(first) = out.first_mut()
    {
        first.double |= last.double;
    }
    Ok(out)
}

/// A root of `f` between `lo` and `hi`, where it has opposite signs.
fn bisect(f: &dyn Fn(f64) -> f64, mut lo: f64, mut hi: f64) -> f64 {
    let s = f(lo).signum();
    for _ in 0..STEPS {
        let mid = (lo + hi) / 2.0;
        if f(mid).signum() == s {
            lo = mid;
        } else {
            hi = mid;
        }
    }
    (lo + hi) / 2.0
}

/// Where `g` is least between `lo` and `hi` (taken as having one minimum there), and its value.
fn golden(g: &dyn Fn(f64) -> f64, mut lo: f64, mut hi: f64) -> (f64, f64) {
    let r = (5f64.sqrt() - 1.0) / 2.0;
    let (mut x1, mut x2) = (hi - r * (hi - lo), lo + r * (hi - lo));
    let (mut g1, mut g2) = (g(x1), g(x2));
    for _ in 0..STEPS {
        if g1 < g2 {
            hi = x2;
            (x2, g2) = (x1, g1);
            x1 = hi - r * (hi - lo);
            g1 = g(x1);
        } else {
            lo = x1;
            (x1, g1) = (x2, g2);
            x2 = lo + r * (hi - lo);
            g2 = g(x2);
        }
    }
    let t = (lo + hi) / 2.0;
    (t, g(t))
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;

    use craft_geom::shapes;
    use kurbo::Rect;

    use super::*;

    fn circle(cx: f64, cy: f64, r: f64) -> Outline {
        Outline::ellipse(Affine::new([r, 0.0, 0.0, r, cx, cy])).unwrap()
    }

    /// An ellipse with semi-axes `rx`, `ry`, turned `deg` degrees, centered at (`cx`, `cy`).
    fn ellipse_map(cx: f64, cy: f64, rx: f64, ry: f64, deg: f64) -> Affine {
        Affine::translate((cx, cy)) * Affine::rotate(deg.to_radians()) * Affine::scale_non_uniform(rx, ry)
    }

    /// Whether `t` has its two ends at `p` and `q`, in either order.
    fn ends_at(t: &Tangent, p: Point, q: Point) -> bool {
        ends_within(t, p, q, 1e-7)
    }

    /// [`ends_at`] within `eps`: the angle of a tangent where outlines touch (a double root) is
    /// found to about 1e-8 radians.
    fn ends_within(t: &Tangent, p: Point, q: Point, eps: f64) -> bool {
        let near = |a: Point, b: Point| (a - b).hypot() < eps;
        (near(t.from, p) && near(t.to, q)) || (near(t.from, q) && near(t.to, p))
    }

    /// Checks `t` against dense samples of the two outlines, without support values: each lies on
    /// one side of the line (the same side for an external tangent, opposite ones for an internal
    /// one) and comes within `eps` of it.
    fn check_against_samples(t: &Tangent, a: &[Point], b: &[Point], eps: f64) {
        let d = t.to - t.from;
        let n = Vec2::new(-d.y, d.x) / d.hypot();
        let side = |pts: &[Point]| {
            let dist: Vec<f64> = pts.iter().map(|p| (*p - t.from).dot(n)).collect();
            let (lo, hi) = dist.iter().fold((f64::MAX, f64::MIN), |(lo, hi), s| (lo.min(*s), hi.max(*s)));
            assert!(lo > -eps || hi < eps, "the line crosses the outline: {lo} .. {hi} ({t:?})");
            assert!(lo.abs() < eps || hi.abs() < eps, "the line misses the outline: {lo} .. {hi} ({t:?})");
            if hi < eps { -1.0 } else { 1.0 }
        };
        let (sa, sb) = (side(a), side(b));
        match t.kind {
            TangentKind::External => assert_eq!(sa, sb, "{t:?}"),
            TangentKind::Internal => assert_ne!(sa, sb, "{t:?}"),
        }
    }

    fn sample_ellipse(m: Affine, count: usize) -> Vec<Point> {
        (0..count).map(|k| m * Point::new((TAU * k as f64 / count as f64).cos(), (TAU * k as f64 / count as f64).sin())).collect()
    }

    fn sample_path(sp: &SubPath, per_segment: usize) -> Vec<Point> {
        (0..sp.segment_count()).flat_map(|k| (0..per_segment).map(move |j| sp.segment(k).eval(j as f64 / per_segment as f64))).collect()
    }

    #[test]
    fn two_circles_have_four_tangents_at_the_closed_form_points() {
        let (c1, r1, c2, r2) = (Point::new(0.0, 0.0), 10.0, Point::new(100.0, 0.0), 20.0);
        let all = common_tangents(&circle(c1.x, c1.y, r1), &circle(c2.x, c2.y, r2)).unwrap();
        assert_eq!(all.len(), 4, "{all:?}");
        // External: (c1 - c2) · n = r2 - r1, so n.x = -0.1; the points are c + r n on each.
        // Internal: (c1 - c2) · n = -(r1 + r2), so n.x = 0.3; c1 + r1 n and c2 - r2 n.
        for (kind, nx) in [(TangentKind::External, -0.1f64), (TangentKind::Internal, 0.3)] {
            for sy in [1.0, -1.0] {
                let n = Vec2::new(nx, sy * (1.0 - nx * nx).sqrt());
                let far = if kind == TangentKind::External { c2 + n * r2 } else { c2 - n * r2 };
                let found = all.iter().filter(|t| t.kind == kind && ends_at(t, c1 + n * r1, far)).count();
                assert_eq!(found, 1, "{kind:?} {n:?} in {all:?}");
            }
        }
        // Each runs from the first outline to the second.
        assert!(all.iter().all(|t| (t.from - c1).hypot() < (t.to - c1).hypot()));
    }

    #[test]
    fn turned_ellipses_match_an_independent_numeric_solution() {
        let (ma, mb) = (ellipse_map(0.0, 0.0, 40.0, 15.0, 30.0), ellipse_map(150.0, 60.0, 25.0, 60.0, -70.0));
        let all = common_tangents(&Outline::ellipse(ma).unwrap(), &Outline::ellipse(mb).unwrap()).unwrap();
        assert_eq!(all.iter().filter(|t| t.kind == TangentKind::External).count(), 2, "{all:?}");
        assert_eq!(all.iter().filter(|t| t.kind == TangentKind::Internal).count(), 2, "{all:?}");
        let (sa, sb) = (sample_ellipse(ma, 5_000), sample_ellipse(mb, 5_000));
        for t in &all {
            check_against_samples(t, &sa, &sb, 1e-3);
        }
        // A brute-force scan of the normal's angle with support values read off the samples finds
        // the same normals.
        let support = |pts: &[Point], n: Vec2| pts.iter().map(|p| p.to_vec2().dot(n)).fold(f64::MIN, f64::max);
        let scan = |f: &dyn Fn(Vec2) -> f64| {
            let steps = 7_200;
            let g = |k: usize| f(dir(TAU * k as f64 / steps as f64));
            (0..steps).filter(|&k| g(k).signum() != g((k + 1) % steps).signum()).map(|k| TAU * (k as f64 + 0.5) / steps as f64).collect::<Vec<_>>()
        };
        let ext = scan(&|n| support(&sa, n) - support(&sb, n));
        let int = scan(&|n| support(&sa, n) + support(&sb, -n));
        for (kind, expected) in [(TangentKind::External, ext), (TangentKind::Internal, int)] {
            assert_eq!(expected.len(), 2);
            for angle in expected {
                let n = dir(angle);
                let hit = all.iter().filter(|t| t.kind == kind).any(|t| {
                    let d = t.to - t.from;
                    let m = Vec2::new(d.y, -d.x) / d.hypot();
                    m.dot(n).abs() > 1.0 - 1e-6
                });
                assert!(hit, "{kind:?} normal at {angle} in {all:?}");
            }
        }
    }

    #[test]
    fn nested_overlapping_and_touching_circles() {
        // One inside the other: none.
        assert!(common_tangents(&circle(0.0, 0.0, 50.0), &circle(5.0, 0.0, 10.0)).unwrap().is_empty());
        // Overlapping: the two external ones only.
        let over = common_tangents(&circle(0.0, 0.0, 30.0), &circle(40.0, 0.0, 30.0)).unwrap();
        assert_eq!(over.len(), 2);
        assert!(over.iter().all(|t| t.kind == TangentKind::External));
        assert!(over.iter().any(|t| ends_at(t, Point::new(0.0, 30.0), Point::new(40.0, 30.0))));
        // Touching from outside at (10, 0): two external ones and one shared internal one, square to
        // the line of centers, as long as the smaller circle is wide.
        let touch = common_tangents(&circle(0.0, 0.0, 10.0), &circle(30.0, 0.0, 20.0)).unwrap();
        assert_eq!(touch.iter().filter(|t| t.kind == TangentKind::External).count(), 2, "{touch:?}");
        let shared: Vec<_> = touch.iter().filter(|t| t.kind == TangentKind::Internal).collect();
        assert_eq!(shared.len(), 1, "{touch:?}");
        assert!(ends_within(shared[0], Point::new(10.0, -10.0), Point::new(10.0, 10.0), 1e-5), "{shared:?}");
        // Touching along a direction between two sampled normals: the sampled minimum is refined
        // down to the double root.
        let along = dir(0.1234);
        let off = common_tangents(&circle(0.0, 0.0, 10.0), &circle(30.0 * along.x, 30.0 * along.y, 20.0)).unwrap();
        assert_eq!(off.iter().filter(|t| t.kind == TangentKind::Internal).count(), 1, "{off:?}");
        // A hair apart: two internal tangents less than a sample apart, both kept.
        let d = 30.000_01;
        let near_miss = common_tangents(&circle(0.0, 0.0, 10.0), &circle(d * along.x, d * along.y, 20.0)).unwrap();
        assert_eq!(near_miss.iter().filter(|t| t.kind == TangentKind::Internal).count(), 2, "{near_miss:?}");
        // Inside with a clearance of 1e-6: no tangent.
        assert!(common_tangents(&circle(0.0, 0.0, 50.0), &circle(40.0 - 1e-6, 0.0, 10.0)).unwrap().is_empty());
        // Equal circles a hair apart: the two external tangents run between their real contact
        // points, that hair apart, not as a touching tangent.
        let hair = common_tangents(&circle(0.0, 0.0, 30.0), &circle(1e-5, 0.0, 30.0)).unwrap();
        assert_eq!(hair.len(), 2, "{hair:?}");
        assert!(hair.iter().all(|t| ((t.to - t.from).hypot() - 1e-5).abs() < 1e-9), "{hair:?}");
        // Touching from inside at (50, 0): the one shared external tangent.
        let inside = common_tangents(&circle(0.0, 0.0, 50.0), &circle(40.0, 0.0, 10.0)).unwrap();
        assert_eq!(inside.len(), 1, "{inside:?}");
        assert!(ends_within(&inside[0], Point::new(50.0, -10.0), Point::new(50.0, 10.0), 1e-5), "{inside:?}");
    }

    #[test]
    fn a_convex_bezier_blob_and_an_ellipse_path() {
        // A lopsided convex blob of four curves.
        let a = |x: f64, y: f64, ix: f64, iy: f64, ox: f64, oy: f64| {
            craft_geom::Anchor::with_handles(Point::new(x, y), Point::new(ix, iy), Point::new(ox, oy))
        };
        let blob = SubPath::new(
            vec![
                a(0.0, 40.0, 0.0, 70.0, 0.0, 10.0),
                a(50.0, 0.0, 20.0, 0.0, 90.0, 0.0),
                a(110.0, 50.0, 110.0, 20.0, 110.0, 90.0),
                a(40.0, 100.0, 80.0, 100.0, 10.0, 100.0),
            ],
            true,
        );
        let round = shapes::ellipse(Rect::new(200.0, 150.0, 260.0, 230.0)).subpaths[0].clone();
        let (oa, ob) = (Outline::convex(&blob).unwrap(), Outline::convex(&round).unwrap());
        let all = common_tangents(&oa, &ob).unwrap();
        assert_eq!(all.iter().filter(|t| t.kind == TangentKind::External).count(), 2, "{all:?}");
        assert_eq!(all.iter().filter(|t| t.kind == TangentKind::Internal).count(), 2, "{all:?}");
        let (sa, sb) = (sample_path(&blob, 4000), sample_path(&round, 4000));
        for t in &all {
            check_against_samples(t, &sa, &sb, 1e-3);
        }
        // The path's center is its center of area: the ellipse path's is its box's center.
        assert!((ob.center() - Point::new(230.0, 190.0)).hypot() < 1e-6, "{:?}", ob.center());
    }

    #[test]
    fn flat_sides_give_the_whole_stretch_they_touch() {
        let square =
            |x: f64| SubPath::polyline(&[Point::new(x, 0.0), Point::new(x + 10.0, 0.0), Point::new(x + 10.0, 10.0), Point::new(x, 10.0)], true);
        let all = common_tangents(&Outline::convex(&square(0.0)).unwrap(), &Outline::convex(&square(30.0)).unwrap()).unwrap();
        let ext: Vec<_> = all.iter().filter(|t| t.kind == TangentKind::External).collect();
        assert_eq!(ext.len(), 2, "{all:?}");
        assert!(ext.iter().any(|t| ends_at(t, Point::new(0.0, 0.0), Point::new(40.0, 0.0))), "{ext:?}");
        assert!(ext.iter().any(|t| ends_at(t, Point::new(0.0, 10.0), Point::new(40.0, 10.0))), "{ext:?}");
        // Far apart, each diamond is still touched only at its top corner.
        let diamond =
            |x: f64| SubPath::polyline(&[Point::new(x - 5.0, 0.0), Point::new(x, -5.0), Point::new(x + 5.0, 0.0), Point::new(x, 5.0)], true);
        let far = common_tangents(&Outline::convex(&diamond(0.0)).unwrap(), &Outline::convex(&diamond(1e9)).unwrap()).unwrap();
        let top = far.iter().find(|t| t.kind == TangentKind::External && t.from.y > 0.0).unwrap();
        assert!(ends_within(top, Point::new(0.0, 5.0), Point::new(1e9, 5.0), 1e-3), "{far:?}");
        // Far from the origin the center of area and the tangents come out as near it.
        let o = 1e9;
        let away = |x: f64| SubPath::polyline(&[(x, o), (x + 10.0, o), (x + 10.0, o + 10.0), (x, o + 10.0)].map(Point::from), true);
        let (sa, sb) = (Outline::convex(&away(o)).unwrap(), Outline::convex(&away(o + 30.0)).unwrap());
        assert!((sa.center() - Point::new(o + 5.0, o + 5.0)).hypot() < 1e-6, "{:?}", sa.center());
        let shifted = common_tangents(&sa, &sb).unwrap();
        assert!(shifted.iter().any(|t| ends_within(t, Point::new(o, o), Point::new(o + 40.0, o), 1e-6)), "{shifted:?}");
        // The internal ones are the diagonals between the near corners.
        let int: Vec<_> = all.iter().filter(|t| t.kind == TangentKind::Internal).collect();
        assert_eq!(int.len(), 2, "{all:?}");
        assert!(int.iter().any(|t| ends_at(t, Point::new(10.0, 0.0), Point::new(30.0, 10.0))), "{int:?}");
        assert!(int.iter().any(|t| ends_at(t, Point::new(10.0, 10.0), Point::new(30.0, 0.0))), "{int:?}");
        // Squares meeting at the corner (10, 10): every line through the corner between their
        // sides touches both, and the one internal line drawn is the one at the middle of those
        // directions, as long as a square is wide along it.
        let box_at = |x: f64, y: f64| {
            SubPath::polyline(&[Point::new(x, y), Point::new(x + 10.0, y), Point::new(x + 10.0, y + 10.0), Point::new(x, y + 10.0)], true)
        };
        let corner = common_tangents(&Outline::convex(&box_at(0.0, 0.0)).unwrap(), &Outline::convex(&box_at(10.0, 10.0)).unwrap()).unwrap();
        let shared: Vec<_> = corner.iter().filter(|t| t.kind == TangentKind::Internal).collect();
        assert_eq!(shared.len(), 1, "{corner:?}");
        assert!(ends_within(shared[0], Point::new(5.0, 15.0), Point::new(15.0, 5.0), 1e-6), "{shared:?}");
        // Sides 1e-6 off parallel: each line ends at the corners it touches, not at the corners
        // of the sides it nearly lies along.
        let tilted = common_tangents(&Outline::convex(&box_at(0.0, 0.0)).unwrap(), &Outline::convex(&box_at(30.0, -1e-6)).unwrap()).unwrap();
        let ext: Vec<_> = tilted.iter().filter(|t| t.kind == TangentKind::External).collect();
        assert_eq!(ext.len(), 2, "{tilted:?}");
        assert!(ext.iter().any(|t| ends_within(t, Point::new(10.0, 10.0), Point::new(40.0, 10.0 - 1e-6), 1e-6)), "{ext:?}");
        assert!(ext.iter().any(|t| ends_within(t, Point::new(0.0, 0.0), Point::new(30.0, -1e-6), 1e-6)), "{ext:?}");
    }

    /// A circle path of four curves, radius 100 about the origin, with an anchor at (100, 0).
    fn circle_path() -> SubPath {
        shapes::ellipse(Rect::new(-100.0, -100.0, 100.0, 100.0)).subpaths[0].clone()
    }

    /// The point of `sp` where `p · n` is largest.
    fn peak(sp: &SubPath, n: Vec2) -> Point {
        let mut best = (Point::ORIGIN, f64::MIN);
        for k in 0..sp.segment_count() {
            each_extreme(&sp.segment(k), n, |p, v| {
                if v > best.1 {
                    best = (p, v);
                }
            });
        }
        best.0
    }

    #[test]
    fn a_curve_is_touched_only_at_its_peak_beside_an_anchor() {
        // The line touches the circle path 5e-4 radians from its anchor at (100, 0), where the
        // anchor lies about 1.25e-5 inside the line: the line still ends at the peak, not at the
        // anchor 0.05 along it.
        let path = circle_path();
        let n = dir(5e-4);
        let away = Vec2::new(n.y, -n.x) * 300.0;
        let a = Outline::convex(&path).unwrap();
        let all = common_tangents(&a, &a.shifted(away)).unwrap();
        let top = all.iter().find(|t| t.kind == TangentKind::External && (t.from.to_vec2().dot(n) - 100.0).abs() < 1e-3).unwrap();
        let p = peak(&path, n);
        assert!(ends_within(top, p, p + away, 1e-6), "{top:?} {p:?}");
        // An exact circle touching the path there from outside: one shared internal line, as long
        // as that circle is wide, not a stub running to the anchor.
        let b = circle(p.x + 20.0 * n.x, p.y + 20.0 * n.y, 20.0);
        let touch = common_tangents(&a, &b).unwrap();
        let shared: Vec<_> = touch.iter().filter(|t| t.kind == TangentKind::Internal).collect();
        assert_eq!(shared.len(), 1, "{touch:?}");
        assert!(((shared[0].to - shared[0].from).hypot() - 40.0).abs() < 1e-3, "{shared:?}");
        assert!(((shared[0].from.midpoint(shared[0].to)) - p).hypot() < 1e-3, "{shared:?}");
    }

    #[test]
    fn outlines_crossing_more_often_than_the_even_samples_keep_every_tangent() {
        // Two regular 400-gons about one center, one turned half a step: their outlines cross 800
        // times, and an external tangent runs across each crossing.
        let gon = |turn: f64| {
            let pts: Vec<Point> = (0..400).map(|k| (dir(turn + TAU * k as f64 / 400.0) * 100.0).to_point()).collect();
            SubPath::polyline(&pts, true)
        };
        let all = common_tangents(&Outline::convex(&gon(0.0)).unwrap(), &Outline::convex(&gon(TAU / 800.0)).unwrap()).unwrap();
        assert_eq!(all.iter().filter(|t| t.kind == TangentKind::External).count(), 800);
        assert_eq!(all.iter().filter(|t| t.kind == TangentKind::Internal).count(), 0);
        // Each runs between a corner of one and the next corner of the other, about 0.785 apart.
        assert!(all.iter().all(|t| ((t.to - t.from).hypot() - 200.0 * (TAU / 1600.0).sin()).abs() < 1e-6), "{:?}", all.first());
    }

    #[test]
    fn each_tangent_measures_only_the_segments_near_where_it_touches() {
        // Two regular 2,000-gons about one center, one turned half a step: 4,000 external
        // tangents. Measuring every segment of both outlines for every tangent takes about 8,000
        // measurements per tangent.
        let sides = 2_000;
        let gon = |turn: f64| {
            let pts: Vec<Point> = (0..sides).map(|k| (dir(turn + TAU * k as f64 / sides as f64) * 100.0).to_point()).collect();
            SubPath::polyline(&pts, true)
        };
        let (a, b) = (Outline::convex(&gon(0.0)).unwrap(), Outline::convex(&gon(TAU / (2 * sides) as f64)).unwrap());
        EXTREMES.with(|c| c.set(0));
        let all = common_tangents(&a, &b).unwrap();
        let calls = EXTREMES.with(Cell::get);
        assert_eq!(all.iter().filter(|t| t.kind == TangentKind::External).count(), 2 * sides);
        assert!(calls < 1_000 * all.len(), "{calls} measurements for {} tangents", all.len());
        let r = 100.0 * (TAU / (4 * sides) as f64).sin();
        assert!(all.iter().all(|t| ((t.to - t.from).hypot() - 2.0 * r).abs() < 1e-6), "{:?}", all.first());
    }

    #[test]
    fn a_very_thin_ellipse_is_measured_exactly() {
        // Semi-axes 1e6 and 1e-7, a ratio of 1e13, beside a circle, then the whole figure turned
        // 30 degrees: the tangents run from the ellipse's tips to the circle.
        for deg in [0.0, 30.0] {
            let turn = Affine::rotate(f64::to_radians(deg));
            let thin = turn * Affine::scale_non_uniform(1e6, 1e-7);
            let round = turn * Affine::new([10.0, 0.0, 0.0, 10.0, 0.0, 100.0]);
            let all = common_tangents(&Outline::ellipse(thin).unwrap(), &Outline::ellipse(round).unwrap()).unwrap();
            assert_eq!(all.iter().filter(|t| t.kind == TangentKind::External).count(), 2, "{all:?}");
            assert_eq!(all.iter().filter(|t| t.kind == TangentKind::Internal).count(), 2, "{all:?}");
            let (sa, sb) = (sample_ellipse(thin, 20_000), sample_ellipse(round, 5_000));
            for t in &all {
                check_against_samples(t, &sa, &sb, 1e-3);
                // The end on the ellipse, taken back to the unturned figure.
                let (from, to) = (turn.inverse() * t.from, turn.inverse() * t.to);
                let tip = if (from.x.abs() - 1e6).abs() < 1e-3 { from } else { to };
                assert!((tip.x.abs() - 1e6).abs() < 1e-3 && tip.y.abs() < 1e-6, "{deg} {t:?}");
            }
        }
        // A determinant a few times its rounding is taken; one lost in the rounding of the map's
        // entries is still refused.
        assert!(Outline::ellipse(Affine::new([1.0, 0.0, 0.0, 4.0 * f64::EPSILON, 0.0, 0.0])).is_ok());
        assert_eq!(Outline::ellipse(Affine::new([1.0, 0.0, 0.0, f64::EPSILON, 0.0, 0.0])).err(), Some(OutlineError::Empty));
        assert_eq!(Outline::ellipse(Affine::new([1.0, 1.0, 1.0, 1.0 + f64::EPSILON, 0.0, 0.0])).err(), Some(OutlineError::Empty));
    }

    #[test]
    fn a_curve_peaking_inside_a_segment_is_measured_at_its_peak() {
        // p(t) · n has the control values 0, 0.25, 0.5, -0.25: its derivative's linear term is
        // zero, and the peak, 0.25 at t = 0.5, lies inside the segment.
        let s = CubicBez::new((0.0, 0.0), (1.0, 0.25), (2.0, 0.5), (3.0, -0.25));
        let mut best = f64::MIN;
        each_extreme(&s, Vec2::new(0.0, 1.0), |_, v| best = best.max(v));
        assert!((best - 0.25).abs() < 1e-12, "{best}");
    }

    #[test]
    fn a_point_outside_a_turned_ellipse_has_two_tangents_through_it() {
        let m = ellipse_map(100.0, 50.0, 40.0, 15.0, 30.0);
        let (oval, p) = (Outline::ellipse(m).unwrap(), Point::new(-60.0, 120.0));
        let samples = sample_ellipse(m, 5_000);
        for point_first in [true, false] {
            let point = Outline::point(p).unwrap();
            let all = if point_first { common_tangents(&point, &oval) } else { common_tangents(&oval, &point) }.unwrap();
            assert_eq!(all.len(), 2, "{all:?}");
            for t in &all {
                assert_eq!(t.kind, TangentKind::External);
                // Each runs from the first outline given to the second: from the point when it
                // comes first.
                let (at_point, on_oval) = if point_first { (t.from, t.to) } else { (t.to, t.from) };
                assert!((at_point - p).hypot() < 1e-9, "{t:?}");
                // Taken back to the unit circle, the far end lies on it and the line runs square to
                // its radius there.
                let inv = m.inverse();
                let (q, d) = ((inv * on_oval).to_vec2(), (inv * on_oval) - (inv * at_point));
                assert!((q.hypot() - 1.0).abs() < 1e-9, "{q:?}");
                assert!((q.dot(d) / d.hypot()).abs() < 1e-7, "{q:?} {d:?}");
                // The whole ellipse lies on one side of the line.
                let dir = on_oval - at_point;
                let n = Vec2::new(-dir.y, dir.x) / dir.hypot();
                let side: Vec<f64> = samples.iter().map(|s| (*s - at_point).dot(n)).collect();
                let (lo, hi) = side.iter().fold((f64::MAX, f64::MIN), |(lo, hi), v| (lo.min(*v), hi.max(*v)));
                assert!(lo > -1e-6 || hi < 1e-6, "crosses: {lo} .. {hi}");
            }
        }
    }

    #[test]
    fn a_point_on_inside_and_beside_a_path() {
        // On a circle: the one tangent there, centered on the point and as long as the circle is
        // wide.
        let on = common_tangents(&Outline::point(Point::new(10.0, 0.0)).unwrap(), &circle(0.0, 0.0, 10.0)).unwrap();
        assert_eq!(on.len(), 1, "{on:?}");
        assert!(ends_within(&on[0], Point::new(10.0, -10.0), Point::new(10.0, 10.0), 1e-5), "{on:?}");
        // Inside, and two points: none.
        assert!(common_tangents(&Outline::point(Point::new(3.0, 4.0)).unwrap(), &circle(0.0, 0.0, 10.0)).unwrap().is_empty());
        let two = (Outline::point(Point::new(0.0, 0.0)).unwrap(), Outline::point(Point::new(5.0, 0.0)).unwrap());
        assert!(common_tangents(&two.0, &two.1).unwrap().is_empty());
        // Beside a square, in line with its bottom side: that tangent covers the whole side, and
        // the other one touches the corner at (10, 10).
        let square = SubPath::polyline(&[(0.0, 0.0), (10.0, 0.0), (10.0, 10.0), (0.0, 10.0)].map(Point::from), true);
        let sq = Outline::convex(&square).unwrap();
        let p = Point::new(20.0, 0.0);
        let all = common_tangents(&Outline::point(p).unwrap(), &sq).unwrap();
        assert_eq!(all.len(), 2, "{all:?}");
        assert!(all.iter().any(|t| ends_at(t, p, Point::new(0.0, 0.0))), "{all:?}");
        assert!(all.iter().any(|t| ends_at(t, p, Point::new(10.0, 10.0))), "{all:?}");
        assert!(all.iter().all(|t| (t.from - p).hypot() < 1e-9), "from the point: {all:?}");
        // On a straight side, off its middle: the one tangent is that side, covered whole.
        let side = common_tangents(&Outline::point(Point::new(2.0, 0.0)).unwrap(), &sq).unwrap();
        assert_eq!(side.len(), 1, "{side:?}");
        assert!(ends_at(&side[0], Point::new(0.0, 0.0), Point::new(10.0, 0.0)), "{side:?}");
        // At a corner of the square: one line through the corner, at the middle of the directions
        // that touch there, as long as the square is wide along it.
        let corner = common_tangents(&Outline::point(Point::new(10.0, 10.0)).unwrap(), &sq).unwrap();
        assert_eq!(corner.len(), 1, "{corner:?}");
        // The diagonal is 10√2 long, so each end lies 5√2 from the corner, along (1, -1).
        assert!(ends_within(&corner[0], Point::new(15.0, 5.0), Point::new(5.0, 15.0), 1e-5), "{corner:?}");
        // A point that is not finite is refused.
        assert_eq!(Outline::point(Point::new(f64::NAN, 0.0)).err(), Some(OutlineError::Empty));
        assert_eq!(Outline::point(Point::new(0.0, f64::INFINITY)).err(), Some(OutlineError::Empty));
    }

    #[test]
    fn a_point_placed_on_a_turned_outline_has_one_tangent() {
        // A point worked out on the outline lies a rounding error inside or outside it, and still
        // gets the one line there, as long as the outline is wide along it.
        let one = |p: Point, o: &Outline, samples: &[Point]| {
            let all = common_tangents(&Outline::point(p).unwrap(), o).unwrap();
            assert_eq!(all.len(), 1, "{p:?}: {all:?}");
            // The line runs through the point.
            let (a, d) = (all[0].from, all[0].to - all[0].from);
            assert!(d.hypot() > 1.0 && ((p - a).cross(d) / d.hypot()).abs() < 1e-6, "{p:?}: {all:?}");
            // Its length is the outline's width along it, measured on dense samples.
            let u = d / d.hypot();
            let (lo, hi) = samples.iter().map(|s| s.to_vec2().dot(u)).fold((f64::MAX, f64::MIN), |(lo, hi), v| (lo.min(v), hi.max(v)));
            assert!((d.hypot() - (hi - lo)).abs() < 1e-3, "{p:?}: {} against {}", d.hypot(), hi - lo);
        };
        let m = ellipse_map(100.0, 50.0, 40.0, 15.0, 30.0);
        let (oval, oval_samples) = (Outline::ellipse(m).unwrap(), sample_ellipse(m, 20_000));
        for k in 0..37 {
            let a = 0.17 + k as f64 * TAU / 37.0;
            one(m * Point::new(a.cos(), a.sin()), &oval, &oval_samples);
        }
        // A closed Bézier circle, turned and moved: on each anchor and partway along each segment.
        let turned = shapes::ellipse(Rect::new(-60.0, -60.0, 60.0, 60.0)).transformed(Affine::translate((37.3, -12.9)) * Affine::rotate(0.7));
        let sp = turned.subpaths[0].clone();
        let (path, path_samples) = (Outline::convex(&sp).unwrap(), sample_path(&sp, 5_000));
        for an in sp.anchors.iter() {
            one(an.p, &path, &path_samples);
        }
        for k in 0..sp.segment_count() {
            for t in [0.21, 0.5, 0.83] {
                one(sp.segment(k).eval(t), &path, &path_samples);
            }
        }
    }

    #[test]
    fn a_point_within_the_touching_tolerance_counts_as_on_the_outline() {
        // The tolerance is a billionth of the larger of the circle's size and the point's distance
        // from its center: closer than that, inside or outside, the point gets the one line there.
        let round = circle(0.0, 0.0, 10.0);
        let tol = 1e-9 * round.size().max(10.0);
        let at = |dx: f64| common_tangents(&Outline::point(Point::new(10.0 + dx, 0.0)).unwrap(), &round).unwrap();
        for dx in [-0.5 * tol, 0.5 * tol] {
            let on = at(dx);
            assert_eq!(on.len(), 1, "{dx}: {on:?}");
            assert!(ends_within(&on[0], Point::new(10.0, -10.0), Point::new(10.0, 10.0), 1e-5), "{dx}: {on:?}");
        }
        // Farther than that, a point inside gets none and a point outside gets two.
        assert!(at(-100.0 * tol).is_empty());
        assert_eq!(at(100.0 * tol).len(), 2);
        assert!(at(-1e-6).is_empty());
        assert_eq!(at(1e-6).len(), 2);
    }

    #[test]
    fn open_concave_empty_and_identical_outlines_are_refused() {
        let open = SubPath::polyline(&[Point::new(0.0, 0.0), Point::new(10.0, 0.0), Point::new(10.0, 10.0)], false);
        assert_eq!(Outline::convex(&open).err(), Some(OutlineError::Open));
        let star = shapes::star(Point::new(0.0, 0.0), 50.0, 20.0, 5, 0.0).subpaths[0].clone();
        assert_eq!(Outline::convex(&star).err(), Some(OutlineError::NotConvex));
        // A half disk whose straight side is a cubic with crossed handles: it bends the other way
        // for a short stretch between two of the points the outline is checked at.
        let mut bp = craft_geom::BezPath::new();
        let k = 0.5523 * 50.0;
        bp.move_to((0.0, 0.0));
        bp.curve_to((62.4, 43.4), (60.3, 44.2), (100.0, 0.0));
        bp.curve_to((100.0, -k), (50.0 + k, -50.0), (50.0, -50.0));
        bp.curve_to((50.0 - k, -50.0), (0.0, -k), (0.0, 0.0));
        bp.close_path();
        let bend = craft_geom::PathData::from_bezpath(&bp).subpaths[0].clone();
        assert_eq!(Outline::convex(&bend).err(), Some(OutlineError::NotConvex));
        // A triangle whose first side is a cubic that leaves its start a little the wrong way: the
        // corner there turns inward, though the checked points and the curve itself turn one way.
        let mut bp = craft_geom::BezPath::new();
        bp.move_to((0.0, 0.0));
        bp.curve_to((-1.0, -1.0), (66.0, -2.0), (100.0, 0.0));
        bp.line_to((50.0, 100.0));
        bp.close_path();
        let notch = craft_geom::PathData::from_bezpath(&bp).subpaths[0].clone();
        assert_eq!(Outline::convex(&notch).err(), Some(OutlineError::NotConvex));
        // A square traced twice around winds around twice.
        let twice: Vec<Point> = [(0.0, 0.0), (10.0, 0.0), (10.0, 10.0), (0.0, 10.0)].repeat(2).into_iter().map(Point::from).collect();
        assert_eq!(Outline::convex(&SubPath::polyline(&twice, true)).err(), Some(OutlineError::NotConvex));
        let flat = SubPath::polyline(&[Point::new(0.0, 0.0), Point::new(10.0, 0.0), Point::new(20.0, 0.0)], true);
        assert_eq!(Outline::convex(&flat).err(), Some(OutlineError::Empty));
        assert_eq!(Outline::ellipse(Affine::scale_non_uniform(10.0, 0.0)).err(), Some(OutlineError::Empty));
        assert_eq!(Outline::ellipse(Affine::new([f64::NAN, 0.0, 0.0, 1.0, 0.0, 0.0])).err(), Some(OutlineError::Empty));
        let many: Vec<Point> = (0..=MAX_TANGENT_SEGMENTS).map(|k| dir(TAU * k as f64 / (MAX_TANGENT_SEGMENTS + 1) as f64).to_point()).collect();
        assert_eq!(Outline::convex(&SubPath::polyline(&many, true)).err(), Some(OutlineError::TooComplex));
        // A square 1e110 wide has a finite center.
        let huge = SubPath::polyline(&[(0.0, 0.0), (1e110, 0.0), (1e110, 1e110), (0.0, 1e110)].map(Point::from), true);
        let center = Outline::convex(&huge).unwrap().center();
        assert!((center - Point::new(5e109, 5e109)).hypot() < 1e98, "{center:?}");
        assert_eq!(common_tangents(&circle(1.0, 2.0, 3.0), &circle(1.0, 2.0, 3.0)), Err(Coincide));
    }
}
