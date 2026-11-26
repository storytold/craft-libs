//! [`MotionPath`]: constant-speed motion along a chain of cubic Bezier segments.

use crate::{EaseError, Vec2, bez, dbez};

/// Most segments one path holds.
pub const MAX_PATH_SEGMENTS: usize = 10_000;
/// Cells in each segment's arc-length table.
const ARC_N: usize = 32;
/// Most steps `Segment::param_at` takes; halving a table cell this often reaches the smallest
/// step an `f64` parameter can take.
const PARAM_STEPS: u32 = 64;

/// 5-point Gauss-Legendre nodes and weights on −1..1.
const GAUSS: [(f64, f64); 5] = [
    (0.0, 0.568_888_888_888_888_9),
    (-0.538_469_310_105_683_1, 0.478_628_670_499_366_5),
    (0.538_469_310_105_683_1, 0.478_628_670_499_366_5),
    (-0.906_179_845_938_664, 0.236_926_885_056_189_1),
    (0.906_179_845_938_664, 0.236_926_885_056_189_1),
];

#[derive(Clone, Debug, PartialEq)]
struct Segment {
    p: [Vec2; 4],
    /// Arc length from the segment's start at `u = i / ARC_N`.
    table: [f64; ARC_N + 1],
    /// Both handles on the chord: the segment is the straight line between its ends.
    straight: bool,
    /// The parameters inside `0..1` where the x or y derivative is zero, in order, with the
    /// unused entries set to 1. Arc lengths are integrated in pieces split there, since the
    /// speed has a kink where the curve stops and turns back.
    stops: [f64; 4],
    /// Arc length from the path's start to this segment's start.
    start: f64,
}

impl Segment {
    fn new(p: [Vec2; 4], start: f64) -> Segment {
        let straight = on_chord(p[0], p[1], p[3]) && on_chord(p[0], p[2], p[3]);
        let stops = stationary(&p);
        let mut table = [0.0; ARC_N + 1];
        if straight {
            let len = p[0].distance(p[3]);
            for (i, v) in table.iter_mut().enumerate() {
                *v = len * (i as f64 / ARC_N as f64);
            }
        } else {
            let mut acc = 0.0;
            for i in 1..=ARC_N {
                acc += arc(&p, &stops, (i - 1) as f64 / ARC_N as f64, i as f64 / ARC_N as f64);
                if let Some(v) = table.get_mut(i) {
                    *v = acc;
                }
            }
        }
        Segment { p, table, straight, stops, start }
    }

    fn length(&self) -> f64 {
        self.table[ARC_N]
    }

    fn point(&self, u: f64) -> Vec2 {
        let p = &self.p;
        Vec2::new(bez(u, [p[0].x, p[1].x, p[2].x, p[3].x]), bez(u, [p[0].y, p[1].y, p[2].y, p[3].y]))
    }

    /// The parameter at arc length `s` from the segment's start (`0..=length`): the table
    /// cell's straight-line guess refined with Newton steps on the true arc length, kept inside
    /// a shrinking bracket and halving it when a step would leave it (where the curve stands
    /// still, for one), until the arc length is within 1e-12 of the segment's length of `s`, in
    /// at most [`PARAM_STEPS`] steps.
    fn param_at(&self, s: f64) -> f64 {
        let len = self.length();
        if len <= 0.0 {
            return 0.0;
        }
        let s = s.clamp(0.0, len);
        if self.straight {
            return s / len;
        }
        let i = self.table.partition_point(|&v| v < s).clamp(1, ARC_N);
        let (Some(&a), Some(&b)) = (self.table.get(i - 1), self.table.get(i)) else { return s / len };
        let (u0, u1) = ((i - 1) as f64 / ARC_N as f64, i as f64 / ARC_N as f64);
        let mut u = u0 + if b > a { (s - a) / (b - a) } else { 0.0 } * (u1 - u0);
        let (mut lo, mut hi) = (u0, u1);
        for _ in 0..PARAM_STEPS {
            let e = a + arc(&self.p, &self.stops, u0, u) - s;
            if e.abs() <= 1e-12 * len {
                break;
            }
            if e < 0.0 {
                lo = u;
            } else {
                hi = u;
            }
            let d = speed(&self.p, u);
            let next = if d > 0.0 { u - e / d } else { f64::NAN };
            u = if next > lo && next < hi { next } else { 0.5 * (lo + hi) };
        }
        u
    }

