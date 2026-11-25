//! Least-squares cubic Bézier fitting of point sequences (the classic "fit a cubic with fixed end
//! tangents, then reparameterise with Newton steps, then split near the worst point" approach from
//! Graphics Gems, written from the published description).

use kurbo::{CubicBez, Line, ParamCurve, ParamCurveDeriv, ParamCurveNearest, Point, Vec2};

/// Maximum Newton reparameterisation rounds per fit.
const MAX_REPARAM: usize = 6;

/// Most pieces joined to the first piece of a fitted curve. Each join refits every point of the
/// curve, so this limits how many times a point is refitted.
const MAX_JOINS: usize = 32;

/// Most samples of a fitted curve checked between two neighboring points. See [`bulge`].
const MAX_BULGE_SAMPLES: f64 = 1024.0;

/// About how many points of a long piece its first fit uses.
const COARSE_POINTS: usize = 16;

/// Most times [`fit_through_rec`] splits a run of points, which bounds its recursion.
const MAX_SPLITS: usize = 40;

/// Unit vector or `None` if (nearly) zero.
pub(crate) fn unit(v: Vec2) -> Option<Vec2> {
    let l = v.hypot();
    if l > 1e-12 && l.is_finite() { Some(v / l) } else { None }
}

/// Direction the curve leaves `p0` (robust to retracted handles).
pub(crate) fn start_tangent(c: &CubicBez) -> Vec2 {
    unit(c.p1 - c.p0).or_else(|| unit(c.p2 - c.p0)).or_else(|| unit(c.p3 - c.p0)).unwrap_or(Vec2::new(1.0, 0.0))
}

/// Direction the curve arrives at `p3`, i.e. pointing *forward* along the travel direction.
pub(crate) fn end_tangent(c: &CubicBez) -> Vec2 {
    unit(c.p3 - c.p2).or_else(|| unit(c.p3 - c.p1)).or_else(|| unit(c.p3 - c.p0)).unwrap_or(Vec2::new(1.0, 0.0))
}

/// Fit a chain of cubics through `pts` with maximum (parametric) error `tol`.
///
/// `t0` is the unit tangent leaving `pts[0]`; `t1` is the unit tangent arriving at the last point
/// (forward direction). Consecutive output cubics are G1 continuous.
pub fn fit_cubics(pts: &[Point], t0: Vec2, t1: Vec2, tol: f64) -> Vec<CubicBez> {
    let pts = dedup(pts);
    if pts.len() < 2 {
        return Vec::new();
    }
    let tol = tol.max(1e-9);
    let mut pieces = Vec::new();
    fit_rec(&pts, 0, pts.len() - 1, t0, t1, tol, &mut pieces);
    join_pieces(&pts, &pieces, t0, tol)
}

/// Fit a chain of cubics through `pts`, sampled from a smooth curve, with maximum error `tol` at
/// the points.
///
/// Unlike [`fit_cubics`], a piece may leave the chords between the points: the points lie on the
/// curve being fitted, so the piece bows with them (each end keeps its tangent, and a two-point
/// piece gets handles a third of its chord long). Use this for dense samples of a known curve, and
/// [`fit_cubics`] for the samples of a polyline. `t0` and `t1` are as in [`fit_cubics`].
pub fn fit_cubics_through(pts: &[Point], t0: Vec2, t1: Vec2, tol: f64) -> Vec<CubicBez> {
    let pts = dedup(pts);
    let mut out = Vec::new();
    if pts.len() < 2 {
        return out;
    }
    fit_through_rec(&pts, t0, t1, tol.max(1e-9), &mut out, 0);
    out
}

/// Fits all of `pts` with one cubic, splitting at its worst point until each piece is within `tol`
/// of its points (see [`fit_cubics_through`]). `depth` bounds the recursion on hostile points.
fn fit_through_rec(pts: &[Point], t0: Vec2, t1: Vec2, tol: f64, out: &mut Vec<CubicBez>, depth: usize) {
    let n = pts.len();
    if n == 2 {
        let d = pts[0].distance(pts[1]) / 3.0;
        out.push(CubicBez::new(pts[0], pts[0] + t0 * d, pts[1] - t1 * d, pts[1]));
        return;
    }
    let (curve, err, split) = fit_single_tol(pts, t0, t1, tol);
    if err <= tol || depth >= MAX_SPLITS {
        out.push(curve);
        return;
    }
    let split = split.clamp(1, n - 2);
    let tc = unit(pts[split + 1] - pts[split - 1]).unwrap_or(t0);
    fit_through_rec(&pts[..=split], t0, tc, tol, out, depth + 1);
    fit_through_rec(&pts[split..], tc, t1, tol, out, depth + 1);
}

/// The curves of `pieces`, which follow each other along `pts` from its first point, leaving it
/// along `t0`. Each curve takes up to [`MAX_JOINS`] of the pieces after its first one, as long as
/// the fit of them together is within `tol`.
fn join_pieces(pts: &[Point], pieces: &[Piece], t0: Vec2, tol: f64) -> Vec<CubicBez> {
    let mut out = Vec::new();
    let (mut start, mut leaving, mut next) = (0, t0, 0);
    while let Some(first) = pieces.get(next) {
        let (mut curve, mut end, mut arriving) = (first.curve, first.end, first.t1);
        next += 1;
        for piece in pieces.iter().skip(next).take(MAX_JOINS) {
            let Ok(joined) = fit_piece(&pts[start..=piece.end], leaving, piece.t1, tol) else { break };
            (curve, end, arriving) = (joined, piece.end, piece.t1);
            next += 1;
        }
        out.push(curve);
        (start, leaving) = (end, arriving);
    }
    out
}

/// A fitted piece: its curve, the index of the point it ends on and its tangent there.
struct Piece {
    curve: CubicBez,
    end: usize,
    t1: Vec2,
}

fn dedup(pts: &[Point]) -> Vec<Point> {
    let mut v: Vec<Point> = Vec::with_capacity(pts.len());
    for &p in pts {
        if v.last().is_none_or(|q: &Point| q.distance(p) > 1e-12) {
            v.push(p);
        }
    }
    v
}

