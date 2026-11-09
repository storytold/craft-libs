// Carved from VectorCraft's crates/pathops (src/fit.rs) at storytold/vectorcraft ff9deee46d1c59dc9ecef0b668c25e57cd80b55e.
// Copyright (c) 2026 ArtCraft Team and the VectorCraft contributors. MIT OR Apache-2.0.

//! Least-squares cubic Bézier fitting of point sequences (the classic "fit a cubic with fixed end
//! tangents, then reparameterise with Newton steps, then split at the worst point" approach from
//! Graphics Gems, written from the published description).

use kurbo::{CubicBez, ParamCurve, ParamCurveDeriv, PathSeg, Point, Vec2};

/// The largest x or y, in size, of a segment's control points for which its length is measured.
/// Drawing coordinates are far inside it; past it (or not finite) the integrator's arithmetic
/// overflows and it subdivides without end.
const MEASURE_LIMIT: f64 = 1e9;

/// The length of `seg` to `accuracy`; `None` for a segment whose control points are not finite or
/// past [`MEASURE_LIMIT`] (its length is not zero: it is not known, and a caller must not sample
/// it as if it were a point).
pub(crate) fn measured_arclen(seg: &PathSeg, accuracy: f64) -> Option<f64> {
    arclen_within(seg, accuracy, MEASURE_LIMIT)
}

/// The largest x or y a segment may have for [`measured_perimeter`] to measure it: it takes an accuracy
/// relative to the segment, which kurbo meets in a few levels at any size, and only stops where the
/// squares of the coordinates would overflow.
const RELATIVE_MEASURE_LIMIT: f64 = 1e100;

/// [`measured_arclen`] for segments up to `limit` in x and y.
fn arclen_within(seg: &PathSeg, accuracy: f64, limit: f64) -> Option<f64> {
    let ok = |p: Point| p.x.is_finite() && p.y.is_finite() && p.x.abs() <= limit && p.y.abs() <= limit;
    let fits = match seg {
        PathSeg::Line(l) => ok(l.p0) && ok(l.p1),
        PathSeg::Quad(q) => ok(q.p0) && ok(q.p1) && ok(q.p2),
        PathSeg::Cubic(c) => ok(c.p0) && ok(c.p1) && ok(c.p2) && ok(c.p3),
    };
    fits.then(|| kurbo::ParamCurveArclen::arclen(seg, accuracy))
}

/// The length of `path`, each cubic measured to `relative` of the length of its control polygon, or
/// `None` when a segment cannot be measured (not finite, or past 1e100 in x or y). kurbo's `perimeter(accuracy)` takes
/// an accuracy in the units of the path, which a cubic of large coordinates can only meet by
/// subdividing it the most it ever does (twenty levels), and a cubic cannot be told apart from one
/// that does; a relative accuracy is met in a few levels whatever the coordinates, so the work for a
/// segment is a small constant. The cubics measured are counted ([`Work::Measured`]).
pub(crate) fn measured_perimeter(path: &kurbo::BezPath, relative: f64) -> Option<f64> {
    let mut total = 0.0;
    let mut curves = 0usize;
    for seg in path.segments() {
        total += match seg {
            PathSeg::Line(l) => (l.p1 - l.p0).hypot(),
            PathSeg::Quad(q) => {
                curves += 1;
                arclen_within(&seg, (relative * ((q.p1 - q.p0).hypot() + (q.p2 - q.p1).hypot())).max(1e-300), RELATIVE_MEASURE_LIMIT)?
            }
            PathSeg::Cubic(c) => {
                curves += 1;
                // Never an accuracy of zero: a cubic that is a point has no error, and no error is
                // not below an accuracy of zero, so it would be subdivided to the bottom.
                let polygon = (c.p1 - c.p0).hypot() + (c.p2 - c.p1).hypot() + (c.p3 - c.p2).hypot();
                arclen_within(&seg, (relative * polygon).max(1e-300), RELATIVE_MEASURE_LIMIT)?
            }
        };
    }
    crate::work::add(crate::work::Work::Measured, curves);
    total.is_finite().then_some(total)
}