    /// The direction of travel at `u`, or the chord's where the curve's derivative vanishes.
    fn direction(&self, u: f64) -> Vec2 {
        let p = &self.p;
        let d = Vec2::new(dbez(u, [p[0].x, p[1].x, p[2].x, p[3].x]), dbez(u, [p[0].y, p[1].y, p[2].y, p[3].y])).normalized();
        if d != Vec2::ZERO {
            return d;
        }
        // At an end whose handle sits on it, the curve leaves toward the next control point.
        let toward = if u < 0.5 {
            [p[1], p[2], p[3]].into_iter().find(|q| *q != p[0]).map(|q| q - p[0])
        } else {
            [p[2], p[1], p[0]].into_iter().find(|q| *q != p[3]).map(|q| p[3] - q)
        };
        toward.unwrap_or(Vec2::ZERO).normalized()
    }
}

/// |dB/du|.
fn speed(p: &[Vec2; 4], u: f64) -> f64 {
    Vec2::new(dbez(u, [p[0].x, p[1].x, p[2].x, p[3].x]), dbez(u, [p[0].y, p[1].y, p[2].y, p[3].y])).length()
}

/// Arc length between parameters `a` and `b` (Gauss-Legendre; exact enough on one table cell),
/// in pieces split at the parameters `stops` that lie between them.
fn arc(p: &[Vec2; 4], stops: &[f64; 4], a: f64, b: f64) -> f64 {
    let mut from = a;
    let mut total = 0.0;
    for &r in stops.iter().filter(|&&r| r > a && r < b) {
        total += gauss(p, from, r);
        from = r;
    }
    total + gauss(p, from, b)
}

/// [`arc`] over one piece. Each term is scaled by the interval before they are added, so a
/// length near the largest `f64` does not overflow on the way.
fn gauss(p: &[Vec2; 4], a: f64, b: f64) -> f64 {
    let (m, h) = ((a + b) * 0.5, (b - a) * 0.5);
    GAUSS.iter().map(|(x, w)| w * h * speed(p, m + h * x)).sum::<f64>()
}

/// The parameters inside `0..1` where the x or y derivative of the segment `p` is zero, sorted
/// and padded with 1 (see `Segment::stops`).
fn stationary(p: &[Vec2; 4]) -> [f64; 4] {
    let mut out = [1.0; 4];
    let mut n = 0;
    for k in [[p[0].x, p[1].x, p[2].x, p[3].x], [p[0].y, p[1].y, p[2].y, p[3].y]] {
        for r in quadratic_roots(k[1] - k[0], k[2] - k[1], k[3] - k[2]) {
            if let Some(slot) = out.get_mut(n) {
                *slot = r;
                n += 1;
            }
        }
    }
    out.sort_by(f64::total_cmp);
    out
}

/// The roots inside `0..1` of the quadratic with Bernstein coefficients `k0`, `k1` and `k2`
/// (a cubic's derivative, up to a factor of 3). The coefficients are scaled to the largest
/// first, so coordinates near the ends of the `f64` range do not overflow when squared.
fn quadratic_roots(k0: f64, k1: f64, k2: f64) -> impl Iterator<Item = f64> {
    let m = k0.abs().max(k1.abs()).max(k2.abs());
    let (k0, k1, k2) = if m.is_finite() && m > 0.0 { (k0 / m, k1 / m, k2 / m) } else { (0.0, 0.0, 0.0) };
    // a u² + b u + c in powers of u.
    let (a, b, c) = (k0 - 2.0 * k1 + k2, 2.0 * (k1 - k0), k0);
    let roots = if a.abs() < 1e-12 {
        [if b != 0.0 { -c / b } else { f64::NAN }, f64::NAN]
    } else {
        let disc = b * b - 4.0 * a * c;
        if disc < 0.0 {
            [f64::NAN, f64::NAN]
        } else {
            // The root of larger size first, then the other from their product, without
            // cancellation.
            let q = -0.5 * (b + disc.sqrt().copysign(b));
            [q / a, if q != 0.0 { c / q } else { f64::NAN }]
        }
    };
    roots.into_iter().filter(|r| *r > 0.0 && *r < 1.0)
}