/// Fits `all[start..=end]`, splitting a piece in two until each piece is within `tol`. A piece of
/// two points is a cubic through both with handles along the tangents, at most `tol` long, so it
/// stays within `tol` of the line between them.
///
/// The split point is the piece's worst point moved, when it is near an end, to two fifths of the
/// piece from that end. Each half then holds about three fifths of the points at most, so the
/// recursion depth is logarithmic in the number of points.
fn fit_rec(all: &[Point], start: usize, end: usize, t0: Vec2, t1: Vec2, tol: f64, out: &mut Vec<Piece>) {
    let pts = &all[start..=end];
    let n = pts.len();
    if n == 2 {
        let d = (pts[0].distance(pts[1]) / 3.0).min(tol);
        out.push(Piece { curve: CubicBez::new(pts[0], pts[0] + t0 * d, pts[1] - t1 * d, pts[1]), end, t1 });
        return;
    }
    let split = match fit_piece(pts, t0, t1, tol) {
        Ok(curve) => {
            out.push(Piece { curve, end, t1 });
            return;
        }
        Err(worst) => worst,
    };
    let margin = (n * 2 / 5).max(1);
    let split = split.clamp(margin, n - 1 - margin);
    let tc = center_tangent(pts, split, tol).unwrap_or(t0);
    fit_rec(all, start, start + split, t0, tc, tol, out);
    fit_rec(all, start + split, end, tc, t1, tol, out);
}

/// One cubic for three or more `pts`, when every point is within `tol` of it and it is within `tol`
/// of the lines between the points; otherwise the index of a point where it is not. A first fit over
/// 8 times `tol` plus 5% of the chord is not reparameterized.
///
/// A piece of many points is first fitted to about [`COARSE_POINTS`] of them, and it is split when
/// that fit is over `tol` at one of those. Each split can be undone by [`join_pieces`].
fn fit_piece(pts: &[Point], t0: Vec2, t1: Vec2, tol: f64) -> Result<CubicBez, usize> {
    let give_up = tol * 8.0 + pts[0].distance(pts[pts.len() - 1]) * 0.05;
    if pts.len() >= COARSE_POINTS * 2 {
        let stride = pts.len() / COARSE_POINTS;
        let some: Vec<Point> = pts.iter().step_by(stride).chain(pts.last()).copied().collect();
        let (_, err, worst, _) = fit_single_within(&some, chord_params(&some), t0, t1, give_up);
        if err > tol {
            return Err((worst * stride).min(pts.len() - 1));
        }
    }
    let (c, err, worst, u) = fit_single_within(pts, chord_params(pts), t0, t1, give_up);
    if err > tol {
        return Err(worst);
    }
    bulge(pts, &c, &u, tol).map_or(Ok(c), Err)
}

/// Direction at `pts[split]`, between the nearest points on each side that are over `tol` from it,
/// or the first and last points when none is, so that details smaller than `tol` do not turn it.
fn center_tangent(pts: &[Point], split: usize, tol: f64) -> Option<Vec2> {
    let p = pts[split];
    let before = pts[..split].iter().rev().find(|q| q.distance(p) > tol).unwrap_or(&pts[0]);
    let after = pts[split + 1..].iter().find(|q| q.distance(p) > tol).unwrap_or(&pts[pts.len() - 1]);
    unit(*after - *before)
}

/// Best single cubic for `pts` with the given end tangents. Returns (curve, max error, index of
/// the worst point).
pub(crate) fn fit_single(pts: &[Point], t0: Vec2, t1: Vec2) -> (CubicBez, f64, usize) {
    fit_single_from(pts, chord_params(pts), t0, t1)
}

/// [`fit_single`] that stops reparameterising once the curve is within `tol` of its points: a
/// curve already that close needs no better fit, and each Newton round reads every point.
fn fit_single_tol(pts: &[Point], t0: Vec2, t1: Vec2, tol: f64) -> (CubicBez, f64, usize) {
    let mut u = chord_params(pts);
    let mut best = generate(pts, &u, t0, t1);
    let (mut best_err, mut best_split) = max_error(pts, &best, &u);
    for _ in 0..MAX_REPARAM {
        if best_err <= tol {
            break;
        }
        for (i, ui) in u.iter_mut().enumerate() {
            *ui = newton(&best, pts[i], *ui);
        }
        let c = generate(pts, &u, t0, t1);
        let (e, s) = max_error(pts, &c, &u);
        if e < best_err {
            best = c;
            best_err = e;
            best_split = s;
        } else {
            break;
        }
    }
    (best, best_err, best_split)
}

/// [`fit_single`] starting from the parameters `u` (one per point, 0 to 1) instead of the
/// chord-length ones.
pub(crate) fn fit_single_from(pts: &[Point], u: Vec<f64>, t0: Vec2, t1: Vec2) -> (CubicBez, f64, usize) {
    let (c, err, worst, _) = fit_single_within(pts, u, t0, t1, f64::INFINITY);
    (c, err, worst)
}

/// [`fit_single_from`] that also returns the fit's parameters, and returns the first fit without
/// any reparameterization when its error is over `give_up`.
fn fit_single_within(pts: &[Point], mut u: Vec<f64>, t0: Vec2, t1: Vec2, give_up: f64) -> (CubicBez, f64, usize, Vec<f64>) {
    let mut best = generate(pts, &u, t0, t1);
    let (mut best_err, mut best_split) = max_error(pts, &best, &u);
    if best_err > give_up {
        return (best, best_err, best_split, u);
    }
    let mut best_u = u.clone();
    for _ in 0..MAX_REPARAM {
        for (i, ui) in u.iter_mut().enumerate() {
            *ui = newton(&best, pts[i], *ui);
        }
        let c = generate(pts, &u, t0, t1);
        let (e, s) = max_error(pts, &c, &u);
        if e < best_err {
            best = c;
            best_err = e;
            best_split = s;
            best_u.copy_from_slice(&u);
        } else {
            break;
        }
    }
    (best, best_err, best_split, best_u)
}