/// Maximum Newton reparameterisation rounds per fit.
const MAX_REPARAM: usize = 6;

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

/// Why a fit gave nothing.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum FitStop {
    /// The work ran out.
    Work,
    /// A stretch of points was split [`MAX_FIT_DEPTH`] times and still did not fit within the tolerance
    /// (the points are too unevenly spaced for the splitting to get anywhere): no cubic of it is
    /// within the tolerance, and one that is not is not given.
    Depth,
}

/// Most times a stretch of points is split in two to fit it.
const MAX_FIT_DEPTH: usize = 40;

/// Fit a chain of cubics through `pts` within `tol` of the path they are samples of, which lies within
/// `between` of their polyline (`(tol, between)`; a `between` of 0 when the polyline is the path).
///
/// `t0` is the unit tangent leaving `pts[0]`; `t1` is the unit tangent arriving at the last point
/// (forward direction). Consecutive output cubics are G1 continuous.
///
/// A cubic is taken when its error at the points, how far it can be from the chords between its
/// values at their parameters ([`spread`]), and `between` add up to `tol` at most: every point of
/// the cubic is then within `tol` of the path, and every point of the path within `tol` of the cubic.
/// Each cubic comes with that sum, how far it and the path can be from each other.
///
/// Each fit of a stretch of points is charged to `work` (its points times the rounds of
/// reparameterisation it may take); an error when `work` runs out first, or when no fit within
/// `tol` is found in [`MAX_FIT_DEPTH`] splits.
pub(crate) fn fit_cubics(pts: &[Point], t0: Vec2, t1: Vec2, tol: (f64, f64), work: &mut usize) -> Result<Vec<(CubicBez, f64)>, FitStop> {
    fit_cubics_to_depth(pts, t0, t1, tol, work, MAX_FIT_DEPTH)
}

/// [`fit_cubics`] splitting a stretch at most `max_depth` times.
fn fit_cubics_to_depth(
    pts: &[Point],
    t0: Vec2,
    t1: Vec2,
    (tol, between): (f64, f64),
    work: &mut usize,
    max_depth: usize,
) -> Result<Vec<(CubicBez, f64)>, FitStop> {
    let mut out = Vec::new();
    let pts = dedup(pts);
    if pts.len() < 2 {
        return Ok(out);
    }
    // What is left of the tolerance for the cubic's own distance from the polyline; none, and no
    // cubic is close enough.
    let room = tol.max(1e-9) - between.max(0.0);
    if room.is_nan() || room <= 0.0 {
        return Err(FitStop::Depth);
    }
    fit_rec(&pts, t0, t1, room, &mut out, (0, max_depth), work)?;
    Ok(out.into_iter().map(|(c, off)| (c, off + between.max(0.0))).collect())
}

/// `pts` without the points that lie within 1e-12 of the one kept before them.
fn dedup(pts: &[Point]) -> Vec<Point> {
    let mut v: Vec<Point> = Vec::with_capacity(pts.len());
    for &p in pts {
        if v.last().is_none_or(|q: &Point| q.distance(p) > 1e-12) {
            v.push(p);
        }
    }
    v
}