/// Whether control point `c` lies on the segment `a`–`b` (within rounding).
fn on_chord(a: Vec2, c: Vec2, b: Vec2) -> bool {
    let (mut d, mut v) = (b - a, c - a);
    // Scaled to the largest component first, so squaring coordinates near 1e154 and above does
    // not overflow and make a curve look straight.
    let scale = d.x.abs().max(d.y.abs()).max(v.x.abs()).max(v.y.abs());
    if scale.is_finite() && scale > 0.0 {
        (d, v) = (Vec2::new(d.x / scale, d.y / scale), Vec2::new(v.x / scale, v.y / scale));
    }
    let dd = d.dot(d);
    if dd <= 0.0 {
        // A segment of zero length is straight when its handles sit on its point.
        return v == Vec2::ZERO;
    }
    let t = v.dot(d) / dd;
    let off = v - d * t;
    (0.0..=1.0).contains(&t) && off.dot(off) <= dd * 1e-28
}

/// A path of cubic Bezier segments with an arc-length table per segment, built once, so a point
/// at a distance along it costs a binary search and a few curve evaluations and does not
/// allocate.
///
/// A distance before the start or past the end continues in a straight line along the path's
/// direction there, so an ease that overshoots carries the motion past the end the same way on
/// curved and straight paths.
#[derive(Clone, Debug, PartialEq)]
pub struct MotionPath {
    segments: Vec<Segment>,
    length: f64,
    /// The first and last segments that have length (the first and last when none has), so the
    /// ends' directions skip anchors repeated at an end.
    ends: (usize, usize),
}

impl MotionPath {
    /// A path from the absolute points `[p0, c1, c2, p1, c3, c4, p2, …]` (3n + 1 of them, n from
    /// 1 to [`MAX_PATH_SEGMENTS`]). An error for a wrong count, a point that is not finite, or
    /// a path too long to measure.
    pub fn new(points: &[Vec2]) -> Result<MotionPath, EaseError> {
        if points.len() < 4 {
            return Err(EaseError::TooFew { what: "a motion path", need: 4 });
        }
        if points.len() % 3 != 1 {
            return Err(EaseError::OutOfRange { what: "the point count (3n + 1)", min: 4.0, max: (3 * MAX_PATH_SEGMENTS + 1) as f64 });
        }
        if points.len() > 3 * MAX_PATH_SEGMENTS + 1 {
            return Err(EaseError::Full(MAX_PATH_SEGMENTS));
        }
        if points.iter().any(|p| !p.is_finite()) {
            return Err(EaseError::NotFinite("path point"));
        }
        let mut segments = Vec::with_capacity(points.len() / 3);
        let mut start = 0.0;
        for w in points.windows(4).step_by(3) {
            let [a, b, c, d] = w else { continue };
            let s = Segment::new([*a, *b, *c, *d], start);
            start += s.length();
            segments.push(s);
        }
        if !start.is_finite() {
            return Err(EaseError::Overflow);
        }
        let first = segments.iter().position(|g| g.length() > 0.0).unwrap_or(0);
        let last = segments.iter().rposition(|g| g.length() > 0.0).unwrap_or(segments.len().saturating_sub(1));
        Ok(MotionPath { segments, length: start, ends: (first, last) })
    }