/// Index of the first point of `pts` after which the curve `c`, between that point's parameter in
/// `u` and the next point's, is over 99% of `tol` from the line between the two points at a sample.
///
/// Over a parameter step `h`, a curve stays within `h * h * m / 8` of the chord between its ends,
/// where `m` is its largest second derivative, at most 6 times the control points' largest second
/// difference. The samples are close enough to keep that to 1% of `tol`. A pair of points that would
/// need more than [`MAX_BULGE_SAMPLES`] counts as a point where the curve is too far.
fn bulge(pts: &[Point], c: &CubicBez, u: &[f64], tol: f64) -> Option<usize> {
    let step = (0.01 * tol * 8.0 / (6.0 * largest_second_difference(c))).sqrt();
    let limit = tol * 0.99;
    pts.windows(2).zip(u.windows(2)).position(|(p, t)| {
        let line = Line::new(p[0], p[1]);
        let samples = ((t[1] - t[0]).abs() / step).ceil().max(1.0);
        if samples > MAX_BULGE_SAMPLES {
            return true;
        }
        let samples = samples as usize;
        (1..=samples).any(|k| {
            let at = t[0] + (t[1] - t[0]) * k as f64 / (samples + 1) as f64;
            line.nearest(c.eval(at), 1e-9).distance_sq > limit * limit
        })
    })
}

/// The larger second difference of the control points of `c`. Its largest second derivative is 6
/// times this.
pub(crate) fn largest_second_difference(c: &CubicBez) -> f64 {
    (c.p2 - c.p1 - (c.p1 - c.p0)).hypot().max((c.p3 - c.p2 - (c.p2 - c.p1)).hypot())
}

fn chord_params(pts: &[Point]) -> Vec<f64> {
    let mut u = Vec::with_capacity(pts.len());
    let mut acc = 0.0;
    u.push(0.0);
    for w in pts.windows(2) {
        acc += w[0].distance(w[1]);
        u.push(acc);
    }
    if acc > 0.0 {
        for x in &mut u {
            *x /= acc;
        }
    }
    u
}

fn bern(t: f64) -> [f64; 4] {
    let s = 1.0 - t;
    [s * s * s, 3.0 * s * s * t, 3.0 * s * t * t, t * t * t]
}

/// Solve the 2x2 normal equations for the handle lengths along the fixed tangents.
fn generate(pts: &[Point], u: &[f64], t0: Vec2, t1: Vec2) -> CubicBez {
    let (Some(&p0), Some(&p3)) = (pts.first(), pts.last()) else { return CubicBez::new(Point::ZERO, Point::ZERO, Point::ZERO, Point::ZERO) };
    let dir_in = -t1; // handle at the end points backwards
    let (mut c00, mut c01, mut c11, mut x0, mut x1) = (0.0, 0.0, 0.0, 0.0, 0.0);
    for (p, &t) in pts.iter().zip(u) {
        let b = bern(t);
        let a0 = t0 * b[1];
        let a1 = dir_in * b[2];
        c00 += a0.dot(a0);
        c01 += a0.dot(a1);
        c11 += a1.dot(a1);
        let tmp = *p - (p0.to_vec2() * (b[0] + b[1]) + p3.to_vec2() * (b[2] + b[3])).to_point();
        x0 += a0.dot(tmp);
        x1 += a1.dot(tmp);
    }
    let det = c00 * c11 - c01 * c01;
    let seg = p0.distance(p3);
    let eps = 1e-6 * seg;
    let (mut al, mut ar) = if det.abs() > 1e-12 { ((x0 * c11 - x1 * c01) / det, (c00 * x1 - c01 * x0) / det) } else { (seg / 3.0, seg / 3.0) };
    if !al.is_finite() || !ar.is_finite() || al < eps || ar < eps {
        al = seg / 3.0;
        ar = seg / 3.0;
    }
    // Guard against wild handles on nearly-degenerate data.
    let cap = seg * 4.0 + 1e-9;
    al = al.min(cap);
    ar = ar.min(cap);
    CubicBez::new(p0, p0 + t0 * al, p3 + dir_in * ar, p3)
}

fn max_error(pts: &[Point], c: &CubicBez, u: &[f64]) -> (f64, usize) {
    let mut e = 0.0;
    let mut idx = pts.len() / 2;
    for (i, (p, &t)) in pts.iter().zip(u).enumerate() {
        let d = c.eval(t).distance(*p);
        if d > e {
            e = d;
            idx = i;
        }
    }
    (e, idx)
}

fn newton(c: &CubicBez, p: Point, t: f64) -> f64 {
    let d1 = c.deriv();
    let d2 = d1.deriv();
    let q = c.eval(t) - p;
    let q1 = d1.eval(t).to_vec2();
    let q2 = d2.eval(t).to_vec2();
    let num = q.dot(q1);
    let den = q1.dot(q1) + q.dot(q2);
    if den.abs() < 1e-12 {
        return t;
    }
    let nt = t - num / den;
    if nt.is_finite() { nt.clamp(0.0, 1.0) } else { t }
}

/// Is the cubic (within `tol`) a straight line from p0 to p3 with handles inside the chord?
pub(crate) fn is_straight(c: &CubicBez, tol: f64) -> bool {
    let chord = c.p3 - c.p0;
    let len = chord.hypot();
    if len < 1e-12 {
        return c.p1.distance(c.p0) <= tol && c.p2.distance(c.p0) <= tol;
    }
    let dir = chord / len;
    [c.p1, c.p2].iter().all(|&h| {
        let v = h - c.p0;
        let along = v.dot(dir);
        v.cross(dir).abs() <= tol && along >= -tol && along <= len + tol
    })
}