/// The fit of `pts` within `work` and the depth `(so far, most)`, or why there is none. A cubic is
/// taken when its error at the points and its [`spread`] add up to `tol` at most, and given with that sum.
fn fit_rec(
    pts: &[Point],
    t0: Vec2,
    t1: Vec2,
    tol: f64,
    out: &mut Vec<(CubicBez, f64)>,
    depth: (usize, usize),
    work: &mut usize,
) -> Result<(), FitStop> {
    let n = pts.len();
    if n == 2 {
        let d = pts[0].distance(pts[1]) / 3.0;
        let c = CubicBez::new(pts[0], pts[0] + t0 * d, pts[1] - t1 * d, pts[1]);
        // Through both points, and no more points to split at: within the tolerance of their chord, or no fit.
        let off = spread(&c, &[0.0, 1.0]);
        if off > tol {
            return Err(FitStop::Depth);
        }
        out.push((c, off));
        return Ok(());
    }
    // A fit reads its points once, then again for each round of reparameterisation.
    let cost = n.saturating_mul(1 + MAX_REPARAM);
    let Some(left) = work.checked_sub(cost) else {
        *work = 0;
        return Err(FitStop::Work);
    };
    *work = left;
    crate::work::add(crate::work::Work::Fit, cost);
    let (c, err, split, u) = fit_with_params(pts, chord_params(pts), t0, t1);
    let off = err + spread(&c, &u);
    if off <= tol {
        out.push((c, off));
        return Ok(());
    }
    // Past the depth the fit is not within the tolerance: say so, rather than give it.
    let (so_far, most) = depth;
    if so_far >= most {
        return Err(FitStop::Depth);
    }
    // Split at the worst point with a centre tangent estimated from its neighbours.
    let split = split.clamp(1, n - 2);
    let tc = unit(pts[split + 1] - pts[split - 1]).unwrap_or(t0);
    fit_rec(&pts[..=split], t0, tc, tol, out, (so_far + 1, most), work)?;
    fit_rec(&pts[split..], tc, t1, tol, out, (so_far + 1, most), work)
}

/// Best single cubic for `pts` with the given end tangents. Returns (curve, max error, index of
/// the worst point).
pub(crate) fn fit_single(pts: &[Point], t0: Vec2, t1: Vec2) -> (CubicBez, f64, usize) {
    fit_single_from(pts, chord_params(pts), t0, t1)
}

/// [`fit_single`] held to the path the points `pts` are samples of: the cubic, its error at the points,
/// and how much more it and that path can be from each other: its [`spread`] and `between`, how far the
/// path can be from the polyline of the points, added up. The two are the bound ([`fit_cubics`] takes a
/// cubic by the same sum), and are given apart for a caller that weighs the error before the rest.
pub(crate) fn fit_single_bounded(pts: &[Point], t0: Vec2, t1: Vec2, between: f64) -> (CubicBez, f64, f64) {
    let (c, err, _, u) = fit_with_params(pts, chord_params(pts), t0, t1);
    (c, err, spread(&c, &u) + between)
}

/// [`fit_single`] starting from the parameters `u` (one per point, 0 to 1) instead of the
/// chord-length ones.
pub(crate) fn fit_single_from(pts: &[Point], u: Vec<f64>, t0: Vec2, t1: Vec2) -> (CubicBez, f64, usize) {
    let (best, best_err, best_split, _) = fit_with_params(pts, u, t0, t1);
    (best, best_err, best_split)
}

/// [`fit_single_from`], and the parameters of the points on the cubic its error is measured at.
fn fit_with_params(pts: &[Point], mut u: Vec<f64>, t0: Vec2, t1: Vec2) -> (CubicBez, f64, usize, Vec<f64>) {
    let mut best = generate(pts, &u, t0, t1);
    let (mut best_err, mut best_split) = max_error(pts, &best, &u);
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
            best_u.clone_from(&u);
        } else {
            break;
        }
    }
    (best, best_err, best_split, best_u)
}

/// How far the cubic `c` can be from the polyline through its values at the parameters `u` (one per
/// point it was fitted to, in order), with its ends at 0 and 1 added: an eighth of the bound on its
/// second derivative times the square of the widest step between consecutive parameters.
///
/// A parameter `t` of `c` lies between two consecutive ones, `u_i` and `u_j`, a fraction `λ` of the way.
/// `c(t)` is within this of the point that fraction along the chord from `c(u_i)` to `c(u_j)`; that
/// point is within the larger of the errors at `i` and `j` of the point the same fraction along the
/// chord between the points themselves; and a point of the points' polyline is within `between` of the
/// path ([`fit_cubics`]). Each point of the path is near the cubic by the same steps the other way.
fn spread(c: &CubicBez, u: &[f64]) -> f64 {
    let mut widest: f64 = 0.0;
    let mut last = 0.0;
    for &t in u.iter().chain(std::iter::once(&1.0)) {
        widest = widest.max((t - last).abs());
        last = t;
    }
    0.75 * second_difference(c) * widest * widest
}