    /// A smooth path through `anchors` (at least 2), its handles set automatically: each inner
    /// anchor's handles point along the line from the anchor before it to the anchor after it,
    /// a sixth of that line long, and an end anchor's handle points a sixth of the way toward
    /// its neighbor. A side that faces an anchor at the same place has no handle, so two equal
    /// anchors make a segment of zero length.
    pub fn through(anchors: &[Vec2]) -> Result<MotionPath, EaseError> {
        if anchors.len() < 2 {
            return Err(EaseError::TooFew { what: "a motion path", need: 2 });
        }
        if anchors.len() > MAX_PATH_SEGMENTS + 1 {
            return Err(EaseError::Full(MAX_PATH_SEGMENTS));
        }
        if anchors.iter().any(|p| !p.is_finite()) {
            return Err(EaseError::NotFinite("path point"));
        }
        let n = anchors.len();
        let at = |i: usize| anchors.get(i).copied().unwrap_or(Vec2::ZERO);
        let handles = |i: usize| -> (Vec2, Vec2) {
            let (a, p, c) = (at(i.saturating_sub(1)), at(i), at((i + 1).min(n - 1)));
            let out = (c - a) * (1.0 / 6.0);
            let h_in = if i > 0 && a == p { Vec2::ZERO } else { -out };
            let h_out = if i + 1 < n && c == p { Vec2::ZERO } else { out };
            (h_in, h_out)
        };
        let mut points = Vec::with_capacity(3 * n - 2);
        for i in 0..n {
            let (h_in, h_out) = handles(i);
            if i > 0 {
                points.push(at(i) + h_in);
            }
            points.push(at(i));
            if i + 1 < n {
                points.push(at(i) + h_out);
            }
        }
        MotionPath::new(&points)
    }

    /// The path's length.
    pub fn length(&self) -> f64 {
        self.length
    }

    /// How many segments the path has.
    pub fn segments(&self) -> usize {
        self.segments.len()
    }

    /// The segment holding distance `s` (clamped to the path) and the distance into it.
    fn locate(&self, s: f64) -> Option<(&Segment, f64)> {
        let i = self.segments.partition_point(|g| g.start <= s).saturating_sub(1);
        let g = self.segments.get(i)?;
        Some((g, s - g.start))
    }

    /// The segments at the ends (see the `ends` field).
    fn ends(&self) -> Option<(&Segment, &Segment)> {
        Some((self.segments.get(self.ends.0)?, self.segments.get(self.ends.1)?))
    }

    /// The point `s` along the path. Before the start and past the end it continues along the
    /// path's direction there; NaN and −∞ give the start, and +∞ the end.
    pub fn point_at_length(&self, s: f64) -> Vec2 {
        let Some((first, last)) = self.ends() else { return Vec2::ZERO };
        if s.is_nan() || s <= 0.0 {
            let s = if s.is_finite() { s } else { 0.0 };
            return first.p[0] + first.direction(0.0) * s;
        }
        if s >= self.length {
            let past = if s.is_finite() { s - self.length } else { 0.0 };
            return last.p[3] + last.direction(1.0) * past;
        }
        match self.locate(s) {
            Some((g, into)) if g.straight => {
                let len = g.length();
                if len > 0.0 { g.p[0].lerp(g.p[3], into / len) } else { g.p[0] }
            }
            Some((g, into)) => g.point(g.param_at(into)),
            None => first.p[0],
        }
    }

    /// The point `progress` of the way along the path (0 the start, 1 the end).
    pub fn point_at(&self, progress: f64) -> Vec2 {
        self.point_at_length(progress * self.length)
    }

    /// The unit direction of travel `s` along the path (zero for a path of zero length).
    pub fn direction_at_length(&self, s: f64) -> Vec2 {
        let Some((first, last)) = self.ends() else { return Vec2::ZERO };
        if s.is_nan() || s <= 0.0 {
            return first.direction(0.0);
        }
        if s >= self.length {
            return last.direction(1.0);
        }
        match self.locate(s) {
            Some((g, into)) => g.direction(g.param_at(into)),
            None => Vec2::ZERO,
        }
    }
}