/// Sample a cubic at `n+1` evenly spaced parameters (including both ends).
pub(crate) fn sample(c: &CubicBez, n: usize, out: &mut Vec<Point>, include_start: bool) {
    let start = if include_start { 0 } else { 1 };
    for i in start..=n {
        out.push(c.eval(i as f64 / n as f64));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use craft_geom::{Anchor, PathData, SubPath};

    #[test]
    fn fits_a_cubic_exactly() {
        let c = CubicBez::new((0.0, 0.0), (30.0, 60.0), (70.0, 60.0), (100.0, 0.0));
        let mut pts = Vec::new();
        sample(&c, 40, &mut pts, true);
        let out = fit_cubics(&pts, start_tangent(&c), end_tangent(&c), 0.05);
        assert_eq!(out.len(), 1);
        assert!(out[0].p1.distance(c.p1) < 1.0, "{:?}", out[0]);
    }

    /// A curve's samples are followed between the points, not chopped to the chords through
    /// them: a smooth outline fitted from coarse samples must keep its bow (an outline's area
    /// depends on it), unlike a polyline's samples.
    #[test]
    fn through_fit_bows_between_coarse_points() {
        // The parabola y = 4t(1 - t) over x = 100t, sampled at 8 points.
        let node = |t: f64| Point::new(100.0 * t, 4.0 * t * (1.0 - t));
        let pts: Vec<Point> = (0..=8).map(|i| node(i as f64 / 8.0)).collect();
        let (t0, t1) = (Vec2::new(1.0, 0.04).normalize(), Vec2::new(1.0, -0.04).normalize());
        let out = fit_cubics_through(&pts, t0, t1, 0.001);
        for c in &out {
            for k in 0..=100 {
                let q = c.eval(k as f64 / 100.0);
                let want = 4.0 * (q.x / 100.0) * (1.0 - q.x / 100.0);
                assert!((q.y - want).abs() < 0.01, "{:?} at {q:?} wants {want}", c);
            }
        }
    }

    #[test]
    fn splits_when_needed() {
        // A zig-zag cannot be fitted with one cubic.
        let pts: Vec<Point> = (0..=40).map(|i| Point::new(i as f64, ((i as f64) * 0.5).sin() * 10.0)).collect();
        let out = fit_cubics(&pts, Vec2::new(1.0, 0.0), Vec2::new(1.0, 0.0), 0.1);
        assert!(out.len() > 1);
        for w in out.windows(2) {
            assert!(w[0].p3.distance(w[1].p0) < 1e-9);
        }
    }

    /// Numbers from 0 to 1, the same ones for the same `seed`.
    fn random(mut seed: u64) -> impl FnMut() -> f64 {
        move || {
            seed = seed.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1_442_695_040_888_963_407);
            (seed >> 11) as f64 / (1u64 << 53) as f64
        }
    }

    /// Largest distance from a point of `pts` to the nearest of the cubics `cs`, or infinity when
    /// a point is more than `reach` from the bounding box of every cubic.
    fn farthest(pts: &[Point], cs: &[CubicBez], reach: f64) -> f64 {
        use kurbo::ParamCurveExtrema;
        let boxes: Vec<_> = cs.iter().map(|c| c.bounding_box().inflate(reach, reach)).collect();
        pts.iter()
            .map(|&p| {
                cs.iter().zip(&boxes).filter(|(_, b)| b.contains(p)).map(|(c, _)| c.nearest(p, 1e-9).distance_sq.sqrt()).fold(f64::INFINITY, f64::min)
            })
            .fold(0.0, f64::max)
    }

    fn cubics(path: &PathData) -> Vec<CubicBez> {
        path.subpaths.iter().flat_map(|s| (0..s.segment_count()).map(|i| s.segment(i))).collect()
    }

    fn samples(cs: &[CubicBez]) -> Vec<Point> {
        let mut pts = Vec::new();
        for c in cs {
            sample(c, 16, &mut pts, true);
        }
        pts
    }

    /// Samples of `cs` that follow each curve to within 1% of `tol`, at least 16 per curve.
    fn close_samples(cs: &[CubicBez], tol: f64) -> Vec<Point> {
        let mut pts = Vec::new();
        for c in cs {
            let steps = (0.75 * largest_second_difference(c) / (0.01 * tol)).sqrt().ceil().clamp(16.0, 20000.0);
            sample(c, steps as usize, &mut pts, true);
        }
        pts
    }

    /// Simplify of `path` has no more anchors, and each of the path and its result is within `tol`
    /// of the other, measured on the curves themselves. Returns the result.
    fn assert_simplifies_within(path: &PathData, tol: f64, name: &str) -> PathData {
        let out = crate::simplify_with(path, &crate::SimplifyOptions { tolerance: tol, ..Default::default() });
        assert!(out.anchor_count() <= path.anchor_count(), "{name}: {} anchors from {}", out.anchor_count(), path.anchor_count());
        assert_within(&cubics(path), &cubics(&out), tol, name);
        out
    }

    /// Each of `before` and `after` is within `tol` of the other, measured on the curves themselves.
    fn assert_within(before: &[CubicBez], after: &[CubicBez], tol: f64, name: &str) {
        let limit = tol * (1.0 + 1e-6);
        let to_result = farthest(&close_samples(before, tol), after, limit * 8.0);
        assert!(to_result <= limit, "{name}: a point of the path is {to_result} from the result at a tolerance of {tol}");
        let to_path = farthest(&close_samples(after, tol), before, limit * 8.0);
        assert!(to_path <= limit, "{name}: a point of the result is {to_path} from the path at a tolerance of {tol}");
    }

    /// The segments of `sp` as a run for `simplify_run_within`.
    fn run_of(sp: &SubPath) -> Vec<crate::boolean::Seg> {
        (0..sp.segment_count()).map(|i| crate::boolean::Seg { c: sp.segment(i), line: false }).collect()
    }

    /// Anchors along a circle of radius `r` about the origin, from `first` degrees in steps of
    /// `step` degrees, smooth, with the handles of a circular arc.
    fn arc_anchors(r: f64, first: f64, step: f64, count: usize) -> Vec<Anchor> {
        let handle = r * 4.0 / 3.0 * (step.to_radians() / 4.0).tan();
        (0..count)
            .map(|i| {
                let (s, c) = (first + i as f64 * step).to_radians().sin_cos();
                let p = Point::new(r * c, r * s);
                Anchor::smooth(p, p + Vec2::new(-s, c) * handle)
            })
            .collect()
    }

    /// `count` corner anchors 0.5 pt apart, going on from `from` along `dir`.
    fn short_anchors(from: Point, dir: Vec2, count: usize) -> Vec<Anchor> {
        (1..=count).map(|i| Anchor::corner(from + dir * (i as f64 * 0.5))).collect()
    }

    #[test]
    fn long_arcs_stay_within_the_tolerance() {
        // Simplify keeps a subpath as it was when the refit has more anchors, so this measures the
        // refit itself.
        for (r, tol) in [(1_000_000.0, 2.0), (1_000_000.0, 1.0), (100_000.0, 0.1), (100_000.0, 1.0 / 32.0), (20_000.0, 0.01), (5_000.0, 0.01)] {
            let run = run_of(&SubPath::new(arc_anchors(r, 0.0, 90.0, 4), false));
            let opts = crate::SimplifyOptions { tolerance: tol, ..Default::default() };
            let out = crate::edit::simplify_run_within(&run, &opts, usize::MAX, None);
            let (before, after): (Vec<_>, Vec<_>) = (run.iter().map(|s| s.c).collect(), out.iter().map(|s| s.c).collect());
            assert_within(&before, &after, tol, &format!("arcs of {r} pt"));
            assert!(after.len() < 200, "arcs of {r} pt at {tol} pt give {} curves", after.len());
        }
    }

    #[test]
    fn long_arcs_and_short_anchors_stay_within_the_tolerance() {
        for (r, tol) in [(100_000.0, 1.0), (100_000.0, 0.1), (100_000.0, 1.0 / 32.0), (20_000.0, 0.01), (5_000.0, 0.01), (1_000_000.0, 1.0)] {
            let mut anchors = arc_anchors(r, 0.0, 90.0, 4);
            let end = anchors[3].p;
            anchors.extend(short_anchors(end, Vec2::new(1.0, 0.0), 3000));
            let out = assert_simplifies_within(&PathData::single(SubPath::new(anchors, false)), tol, &format!("arcs of {r} pt and short anchors"));
            assert!(out.anchor_count() < 200, "arcs of {r} pt at {tol} pt give {} anchors", out.anchor_count());
        }
    }

    #[test]
    fn long_arcs_converted_to_straight_lines_stay_within_the_tolerance() {
        for (r, tol) in [(100_000.0, 0.1), (20_000.0, 0.01)] {
            let mut anchors = arc_anchors(r, 0.0, 90.0, 4);
            let end = anchors[3].p;
            anchors.extend(short_anchors(end, Vec2::new(1.0, 0.0), 3000));
            let path = PathData::single(SubPath::new(anchors, false));
            let out = crate::simplify_with(&path, &crate::SimplifyOptions { tolerance: tol, straight_lines: true, ..Default::default() });
            assert_within(&cubics(&path), &cubics(&out), tol, &format!("lines along arcs of {r} pt"));
        }
    }

    #[test]
    fn a_path_that_turns_back_past_its_end_converted_to_straight_lines_keeps_its_tip() {
        // A line to (100, 0), a smooth 0.2 pt U-turn and a line back to (50, 0.2): every point is
        // within 0.4 pt of the line through the ends, but the tip is 50 pt past the end.
        let k = 0.2 * 2.0 / 3.0;
        let (a, b) = (Point::new(100.0, 0.0), Point::new(100.0, 0.2));
        let anchors = vec![
            Anchor::corner(Point::new(0.0, 0.0)),
            Anchor::with_handles(a, a, Point::new(100.0 + k, 0.0)),
            Anchor::with_handles(b, Point::new(100.0 + k, 0.2), b),
            Anchor::corner(Point::new(50.0, 0.2)),
        ];
        let path = PathData::single(SubPath::new(anchors, false));
        let out = crate::simplify_with(&path, &crate::SimplifyOptions { tolerance: 1.0, straight_lines: true, ..Default::default() });
        assert_within(&cubics(&path), &cubics(&out), 1.0, "a U-turn past the end");
    }

    #[test]
    fn long_and_short_segments_stay_within_the_tolerance() {
        for (r, tol) in [(50_000.0, 2.0), (50_000.0, 1.0 / 32.0), (8_000.0, 0.01), (500.0, 0.5)] {
            let mut anchors = Vec::new();
            for block in 0..6 {
                let first = f64::from(block) * 60.0;
                let arc = arc_anchors(r, first, 20.0, 3);
                let (s, c) = (first + 40.0).to_radians().sin_cos();
                let end = arc[2].p;
                anchors.extend(arc);
                anchors.extend(short_anchors(end, Vec2::new(-s, c), 300));
            }
            assert_simplifies_within(&PathData::single(SubPath::new(anchors, false)), tol, &format!("radius {r}"));
        }
    }

    #[test]
    fn a_segment_that_needs_too_many_samples_stays_as_it_is() {
        // Radius 10,000,000 pt at 0.001 pt needs about 770,000 steps per quarter circle.
        let mut anchors = arc_anchors(10_000_000.0, 0.0, 90.0, 4);
        let end = anchors[3].p;
        anchors.extend(short_anchors(end, Vec2::new(1.0, 0.0), 500));
        let path = PathData::single(SubPath::new(anchors, false));
        let out = crate::simplify_with(&path, &crate::SimplifyOptions { tolerance: 0.001, ..Default::default() });
        assert_eq!(cubics(&out)[..3], cubics(&path)[..3]);
        assert!(out.anchor_count() < 20, "{} anchors", out.anchor_count());
    }

    #[test]
    fn a_segment_that_needs_too_many_samples_becomes_lines_when_asked_for_lines() {
        // About 55,000 lines keep this arc within 0.001 pt, and about 770,000 samples would be needed
        // to fit it.
        let (r, tol) = (10_000_000.0, 0.001);
        let path = PathData::single(SubPath::new(arc_anchors(r, 0.0, 90.0, 2), false));
        let arc = cubics(&path)[0];
        let out = cubics(&crate::simplify_with(&path, &crate::SimplifyOptions { tolerance: tol, straight_lines: true, ..Default::default() }));
        assert!(out.iter().all(|c| is_straight(c, 1e-6)));
        for (k, line) in out.iter().enumerate() {
            let middle = arc.eval((k as f64 + 0.5) / out.len() as f64);
            assert!(middle.distance(line.p0.midpoint(line.p3)) <= tol, "line {k} of {}", out.len());
        }
    }

    #[test]
    fn lines_for_segments_that_need_too_many_samples_are_limited() {
        // Each of these 35 segments takes the most lines a segment can have.
        let run = run_of(&SubPath::new(arc_anchors(1e9, 0.0, 10.0, 36), false));
        let opts = crate::SimplifyOptions { tolerance: 1e-3, straight_lines: true, ..Default::default() };
        let out = crate::edit::simplify_run_within(&run, &opts, usize::MAX, None);
        assert!(out.len() <= crate::edit::MAX_RUN_RESULT + run.len(), "{} lines", out.len());
        assert!(out.iter().all(|s| s.line));
        assert_eq!(out[out.len() - 1].c.p3, run[run.len() - 1].c.p3);
    }

    #[test]
    fn a_run_past_the_samples_of_one_fit_has_an_anchor_where_the_parts_meet() {
        let (r, tol) = (1_000_000.0, 0.0005);
        let run = run_of(&SubPath::new(arc_anchors(r, 0.0, 15.0, 24), false));
        let steps = crate::edit::sample_steps(&run[0].c, tol);
        let segments = crate::edit::MAX_FIT_SAMPLES / steps;
        assert!(segments + 1 < run.len(), "{steps} steps per segment");
        let opts = crate::SimplifyOptions { tolerance: tol, ..Default::default() };
        let out = crate::edit::simplify_run_within(&run, &opts, usize::MAX, None);
        let meet = run[segments - 1].c.p3;
        assert!(out.iter().any(|s| s.c.p3 == meet), "no anchor at {meet:?}");
    }

    #[test]
    fn segments_past_the_sample_budget_stay_as_they_are() {
        // Every segment takes the same steps, so the budget covers the first ten.
        let sp = SubPath::new(arc_anchors(100.0, 0.0, 15.0, 20), false);
        let run = run_of(&sp);
        let opts = crate::SimplifyOptions { tolerance: 1.0, ..Default::default() };
        let steps = crate::edit::sample_steps(&run[0].c, 1.0);
        let out = crate::edit::simplify_run_within(&run, &opts, steps * 10, None);
        assert!(out.len() < 19, "{} segments", out.len());
        let tail: Vec<CubicBez> = out[out.len() - 9..].iter().map(|s| s.c).collect();
        let want: Vec<CubicBez> = run[10..].iter().map(|s| s.c).collect();
        assert_eq!(tail, want);
    }

    #[test]
    fn a_curve_that_needs_too_many_checks_is_rejected() {
        // The first curve goes along the line and back. The second one goes along it evenly.
        let pts = [Point::new(0.0, 0.0), Point::new(1e9, 0.0)];
        let c = CubicBez::new(pts[0], pts[1], pts[0], pts[1]);
        assert_eq!(bulge(&pts, &c, &[0.0, 1.0], 1e-3), Some(0));
        let c = CubicBez::new(pts[0], Point::new(1e9 / 3.0, 0.0), Point::new(2e9 / 3.0, 0.0), pts[1]);
        assert_eq!(bulge(&pts, &c, &[0.0, 1.0], 1e-3), None);
    }

    /// `rows` rows of `cols` smooth anchors 1.6 pt apart, alternately 2 pt up.
    fn serpentine(rows: usize, cols: usize, closed: bool) -> SubPath {
        let mut anchors = Vec::with_capacity(rows * cols);
        for row in 0..rows {
            let dir = if row % 2 == 0 { 1.0 } else { -1.0 };
            for col in 0..cols {
                let x = if row % 2 == 0 { col } else { cols - 1 - col };
                let p = Point::new(x as f64 * 1.6, row as f64 * 6.0 + if col % 2 == 0 { 0.0 } else { 2.0 });
                anchors.push(Anchor::smooth(p, p + Vec2::new(0.5 * dir, 0.0)));
            }
        }
        SubPath::new(anchors, closed)
    }

    /// `n` straight segments of `step` pt that turn by `degrees` left and right in turn.
    fn zigzag(n: usize, degrees: f64, step: f64, closed: bool) -> SubPath {
        let half = (degrees / 2.0).to_radians();
        let mut p = Point::ZERO;
        let mut pts = Vec::with_capacity(n);
        for i in 0..n {
            pts.push(p);
            let a = if i % 2 == 0 { half } else { -half };
            p += Vec2::new(a.cos(), a.sin()) * step;
        }
        SubPath::polyline(&pts, closed)
    }

    /// Smooth anchors along arcs that bend one way and then the other, 40 anchors per arc.
    fn alternating_arcs(arcs: usize) -> SubPath {
        let mut anchors = Vec::with_capacity(arcs * 40);
        let (mut p, mut heading) = (Point::ZERO, 0.0_f64);
        for arc in 0..arcs {
            for _ in 0..40 {
                heading += if arc % 2 == 0 { 0.08 } else { -0.08 };
                let d = Vec2::new(heading.cos(), heading.sin()) * 2.0;
                anchors.push(Anchor::smooth(p, p + d / 3.0));
                p += d;
            }
        }
        SubPath::new(anchors, false)
    }

    /// A polyline of random steps, with some points repeated when `repeats` is set.
    fn random_walk(rnd: &mut impl FnMut() -> f64, n: usize, repeats: bool, closed: bool) -> SubPath {
        let mut p = Point::new(rnd() * 100.0, rnd() * 100.0);
        let mut pts = Vec::with_capacity(n);
        for _ in 0..n {
            pts.push(p);
            if repeats && rnd() < 0.3 {
                pts.push(p);
            }
            p += Vec2::new(rnd() * 6.0 - 3.0, rnd() * 6.0 - 3.0);
        }
        SubPath::polyline(&pts, closed)
    }

    /// A chain of smooth anchors with random steps and handles.
    fn random_curves(rnd: &mut impl FnMut() -> f64, n: usize, closed: bool) -> SubPath {
        let mut p = Point::ZERO;
        let mut anchors = Vec::with_capacity(n);
        for _ in 0..n {
            anchors.push(Anchor::smooth(p, p + Vec2::new(1.0 + rnd() * 6.0, rnd() * 8.0 - 4.0)));
            p += Vec2::new(5.0 + rnd() * 20.0, rnd() * 30.0 - 15.0);
        }
        SubPath::new(anchors, closed)
    }

    /// `n` smooth anchors 1.6 pt apart, each at a random height of up to 3 pt.
    fn random_ripple(rnd: &mut impl FnMut() -> f64, n: usize, closed: bool) -> SubPath {
        let anchors = (0..n)
            .map(|i| {
                let p = Point::new(i as f64 * 1.6, rnd() * 3.0);
                Anchor::smooth(p, p + Vec2::new(0.5, 0.0))
            })
            .collect();
        SubPath::new(anchors, closed)
    }

    #[test]
    fn no_fitted_piece_is_farther_than_the_tolerance() {
        for (rows, cols) in [(1, 300), (1, 1000), (5, 400)] {
            assert_simplifies_within(&PathData::single(serpentine(rows, cols, false)), 1.0, &format!("serpentine {rows} x {cols}"));
        }
        assert_simplifies_within(&PathData::single(serpentine(1, 1000, false)), 0.25, "serpentine at 0.25 pt");
        assert_simplifies_within(&PathData::single(alternating_arcs(25)), 0.5, "alternating arcs");
    }

    #[test]
    fn a_zigzag_stays_within_the_tolerance() {
        // The turns are below the 30 degree corner threshold, so the fit runs across them.
        assert_simplifies_within(&PathData::single(zigzag(1500, 20.0, 3.0, false)), 0.5, "zigzag");
        assert_simplifies_within(&PathData::single(zigzag(1500, 29.0, 1.0, false)), 0.1, "fine zigzag");
        // These turns are corners, so every segment is a run of its own.
        assert_simplifies_within(&PathData::single(zigzag(500, 60.0, 3.0, false)), 2.0, "zigzag of corners");
        // Simplify samples a segment this long more than `tol` apart, so a curve between two
        // samples can leave the path without a sample showing it.
        assert_simplifies_within(&PathData::single(zigzag(60, 25.0, 900.0, false)), 1.0, "zigzag of long segments");
        assert_simplifies_within(&PathData::single(zigzag(60, 8.0, 2500.0, true)), 0.5, "closed zigzag of long segments");
        // The short anchors make the result smaller than the path, so Simplify does not return the
        // path as it was.
        for (step, tol) in [(1000.0, 0.1), (5000.0, 1.0), (20000.0, 1.0)] {
            let mut sp = zigzag(40, 25.0, step, false);
            let end = sp.anchors[sp.anchors.len() - 1].p;
            sp.anchors.extend((1..=2000).map(|i| Anchor::corner(end + Vec2::new(i as f64 * 0.5, 0.0))));
            assert_simplifies_within(&PathData::single(sp), tol, &format!("zigzag of {step} pt segments and short anchors"));
        }
    }

    #[test]
    fn closed_paths_stay_within_the_tolerance() {
        assert_simplifies_within(&PathData::single(serpentine(3, 1000, true)), 1.0, "closed serpentine");
        assert_simplifies_within(&PathData::single(zigzag(800, 20.0, 3.0, true)), 0.5, "closed zigzag");
        let circle: Vec<Anchor> = (0..2000)
            .map(|i| {
                let a = i as f64 / 2000.0 * std::f64::consts::TAU;
                let p = Point::new(300.0 * a.cos(), 300.0 * a.sin());
                Anchor::smooth(p, p + Vec2::new(-a.sin(), a.cos()) * 0.3)
            })
            .collect();
        assert_simplifies_within(&PathData::single(SubPath::new(circle, true)), 0.05, "closed circle of 2,000 anchors");
    }

    #[test]
    fn compound_paths_stay_within_the_tolerance() {
        let mut rnd = random(11);
        let compound = PathData::new(vec![
            serpentine(1, 1000, false),
            zigzag(400, 20.0, 3.0, true),
            random_curves(&mut rnd, 100, false),
            SubPath::polyline(&[Point::new(0.0, -50.0), Point::new(100.0, -50.0)], false),
        ]);
        assert_simplifies_within(&compound, 0.75, "compound");
    }

    #[test]
    fn coincident_anchors_stay_within_the_tolerance() {
        let mut rnd = random(23);
        assert_simplifies_within(&PathData::single(random_walk(&mut rnd, 600, true, false)), 1.0, "repeated points");
        assert_simplifies_within(&PathData::single(random_walk(&mut rnd, 600, true, true)), 1.0, "repeated points, closed");
        let p = Point::new(5.0, 5.0);
        let same = SubPath::new(vec![Anchor::smooth(p, p + Vec2::new(1.0, 0.0)); 50], false);
        assert_simplifies_within(&PathData::single(same), 0.5, "one place");
        let mut doubled = serpentine(1, 1500, false);
        let copy = doubled.anchors.clone();
        doubled.anchors = copy.iter().flat_map(|a| [*a, *a]).collect();
        assert_simplifies_within(&PathData::single(doubled), 1.0, "every anchor twice");
    }

    #[test]
    fn random_paths_stay_within_the_tolerance() {
        let mut rnd = random(0x9e37_79b9_7f4a_7c15);
        for case in 0..60 {
            let tol = [0.05, 0.25, 1.0, 4.0][case % 4];
            let n = 2 + (rnd() * 300.0) as usize;
            let closed = case % 3 == 0;
            let sp = if case % 2 == 0 { random_walk(&mut rnd, n, false, closed) } else { random_curves(&mut rnd, n, closed) };
            assert_simplifies_within(&PathData::single(sp), tol, &format!("case {case}"));
        }
        for (case, tol) in [0.5, 1.0, 2.0].into_iter().enumerate() {
            assert_simplifies_within(&PathData::single(random_ripple(&mut rnd, 2000, case == 1)), tol, &format!("ripple at {tol} pt"));
        }
    }

    #[test]
    fn a_curve_stays_within_the_tolerance_between_wide_points() {
        let up = Vec2::new(0.6, 0.8);
        let down = Vec2::new(0.6, -0.8);
        let runs: [&[Point]; 3] = [
            // Two points: the tangents alone shape the curve.
            &[Point::new(0.0, 0.0), Point::new(600.0, 0.0)],
            // Three points: a curve through the middle one can still bulge on each side of it.
            &[Point::new(0.0, 0.0), Point::new(300.0, 0.0), Point::new(600.0, 0.0)],
            &[Point::new(0.0, 0.0), Point::new(100.0, 20.0), Point::new(200.0, 0.0), Point::new(300.0, 20.0), Point::new(400.0, 0.0)],
        ];
        for pts in runs {
            let out = fit_cubics(pts, up, down, 1.0);
            let far = samples(&out)
                .iter()
                .map(|&q| pts.windows(2).map(|w| Line::new(w[0], w[1]).nearest(q, 1e-9).distance_sq.sqrt()).fold(f64::INFINITY, f64::min))
                .fold(0.0, f64::max);
            assert!(far <= 1.0 + 1e-6, "a point of the curve is {far} from the {} points", pts.len());
        }
    }

    #[test]
    fn a_curve_stays_within_the_tolerance_between_its_checks() {
        // A curve with handles this long has its peak between evenly spaced samples.
        let pts = [Point::new(0.0, 0.0), Point::new(300.0, 0.0), Point::new(600.0, 0.0)];
        let tol = 105.47;
        let mut dense = Vec::new();
        for c in fit_cubics(&pts, Vec2::new(0.6, 0.8), Vec2::new(0.8, -0.6), tol) {
            sample(&c, 4096, &mut dense, true);
        }
        let far = dense.iter().map(|q| q.y.abs()).fold(0.0, f64::max);
        assert!(far <= tol, "a point of the curve is {far} from the line");
    }

    #[test]
    fn every_point_is_within_the_tolerance_of_its_chain() {
        let mut rnd = random(0x2545_f491_4f6c_dd1d);
        for case in 0..400 {
            let tol = [0.01, 0.1, 1.0, 5.0][case % 4];
            let mut pts: Vec<Point> = Vec::new();
            match case % 5 {
                // Open random walk.
                0 => {
                    let mut p = Point::new(rnd() * 100.0, rnd() * 100.0);
                    for _ in 0..2 + (rnd() * 400.0) as usize {
                        pts.push(p);
                        p += Vec2::new(rnd() * 4.0 - 1.0, rnd() * 4.0 - 2.0);
                    }
                }
                // Closed noisy loop: the last point is the first.
                1 => {
                    let n = 3 + (rnd() * 300.0) as usize;
                    for i in 0..n {
                        let a = i as f64 / n as f64 * std::f64::consts::TAU;
                        let r = 50.0 + rnd() * 3.0;
                        pts.push(Point::new(r * a.cos(), r * a.sin()));
                    }
                    pts.push(pts[0]);
                }
                // Very short runs.
                2 => {
                    for _ in 0..2 + (rnd() * 3.0) as usize {
                        pts.push(Point::new(rnd() * 20.0, rnd() * 20.0));
                    }
                }
                // Coincident points.
                3 => {
                    let mut p = Point::new(0.0, 0.0);
                    for _ in 0..2 + (rnd() * 200.0) as usize {
                        pts.push(p);
                        if rnd() < 0.3 {
                            pts.push(p);
                        }
                        p += Vec2::new(rnd() * 3.0, rnd() * 6.0 - 3.0);
                    }
                }
                // Samples of a chain of wavy cubics, as Simplify takes them.
                _ => {
                    let mut p0 = Point::new(0.0, 0.0);
                    for k in 0..1 + (rnd() * 60.0) as usize {
                        let p3 = p0 + Vec2::new(5.0 + rnd() * 10.0, rnd() * 10.0 - 5.0);
                        let c = CubicBez::new(p0, p0 + Vec2::new(2.0, rnd() * 8.0 - 4.0), p3 - Vec2::new(2.0, rnd() * 8.0 - 4.0), p3);
                        sample(&c, 4 + (rnd() * 20.0) as usize, &mut pts, k == 0);
                        p0 = p3;
                    }
                }
            }
            let t0 = unit(Vec2::new(rnd() - 0.5, rnd() - 0.5)).unwrap_or(Vec2::new(1.0, 0.0));
            let t1 = unit(Vec2::new(rnd() - 0.5, rnd() - 0.5)).unwrap_or(Vec2::new(1.0, 0.0));
            let out = fit_cubics(&pts, t0, t1, tol);
            assert_eq!(out.first().map(|c| c.p0), pts.first().copied(), "case {case}");
            assert_eq!(out.last().map(|c| c.p3), pts.last().copied(), "case {case}");
            for w in out.windows(2) {
                assert_eq!(w[0].p3, w[1].p0, "case {case}");
            }
            let far = farthest(&pts, &out, tol);
            assert!(far <= tol + 1e-9, "case {case}: a point is {far} from the fit at a tolerance of {tol}");
        }
        // Samples of a long serpentine, which split lopsidedly at their worst points.
        let pts = samples(&cubics(&PathData::single(serpentine(1, 1000, false))));
        let out = fit_cubics(&pts, Vec2::new(1.0, 0.0), Vec2::new(1.0, 0.0), 1.0);
        let far = farthest(&pts, &out, 1.0);
        assert!(far <= 1.0 + 1e-9, "a point of the serpentine is {far} from the fit");
    }

    /// `n` pieces of one segment each along a straight line, and the line's points.
    fn straight_pieces(n: usize) -> (Vec<Point>, Vec<Piece>) {
        let pts: Vec<Point> = (0..=n).map(|i| Point::new(i as f64, 0.0)).collect();
        let t1 = Vec2::new(1.0, 0.0);
        let pieces = (1..=n).map(|end| Piece { curve: CubicBez::new(pts[end - 1], pts[end - 1], pts[end], pts[end]), end, t1 }).collect();
        (pts, pieces)
    }

    #[test]
    fn joined_pieces_make_one_curve_up_to_the_limit() {
        let t0 = Vec2::new(1.0, 0.0);
        let (pts, pieces) = straight_pieces(MAX_JOINS + 1);
        let out = join_pieces(&pts, &pieces, t0, 1.0);
        assert_eq!(out.len(), 1);
        assert_eq!((out[0].p0, out[0].p3), (pts[0], pts[MAX_JOINS + 1]));
    }

    #[test]
    fn a_curve_joins_no_more_pieces_than_the_limit() {
        let (pts, pieces) = straight_pieces(1000);
        let out = join_pieces(&pts, &pieces, Vec2::new(1.0, 0.0), 1.0);
        assert_eq!(out.len(), 1000_usize.div_ceil(MAX_JOINS + 1));
        assert_eq!((out[0].p0, out[out.len() - 1].p3), (pts[0], pts[1000]));
        for w in out.windows(2) {
            assert_eq!(w[0].p3, w[1].p0);
        }
    }

    #[test]
    fn straight_detection() {
        let c = CubicBez::new((0.0, 0.0), (3.0, 0.0), (6.0, 0.0), (9.0, 0.0));
        assert!(is_straight(&c, 1e-6));
        let c = CubicBez::new((0.0, 0.0), (3.0, 1.0), (6.0, 0.0), (9.0, 0.0));
        assert!(!is_straight(&c, 1e-6));
    }
}