/// The length of the control polygon of `c`: at least the length of the curve.
pub(crate) fn polygon_length(c: &CubicBez) -> f64 {
    (c.p1 - c.p0).hypot() + (c.p2 - c.p1).hypot() + (c.p3 - c.p2).hypot()
}

/// The larger of the two second differences of the control points of `c`: six times it bounds the
/// second derivative of the curve.
pub(crate) fn second_difference(c: &CubicBez) -> f64 {
    let second = |a: Point, b: Point, c: Point| ((c - b) - (b - a)).hypot();
    second(c.p0, c.p1, c.p2).max(second(c.p1, c.p2, c.p3))
}

/// The parameter of each of `pts` by the length of the polyline through them, from 0 at the first to 1
/// at the last (all 0 when the polyline has no length).
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

/// The largest distance of `pts` from the cubic `c` at their parameters `u`, and the index of the point
/// it is at (the middle one when all are on it).
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

/// One Newton step from `t` towards the parameter of the point of `c` nearest `p`, kept in `[0, 1]`
/// (`t` itself where the step is not defined).
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

/// How far the cubic `c` and its chord can be from each other: the larger distance of its two handles
/// from the chord. The cubic lies in the hull of its control points, which are no further from the chord
/// than that; and each point of the chord has a point of the cubic level with it along the chord, no
/// further off it than the hull.
pub(crate) fn chord_distance(c: &CubicBez) -> f64 {
    let to_chord = |p: Point| {
        let (d, h) = (c.p3 - c.p0, p - c.p0);
        let len_sq = d.hypot2();
        let t = if len_sq > 0.0 { (h.dot(d) / len_sq).clamp(0.0, 1.0) } else { 0.0 };
        (h - d * t).hypot()
    };
    to_chord(c.p1).max(to_chord(c.p2))
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

    #[test]
    fn fits_a_cubic_exactly() {
        let c = CubicBez::new((0.0, 0.0), (30.0, 60.0), (70.0, 60.0), (100.0, 0.0));
        let mut pts = Vec::new();
        sample(&c, 40, &mut pts, true);
        let mut work = usize::MAX;
        let out: Vec<CubicBez> =
            fit_cubics(&pts, start_tangent(&c), end_tangent(&c), (0.05, 0.0), &mut work).unwrap().into_iter().map(|f| f.0).collect();
        assert_eq!(out.len(), 1);
        assert!(out[0].p1.distance(c.p1) < 1.0, "{:?}", out[0]);
    }

    #[test]
    fn splits_when_needed() {
        // A zig-zag cannot be fitted with one cubic. (Its points are taken for the path, as their
        // polyline: dense enough that cubics can follow it within the tolerance.)
        let pts: Vec<Point> = (0..=800).map(|i| Point::new(f64::from(i) * 0.05, (f64::from(i) * 0.025).sin() * 10.0)).collect();
        let mut work = usize::MAX;
        let out: Vec<CubicBez> =
            fit_cubics(&pts, Vec2::new(1.0, 0.0), Vec2::new(1.0, 0.0), (0.1, 0.0), &mut work).unwrap().into_iter().map(|f| f.0).collect();
        assert!(out.len() > 1);
        for w in out.windows(2) {
            assert!(w[0].p3.distance(w[1].p0) < 1e-9);
        }
    }

    /// The room left for the fit is the tolerance less the sampling bound, and the bound given with
    /// each cubic includes it: a cubic that fits in the room is given with its own bound and the
    /// sampling bound added; one that does not is split, or no fit when no split is allowed.
    #[test]
    fn the_sampling_bound_is_taken_from_the_room_and_added_to_the_bound() {
        // A gentle wave, one cubic's worth: its bound with nothing reserved is `alone`.
        let pts: Vec<Point> =
            (0..=200).map(|i| Point::new(f64::from(i) * 0.1, (f64::from(i) * 0.1 * std::f64::consts::PI / 20.0).sin() * 0.2)).collect();
        let (t0, t1) = (Vec2::new(1.0, 0.0), unit(pts[200] - pts[199]).unwrap_or(Vec2::new(1.0, 0.0)));
        let tol = 1.0;
        let mut work = usize::MAX;
        let one = fit_cubics_to_depth(&pts, t0, t1, (tol, 0.0), &mut work, 0).unwrap();
        assert_eq!(one.len(), 1);
        let alone = one[0].1;
        assert!(alone > 0.0 && alone < tol / 4.0, "{alone}");
        // With room for it, the same cubic, its bound the sampling bound more.
        let between = tol - 2.0 * alone;
        let reserved = fit_cubics_to_depth(&pts, t0, t1, (tol, between), &mut work, 0).unwrap();
        assert_eq!(reserved.len(), 1);
        assert!((reserved[0].1 - (alone + between)).abs() < 1e-12 && reserved[0].1 <= tol, "{} against {}", reserved[0].1, alone + between);
        // Without room for it the cubic is no fit: split, or past the splits allowed, none.
        assert_eq!(fit_cubics_to_depth(&pts, t0, t1, (tol, tol - alone / 2.0), &mut work, 0), Err(FitStop::Depth));
        let split = fit_cubics(&pts, t0, t1, (tol, tol - alone / 2.0), &mut work).unwrap();
        assert!(split.len() > 1 && split.iter().all(|piece| piece.1 <= tol));
        // And none at all when the sampling bound is the whole tolerance.
        assert_eq!(fit_cubics(&pts, t0, t1, (tol, tol), &mut work), Err(FitStop::Depth));
    }

    /// A stretch that has not fitted when the splitting stops is no fit: the cubic that failed is not
    /// given as if it were within the tolerance, and the work is not spent as if it had run out.
    #[test]
    fn a_fit_that_runs_out_of_splits_is_an_error_not_a_cubic_out_of_tolerance() {
        let pts: Vec<Point> = (0..=800).map(|i| Point::new(f64::from(i) * 0.05, (f64::from(i) * 0.025).sin() * 10.0)).collect();
        let (t0, t1) = (Vec2::new(1.0, 0.0), Vec2::new(1.0, 0.0));
        let mut work = usize::MAX;
        assert_eq!(fit_cubics_to_depth(&pts, t0, t1, (0.1, 0.0), &mut work, 0), Err(FitStop::Depth));
        assert!(work > usize::MAX / 2, "the work left is what the one fit tried cost: {}", usize::MAX - work);
        // With room to split the cubics given are within the tolerance of the points.
        let mut work = usize::MAX;
        let out: Vec<CubicBez> = fit_cubics(&pts, t0, t1, (0.1, 0.0), &mut work).unwrap().into_iter().map(|f| f.0).collect();
        for p in &pts {
            let near = out.iter().map(|c| kurbo::ParamCurveNearest::nearest(c, *p, 1e-9).distance_sq.sqrt()).fold(f64::INFINITY, f64::min);
            assert!(near <= 0.1 + 1e-6, "{p:?} is {near} from the fit");
        }
        // Work that runs out is the other error.
        let mut little = 100;
        assert_eq!(fit_cubics(&pts, t0, t1, (0.1, 0.0), &mut little), Err(FitStop::Work));
        assert_eq!(little, 0);
    }

    /// The perimeter is measured to a relative accuracy: a cubic that is a point is nothing (an accuracy
    /// of zero takes kurbo 42 ms to subdivide it to the bottom, against 180 ns for the smallest there is;
    /// that is a time, so no test here can fail on it), a segment past the limit or not finite cannot be
    /// measured, and the cubics measured are counted.
    #[test]
    fn the_perimeter_is_measured_to_a_relative_accuracy_and_counted() {
        let mut point = kurbo::BezPath::new();
        point.move_to((1e5, 1e5));
        point.curve_to((1e5, 1e5), (1e5, 1e5), (1e5, 1e5));
        crate::work::reset();
        assert_eq!(measured_perimeter(&point, 1e-9), Some(0.0));
        assert_eq!(crate::work::take(crate::work::Work::Measured), 1);
        let mut square = kurbo::BezPath::new();
        square.move_to((0.0, 0.0));
        square.line_to((3.0, 0.0));
        square.line_to((3.0, 4.0));
        assert_eq!(measured_perimeter(&square, 1e-9), Some(7.0));
        assert_eq!(crate::work::take(crate::work::Work::Measured), 0, "lines are chords");
        for (name, bad) in [("past the limit", 1e200), ("not a number", f64::NAN), ("infinite", f64::INFINITY)] {
            let mut p = kurbo::BezPath::new();
            p.move_to((0.0, 0.0));
            p.curve_to((bad, 0.0), (0.0, bad), (1.0, 1.0));
            assert_eq!(measured_perimeter(&p, 1e-9), None, "{name}");
        }
    }

    /// The relative accuracy needs no limit of a billion: a curve of coordinates up to the sweep's own
    /// (about 7e13) is measured, to the same relative accuracy as one of ordinary size.
    #[test]
    fn large_curves_are_measured_to_the_same_relative_accuracy() {
        for scale in [1e9, 1e10, 1e12, 5e13] {
            let ring = kurbo::Shape::to_path(&kurbo::Circle::new((0.0, 0.0), scale), scale * 1e-4);
            let len = measured_perimeter(&ring, 1e-9).unwrap_or_else(|| panic!("scale {scale} cannot be measured"));
            let exact = kurbo::Shape::perimeter(&ring, scale * 1e-12);
            assert!((len - exact).abs() < 1e-8 * exact, "scale {scale}: {len} against {exact}");
        }
    }

    #[test]
    fn straight_detection() {
        let c = CubicBez::new((0.0, 0.0), (3.0, 0.0), (6.0, 0.0), (9.0, 0.0));
        assert!(is_straight(&c, 1e-6));
        let c = CubicBez::new((0.0, 0.0), (3.0, 1.0), (6.0, 0.0), (9.0, 0.0));
        assert!(!is_straight(&c, 1e-6));
    }

    /// A fit held to its path gives the cubic and its error at the points as a plain fit does, and apart
    /// from them the rest of the bound: the cubic's spread from its chords, and `between`.
    #[test]
    fn a_bounded_fit_gives_its_error_and_the_rest_apart() {
        let c = CubicBez::new((0.0, 0.0), (10.0, 20.0), (30.0, 20.0), (40.0, 0.0));
        let mut pts = Vec::new();
        sample(&c, 4, &mut pts, true);
        let (t0, t1) = (start_tangent(&c), end_tangent(&c));
        let (fit, err, rest) = fit_single_bounded(&pts, t0, t1, 0.25);
        let (plain, plain_err, _) = fit_single(&pts, t0, t1);
        assert_eq!((fit, err), (plain, plain_err));
        // Four intervals of a cubic that bends this much are far from their chords: the spread is a part
        // of the rest, and `between` the other.
        let (_, _, spread_only) = fit_single_bounded(&pts, t0, t1, 0.0);
        assert!(spread_only > 0.5, "{spread_only}");
        assert!((rest - spread_only - 0.25).abs() < 1e-12, "{rest} against {spread_only} and 0.25");
    }
}
