// Carved from VectorCraft's crates/pathops (src/fit.rs) at storytold/vectorcraft ff9deee46d1c59dc9ecef0b668c25e57cd80b55e.
// Copyright (c) 2026 ArtCraft Team and the VectorCraft contributors. MIT OR Apache-2.0.
// Brought up to storytold/vectorcraft 0945eaf2b3d4e93963aaa6d796b9efe53feaf624 by porting its changes to this file (3fcd157..0945eaf; see the README).

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

/// Most times a stretch of points is split in two to fit it. A split keeps at least two fifths of a stretch on each side
/// ([`fit_rec`]), so a stretch of `n` points is split at most about 2 log2 `n` times in a run: a million points need about 27.
const MAX_FIT_DEPTH: usize = 40;

/// Most pieces joined to the first piece of a fitted curve ([`join_pieces`]). Each join refits every point of the curve, so this limits
/// how many times a point is refitted for a join.
const MAX_JOINS: usize = 32;

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
/// A stretch that is not taken is split at its worst point, moved to two fifths of the stretch from an end when it is nearer one
/// (a worst point at an end peels off a few points, so a long path refitted nearly all of its points at every level, and ran
/// out of splits), and neighbouring pieces that one cubic fits are joined again ([`join_pieces`]).
///
/// Each fit of a stretch of points is charged to `work` (its points times the rounds of
/// reparameterisation it may take), as is each fit that tries a join; an error when `work` runs out first, or when no fit within
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
    let pts = dedup(pts);
    if pts.len() < 2 {
        return Ok(Vec::new());
    }
    // What is left of the tolerance for the cubic's own distance from the polyline; none, and no
    // cubic is close enough.
    let room = tol.max(1e-9) - between.max(0.0);
    if room.is_nan() || room <= 0.0 {
        return Err(FitStop::Depth);
    }
    let mut pieces = Vec::new();
    fit_rec(&pts, (0, pts.len() - 1), (t0, t1), room, &mut pieces, (0, max_depth), work)?;
    let pieces = join_pieces(&pts, pieces, t0, room, work);
    Ok(pieces.into_iter().map(|p| (p.curve, p.off + between.max(0.0))).collect())
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

/// A fitted piece: its curve and its bound (how far it and the path can be from each other, less what the caller holds back), the index
/// of the point it ends on, and its tangent there.
#[derive(Clone, Copy)]
struct Piece {
    curve: CubicBez,
    off: f64,
    end: usize,
    t1: Vec2,
}

/// The fit of `all[start..=end]` within `work` and the depth `(so far, most)`, or why there is none. A cubic is taken when its error at
/// the points and its [`spread`] add up to `tol` at most, and given with that sum.
///
/// A piece of two points is a cubic through both with handles along the tangents, at most `tol` long, so that it lies within `tol` of
/// the chord between them, which is all its bound is: no more points to split at.
fn fit_rec(
    all: &[Point],
    (start, end): (usize, usize),
    (t0, t1): (Vec2, Vec2),
    tol: f64,
    out: &mut Vec<Piece>,
    depth: (usize, usize),
    work: &mut usize,
) -> Result<(), FitStop> {
    let Some(pts) = all.get(start..=end) else { return Err(FitStop::Depth) };
    let n = pts.len();
    if n == 2 {
        // (A handle that points away from the chord is `d` from its end, so `d` is held a hair under the tolerance, for the rounding.)
        let d = (pts[0].distance(pts[1]) / 3.0).min(tol * (1.0 - 1e-9));
        let c = CubicBez::new(pts[0], pts[0] + t0 * d, pts[1] - t1 * d, pts[1]);
        // Through both points: within the tolerance of their chord by where its handles lie (the hull), or no fit.
        let off = chord_distance(&c);
        if off > tol {
            return Err(FitStop::Depth);
        }
        out.push(Piece { curve: c, off, end, t1 });
        return Ok(());
    }
    let split = match try_fit(pts, (t0, t1), tol, &mut paying(work)) {
        Ok((curve, off)) => {
            out.push(Piece { curve, off, end, t1 });
            return Ok(());
        }
        Err(Unfit::Work) => return Err(FitStop::Work),
        Err(Unfit::At(split)) => split,
    };
    // Past the depth the fit is not within the tolerance: say so, rather than give it.
    let (so_far, most) = depth;
    if so_far >= most {
        return Err(FitStop::Depth);
    }
    // Split at the worst point, moved to two fifths of the stretch from the nearer end if it is nearer than that, so that each half holds
    // three fifths of the points at most and the splitting is as deep as the logarithm of the points. `n` is 3 or more here, and the
    // bounds are in order for it (3 and 4 give 1 and 1 or 2, and 5 and more give two fifths and its mirror).
    let split = split_index(split, n);
    // The direction of the path at the split, between the nearest points on each side that are over `tol` from it, so that details
    // smaller than the tolerance do not turn it. The search may read every point of the stretch (all within the tolerance of the split),
    // which a fit that was refused at its first sixteen points did not cost: charged, before it is made.
    if !paying(work)(n) {
        return Err(FitStop::Work);
    }
    crate::work::add(crate::work::Work::SplitScans, n);
    let tc = center_tangent(pts, split, tol).unwrap_or(t0);
    fit_rec(all, (start, start + split), (t0, tc), tol, out, (so_far + 1, most), work)?;
    fit_rec(all, (start + split, end), (tc, t1), tol, out, (so_far + 1, most), work)
}

/// Where a stretch of `n` points (3 or more) is split, given its worst point `worst`: there, moved to two fifths of the stretch from the nearer
/// end where it is nearer than that. Each half then holds three fifths of the points at most, and the splitting is as deep as the
/// logarithm of the points. For `n` of 3 and 4 the stretch is split at the point beside the end that is (3: the middle; 4: either of the
/// two inner ones), and for 5 and more the bounds are two fifths and its mirror.
fn split_index(worst: usize, n: usize) -> usize {
    let margin = (n * 2 / 5).max(1);
    if margin <= n.saturating_sub(1 + margin) { worst.clamp(margin, n - 1 - margin) } else { n / 2 }
}

/// Direction at `pts[split]`, between the nearest points on each side that are over `tol` from it, or the first and last points when
/// none is, so that details smaller than `tol` do not turn it.
fn center_tangent(pts: &[Point], split: usize, tol: f64) -> Option<Vec2> {
    let p = *pts.get(split)?;
    let before = pts.get(..split)?.iter().rev().find(|q| q.distance(p) > tol).or(pts.first())?;
    let after = pts.get(split + 1..)?.iter().find(|q| q.distance(p) > tol).or(pts.last())?;
    unit(*after - *before)
}

/// The pieces of a fit, which follow each other along `pts` from its first point and leave it along `t0`, with each curve taking up to
/// [`MAX_JOINS`] of the pieces after its first one, as long as the fit of them together is within `tol` (a cubic fits where the split
/// was only the depth of a recursion that moved its splits inward). Each try is a fit of the curve's points, charged to `work` as a
/// fit is; where `work` runs out the pieces after are not joined, which leaves them as fitted.
fn join_pieces(pts: &[Point], pieces: Vec<Piece>, t0: Vec2, tol: f64, work: &mut usize) -> Vec<Piece> {
    let mut out = Vec::with_capacity(pieces.len());
    let (mut start, mut leaving, mut next) = (0usize, t0, 0usize);
    while let Some(&first) = pieces.get(next) {
        let mut curve = first;
        next += 1;
        for piece in pieces.iter().skip(next).take(MAX_JOINS) {
            let Some(span) = pts.get(start..=piece.end) else { break };
            let Ok((c, off)) = try_fit(span, (leaving, piece.t1), tol, &mut paying(work)) else { break };
            curve = Piece { curve: c, off, end: piece.end, t1: piece.t1 };
            next += 1;
        }
        (start, leaving) = (curve.end, curve.t1);
        out.push(curve);
    }
    out
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
fn fit_with_params(pts: &[Point], u: Vec<f64>, t0: Vec2, t1: Vec2) -> (CubicBez, f64, usize, Vec<f64>) {
    let first = u.clone();
    fit_within(pts, u, t0, t1, f64::INFINITY, &mut |_| true).unwrap_or_else(|| {
        // Not reached: the charge always pays. The first fit, without a round of reparameterisation, is the answer that is left.
        let c = generate(pts, &first, t0, t1);
        let (err, split) = max_error(pts, &c, &first);
        (c, err, split, first)
    })
}

/// [`fit_with_params`] charging its work to `pay` as it goes (the points once for the first fit, and again for each of the
/// rounds of reparameterisation, which are paid for together before the first), and giving the first fit as the answer, without a
/// round, when its error is over `give_up` (a fit that far off is split, and the rounds would not bring it near). `None`
/// when `pay` refuses a charge, with nothing more made.
fn fit_within(
    pts: &[Point],
    mut u: Vec<f64>,
    t0: Vec2,
    t1: Vec2,
    give_up: f64,
    pay: &mut dyn FnMut(usize) -> bool,
) -> Option<(CubicBez, f64, usize, Vec<f64>)> {
    if !pay(pts.len()) {
        return None;
    }
    let mut best = generate(pts, &u, t0, t1);
    let (mut best_err, mut best_split) = max_error(pts, &best, &u);
    let mut best_u = u.clone();
    if best_err > give_up {
        return Some((best, best_err, best_split, best_u));
    }
    if !pay(pts.len().saturating_mul(MAX_REPARAM)) {
        return None;
    }
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
    Some((best, best_err, best_split, best_u))
}

/// About how many points of a long stretch its first fit uses ([`try_fit`]).
const COARSE_POINTS: usize = 16;

/// Why [`try_fit`] gave no cubic.
enum Unfit {
    /// The charge was refused: the work ran out.
    Work,
    /// The index of the point to split at: no cubic of the stretch is within the tolerance.
    At(usize),
}

/// One cubic for the stretch `pts`, with its bound, when it is within `tol` (its error at the points and its [`spread`] add up to it
/// at most); otherwise the point to split at. A stretch of `2 * COARSE_POINTS` points or more is first fitted to about
/// [`COARSE_POINTS`] of them and is split at once when that fit is over `tol` there, which costs the sixteen and not the stretch;
/// and a fit over 8 times `tol` plus 5% of the chord is not reparameterized (it is split, and the rounds would not make it fit).
/// Each fit is charged to `pay` before it is made.
fn try_fit(pts: &[Point], (t0, t1): (Vec2, Vec2), tol: f64, pay: &mut dyn FnMut(usize) -> bool) -> Result<(CubicBez, f64), Unfit> {
    let n = pts.len();
    let (Some(first), Some(last)) = (pts.first(), pts.last()) else { return Err(Unfit::Work) };
    let give_up = tol * 8.0 + first.distance(*last) * 0.05;
    if n >= COARSE_POINTS * 2 {
        let stride = n / COARSE_POINTS;
        let some: Vec<Point> = pts.iter().step_by(stride).chain(pts.last()).copied().collect();
        let (_, err, worst, _) = fit_within(&some, chord_params(&some), t0, t1, give_up, pay).ok_or(Unfit::Work)?;
        if err > tol {
            return Err(Unfit::At((worst * stride).min(n - 1)));
        }
    }
    let (c, err, worst, u) = fit_within(pts, chord_params(pts), t0, t1, give_up, pay).ok_or(Unfit::Work)?;
    let off = err + spread(&c, &u);
    if off <= tol { Ok((c, off)) } else { Err(Unfit::At(worst)) }
}

/// A charge to `work` that says so, in the shape [`fit_within`] takes: it pays what it is asked while `work` holds it, and
/// where it does not leaves `work` at nothing and refuses.
fn paying(work: &mut usize) -> impl FnMut(usize) -> bool + '_ {
    move |cost| match work.checked_sub(cost) {
        Some(left) => {
            *work = left;
            crate::work::add(crate::work::Work::Fit, cost);
            true
        }
        None => {
            *work = 0;
            false
        }
    }
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
    /// Numbers from 0 to 1, the same ones for the same `seed`.
    fn random(mut seed: u64) -> impl FnMut() -> f64 {
        move || {
            seed = seed.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1_442_695_040_888_963_407);
            (seed >> 11) as f64 / (1u64 << 53) as f64
        }
    }

    /// Largest distance from a point of `pts` to the nearest of the cubics `cs`.
    fn farthest(pts: &[Point], cs: &[CubicBez]) -> f64 {
        use kurbo::ParamCurveNearest;
        pts.iter().map(|&p| cs.iter().map(|c| c.nearest(p, 1e-9).distance_sq.sqrt()).fold(f64::INFINITY, f64::min)).fold(0.0, f64::max)
    }

    /// `n` pieces of one segment each along a straight line, and the line's points.
    fn straight_pieces(n: usize) -> (Vec<Point>, Vec<Piece>) {
        let pts: Vec<Point> = (0..=n).map(|i| Point::new(i as f64, 0.0)).collect();
        let t1 = Vec2::new(1.0, 0.0);
        let pieces = (1..=n).map(|end| Piece { curve: CubicBez::new(pts[end - 1], pts[end - 1], pts[end], pts[end]), off: 0.0, end, t1 }).collect();
        (pts, pieces)
    }

    /// Pieces that one cubic fits are one curve, up to [`MAX_JOINS`] more than the first (VectorCraft 199c4c3's test).
    #[test]
    fn joined_pieces_make_one_curve_up_to_the_limit() {
        let t0 = Vec2::new(1.0, 0.0);
        let (pts, pieces) = straight_pieces(MAX_JOINS + 1);
        let mut work = usize::MAX;
        let out = join_pieces(&pts, pieces, t0, 1.0, &mut work);
        assert_eq!(out.len(), 1);
        assert_eq!((out[0].curve.p0, out[0].curve.p3), (pts[0], pts[MAX_JOINS + 1]));
    }

    #[test]
    fn a_curve_joins_no_more_pieces_than_the_limit() {
        let (pts, pieces) = straight_pieces(1000);
        let mut work = usize::MAX;
        let out = join_pieces(&pts, pieces, Vec2::new(1.0, 0.0), 1.0, &mut work);
        assert_eq!(out.len(), 1000_usize.div_ceil(MAX_JOINS + 1));
        assert_eq!((out[0].curve.p0, out[out.len() - 1].curve.p3), (pts[0], pts[1000]));
        for w in out.windows(2) {
            assert_eq!(w[0].curve.p3, w[1].curve.p0);
        }
    }

    /// Each try of a join is charged to the work as a fit is, before it is made, and where the work runs out the pieces after are
    /// as they were fitted: the joins cost what they are charged (counted at chained sizes), and none is made with none left.
    #[test]
    fn joins_are_charged_to_the_work_and_stop_where_it_runs_out() {
        let mut last = None;
        for n in [100usize, 200, 400, 800] {
            let (pts, pieces) = straight_pieces(n);
            let mut work = usize::MAX / 2;
            crate::work::reset();
            let out = join_pieces(&pts, pieces.clone(), Vec2::new(1.0, 0.0), 1.0, &mut work);
            let charged = crate::work::take(crate::work::Work::Fit);
            assert_eq!(usize::MAX / 2 - work, charged, "{n} pieces: what was spent is what was counted");
            assert_eq!(out.len(), n.div_ceil(MAX_JOINS + 1));
            // A curve of 33 pieces is 33 tries of 2 to 33 points, each at 7 times its points: the same for every 33 pieces.
            if let Some((last_n, last_charged)) = last {
                assert!(
                    charged < 2 * last_charged + last_charged / 4 && charged > last_charged,
                    "{last_n} pieces cost {last_charged}, {n} cost {charged}"
                );
            }
            last = Some((n, charged));
            // With less than the joins cost, the pieces after the work runs out are not joined: more curves, and none made past the work.
            let mut half = charged / 2;
            let cut = join_pieces(&pts, pieces, Vec2::new(1.0, 0.0), 1.0, &mut half);
            assert!(cut.len() > out.len(), "{n} pieces: {} curves with half the work, {} with all of it", cut.len(), out.len());
            assert_eq!(cut.last().map(|p| p.curve.p3), Some(pts[n]), "the line still ends where it did");
        }
        let (pts, pieces) = straight_pieces(50);
        let mut none = 0;
        assert_eq!(join_pieces(&pts, pieces.clone(), Vec2::new(1.0, 0.0), 1.0, &mut none).len(), pieces.len(), "no work, no join");
    }

    /// A curve stays within the tolerance of its points between them, for points far apart (VectorCraft's test of the case where the
    /// tangents alone shape the curve): where this crate gives a fit, every point of the curve is within the tolerance of the lines
    /// between the points; where it does not, it says so (VectorCraft gives a curve that may be out).
    #[test]
    fn a_curve_stays_within_the_tolerance_between_wide_points() {
        use kurbo::{Line, ParamCurveNearest};
        let up = Vec2::new(0.6, 0.8);
        let down = Vec2::new(0.6, -0.8);
        let runs: [&[Point]; 3] = [
            // Two points: the tangents alone shape the curve.
            &[Point::new(0.0, 0.0), Point::new(600.0, 0.0)],
            // Three points: a curve through the middle one can still bulge on each side of it.
            &[Point::new(0.0, 0.0), Point::new(300.0, 0.0), Point::new(600.0, 0.0)],
            &[Point::new(0.0, 0.0), Point::new(100.0, 20.0), Point::new(200.0, 0.0), Point::new(300.0, 20.0), Point::new(400.0, 0.0)],
        ];
        let mut fitted = 0;
        for pts in runs {
            let mut work = usize::MAX;
            let Ok(out) = fit_cubics(pts, up, down, (1.0, 0.0), &mut work) else { continue };
            fitted += 1;
            let mut dense = Vec::new();
            for (c, _) in &out {
                sample(c, 4096, &mut dense, true);
            }
            let far = dense
                .iter()
                .map(|&q| pts.windows(2).map(|w| Line::new(w[0], w[1]).nearest(q, 1e-9).distance_sq.sqrt()).fold(f64::INFINITY, f64::min))
                .fold(0.0, f64::max);
            assert!(far <= 1.0 + 1e-6, "a point of the curve is {far} from the {} points", pts.len());
        }
        // Two points always fit (a handle held to the tolerance); the others may not.
        assert!(fitted >= 1);
    }

    /// Every point is within the tolerance of its chain of cubics, for random walks, noisy loops, short runs, coincident points and
    /// samples of wavy cubics (VectorCraft's 400 cases), and a long serpentine, which split lopsidedly at their worst points, is fitted
    /// without running out of splits. Every case has a fit: a pair of points always does (its handles are held under the tolerance),
    /// and a stretch splits down to pairs.
    #[test]
    fn every_point_is_within_the_tolerance_of_its_chain() {
        let mut rnd = random(0x2545_f491_4f6c_dd1d);
        let mut unfitted = 0;
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
            let mut work = usize::MAX;
            let Ok(fits) = fit_cubics(&pts, t0, t1, (tol, 0.0), &mut work) else {
                unfitted += 1;
                continue;
            };
            let out: Vec<CubicBez> = fits.iter().map(|f| f.0).collect();
            assert_eq!(out.first().map(|c| c.p0), pts.first().copied(), "case {case}");
            assert_eq!(out.last().map(|c| c.p3), pts.last().copied(), "case {case}");
            for w in out.windows(2) {
                assert_eq!(w[0].p3, w[1].p0, "case {case}");
            }
            let far = farthest(&pts, &out);
            assert!(far <= tol + 1e-9, "case {case}: a point is {far} from the fit at a tolerance of {tol}");
        }
        assert_eq!(unfitted, 0, "cases with no fit (a pair of points always fits, and a stretch splits down to pairs)");
        // Samples of a long serpentine: split lopsidedly at their worst points, they are fitted within the splits the depth allows.
        let mut pts = Vec::new();
        for i in 0..1000 {
            let p = Point::new(f64::from(i) * 1.6, if i % 2 == 0 { 0.0 } else { 2.0 });
            let (here, next) = (p, Point::new(f64::from(i + 1) * 1.6, if i % 2 == 0 { 2.0 } else { 0.0 }));
            let c = CubicBez::new(here, here + Vec2::new(0.5, 0.0), next - Vec2::new(0.5, 0.0), next);
            sample(&c, 16, &mut pts, i == 0);
        }
        let mut work = usize::MAX;
        let out = fit_cubics(&pts, Vec2::new(1.0, 0.0), Vec2::new(1.0, 0.0), (1.0, 0.0), &mut work).expect("the serpentine is fitted");
        let out: Vec<CubicBez> = out.iter().map(|f| f.0).collect();
        let far = farthest(&pts, &out);
        assert!(far <= 1.0 + 1e-9, "a point of the serpentine is {far} from the fit");
    }
    /// The search for the direction at a split reads up to the stretch's points, which a fit refused at its first sixteen did not cost:
    /// each is charged to the work before it is made (counted apart as well as in the work), and the work spent is the work counted.
    #[test]
    fn the_search_for_the_direction_at_a_split_is_charged() {
        let mut rnd = random(77);
        let mut p = Point::new(0.0, 0.0);
        let pts: Vec<Point> = (0..600)
            .map(|_| {
                p += Vec2::new(rnd() * 2.0, rnd() * 4.0 - 2.0);
                p
            })
            .collect();
        crate::work::reset();
        let mut work = usize::MAX / 2;
        let fits = fit_cubics(&pts, Vec2::new(1.0, 0.0), Vec2::new(1.0, 0.0), (0.1, 0.0), &mut work).expect("a fit");
        assert!(fits.len() > 20, "{} curves: a walk this rough splits a lot", fits.len());
        let (scans, fitted) = (crate::work::take(crate::work::Work::SplitScans), crate::work::take(crate::work::Work::Fit));
        assert!(scans >= 600, "{scans} points charged for the searches of the splits: the first alone is the 600");
        assert!(scans < fitted, "{scans} of the {fitted} the fit cost");
        assert_eq!(usize::MAX / 2 - work, fitted, "what was spent is what was counted");
        // With work for the fits of the top and not for the search after the first of them, the run stops there: a budget of the
        // first fit's cost and no more is refused at the search.
        crate::work::reset();
        let mut little = 2_000;
        assert_eq!(fit_cubics(&pts, Vec2::new(1.0, 0.0), Vec2::new(1.0, 0.0), (0.1, 0.0), &mut little), Err(FitStop::Work));
        assert!(crate::work::take(crate::work::Work::Fit) <= 2_000);
    }
    /// A split is at the worst point where that is in the middle fifth, and is moved to two fifths of the stretch from the end where it is nearer.
    #[test]
    fn a_split_keeps_two_fifths_of_the_stretch_each_side() {
        // 1,000 points: the worst point at either end is moved to the 400th from it, anywhere between stays.
        assert_eq!((split_index(0, 1000), split_index(1, 1000), split_index(399, 1000), split_index(400, 1000)), (400, 400, 400, 400));
        assert_eq!((split_index(450, 1000), split_index(599, 1000), split_index(600, 1000), split_index(999, 1000)), (450, 599, 599, 599));
        // 10 points: [4, 5]. Small stretches have the one or two inner points.
        assert_eq!((0..10).map(|w| split_index(w, 10)).collect::<Vec<_>>(), [4, 4, 4, 4, 4, 5, 5, 5, 5, 5]);
        assert_eq!((0..5).map(|w| split_index(w, 5)).collect::<Vec<_>>(), [2, 2, 2, 2, 2]);
        assert_eq!((0..4).map(|w| split_index(w, 4)).collect::<Vec<_>>(), [1, 1, 2, 2]);
        assert_eq!((0..3).map(|w| split_index(w, 3)).collect::<Vec<_>>(), [1, 1, 1]);
        // A pair is never split (it is a cubic), but the function says where it would be: the one point between, and for fewer, the middle.
        assert_eq!((split_index(0, 2), split_index(1, 2), split_index(0, 1), split_index(0, 0)), (1, 1, 0, 0));
        // The split is always an inner point, so that both halves are shorter than the stretch.
        for n in 3..200 {
            for w in 0..n {
                let k = split_index(w, n);
                assert!(k >= 1 && k <= n - 2, "{n} points, worst {w}: split at {k}");
                assert!(k.max(n - 1 - k) * 5 <= n * 3 + 8, "{n} points, split at {k}: a half holds more than three fifths");
            }
        }
    }

    /// A walk whose worst point is always near an end is split in the logarithm of its points: a million points in about 27 splits, which a
    /// split at the worst point (peeling off a few points) did not manage in 40. At the depth that logarithm allows it is fitted.
    #[test]
    fn a_long_serpentine_is_fitted_in_the_depth_the_logarithm_allows() {
        let mut pts = Vec::new();
        for i in 0..2_000 {
            let p = Point::new(f64::from(i) * 1.6, if i % 2 == 0 { 0.0 } else { 2.0 });
            let next = Point::new(f64::from(i + 1) * 1.6, if i % 2 == 0 { 2.0 } else { 0.0 });
            let c = CubicBez::new(p, p + Vec2::new(0.5, 0.0), next - Vec2::new(0.5, 0.0), next);
            sample(&c, 10, &mut pts, i == 0);
        }
        // 20,001 points: log of them to base 5/3, with some to spare for the halves that are not a fifth either side.
        let depth = ((pts.len() as f64).ln() / (5.0f64 / 3.0).ln()).ceil() as usize + 2;
        assert!(depth < 30, "{depth}");
        let mut work = usize::MAX;
        let out = fit_cubics_to_depth(&pts, Vec2::new(1.0, 0.0), Vec2::new(1.0, 0.0), (1.0, 0.0), &mut work, depth).expect("fitted within the depth");
        assert!(out.len() > 50);
    }
    /// A pair of points is a cubic through both whose handles are a third of the chord along the tangents, held under the tolerance: the
    /// control points are where they are said to be (the second handle arrives along the tangent, behind the end).
    #[test]
    fn a_pair_of_points_is_a_cubic_with_handles_of_a_third_of_the_chord() {
        let (a, b) = (Point::new(0.0, 0.0), Point::new(3.0, 0.0));
        let up = Vec2::new(0.0, 1.0);
        let mut work = usize::MAX;
        let out = fit_cubics(&[a, b], up, up, (5.0, 0.0), &mut work).expect("a pair fits");
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].0, CubicBez::new(a, Point::new(0.0, 1.0), Point::new(3.0, -1.0), b));
        assert!((out[0].1 - 1.0).abs() < 1e-12, "its bound is the distance of its handles from the chord: {}", out[0].1);
        // A long chord: the handles are held a hair under the tolerance (0.1 here), whatever the third of the chord is.
        let far = Point::new(300.0, 0.0);
        let out = fit_cubics(&[a, far], up, up, (0.1, 0.0), &mut work).expect("a pair fits");
        let handle = out[0].0.p1 - a;
        assert!(handle.hypot() < 0.1 && handle.hypot() > 0.1 * (1.0 - 1e-6), "{handle:?}");
        assert!(out[0].1 <= 0.1, "{}", out[0].1);
    }

    /// The pieces `fit_rec` makes (before any are joined) follow each other from the first point to the last, each ending at the point the
    /// next starts from, and their ends increase.
    #[test]
    fn the_pieces_of_a_fit_follow_each_other_from_end_to_end() {
        let mut rnd = random(404);
        let mut p = Point::new(0.0, 0.0);
        let pts: Vec<Point> = (0..500)
            .map(|_| {
                p += Vec2::new(rnd() * 2.0, rnd() * 4.0 - 2.0);
                p
            })
            .collect();
        let mut out = Vec::new();
        let mut work = usize::MAX;
        let (t0, t1) = (Vec2::new(1.0, 0.0), Vec2::new(1.0, 0.0));
        fit_rec(&pts, (0, pts.len() - 1), (t0, t1), 0.1, &mut out, (0, MAX_FIT_DEPTH), &mut work).expect("a fit");
        assert!(out.len() > 30, "{} pieces", out.len());
        assert_eq!(out[0].curve.p0, pts[0]);
        assert_eq!(out[out.len() - 1].curve.p3, pts[pts.len() - 1]);
        assert_eq!(out[out.len() - 1].end, pts.len() - 1);
        for w in out.windows(2) {
            assert_eq!(w[0].curve.p3, w[1].curve.p0, "a piece starts where the one before ends");
            assert!(w[0].end < w[1].end);
            assert_eq!(pts[w[0].end], w[0].curve.p3, "a piece ends at the point its end names");
        }
    }

    /// The direction at a split is between the nearest points on each side that are over the tolerance from it (a point at exactly the
    /// tolerance is not over it), or the first and last points of the stretch where none is.
    #[test]
    fn the_direction_at_a_split_is_between_the_nearest_points_over_the_tolerance() {
        let p = |x: f64, y: f64| Point::new(x, y);
        // At exactly the tolerance (1) the points beside the split are not over it: the far ones are taken.
        let pts = [p(-5.0, 5.0), p(-1.0, 0.0), p(0.0, 0.0), p(1.0, 0.0), p(5.0, -5.0)];
        let d = center_tangent(&pts, 2, 1.0).expect("a direction");
        assert!(d.dot(Vec2::new(1.0, -1.0).normalize()) > 0.999_999, "{d:?}");
        // Over the tolerance, they are the nearest ones; the point after is not the one before, nor the split's own.
        let pts = [p(-5.0, 5.0), p(-2.0, 0.0), p(0.0, 0.0), p(2.0, 0.0), p(5.0, -5.0)];
        let d = center_tangent(&pts, 2, 1.0).expect("a direction");
        assert!(d.dot(Vec2::new(1.0, 0.0)) > 0.999_999, "{d:?}");
        // None over the tolerance: the ends of the stretch.
        let pts = [p(-0.3, 0.2), p(-0.1, 0.0), p(0.0, 0.0), p(0.1, 0.0), p(0.3, -0.2)];
        let d = center_tangent(&pts, 2, 1.0).expect("a direction");
        assert!(d.dot(Vec2::new(0.6, -0.4).normalize()) > 0.999_999, "{d:?}");
    }
    /// A zigzag of `n` points, 1 across and `height` up and down: no cubic follows it.
    fn zigzag_points(n: usize, height: f64) -> Vec<Point> {
        (0..n).map(|i| Point::new(i as f64, if i % 2 == 0 { 0.0 } else { height })).collect()
    }

    /// What a try of a fit is charged: a stretch of 32 points or more is first fitted to about 16 of them (every 62nd and the last, of a
    /// thousand: 18, seven passes each), and split at once where that is over the tolerance, for that price and not the stretch's; a fit that is
    /// over 8 tolerances and 5% of the chord off is not reparameterized (one pass over the points), and one that is not is (seven).
    #[test]
    fn a_try_of_a_fit_is_charged_the_coarse_fit_and_gives_up_on_a_far_fit() {
        let (t0, t1) = (Vec2::new(1.0, 0.0), Vec2::new(1.0, 0.0));
        let tries = |pts: &[Point], tol: f64| {
            let mut work = usize::MAX;
            let got = try_fit(pts, (t0, t1), tol, &mut paying(&mut work));
            (got, usize::MAX - work)
        };
        // 1,000 points: the first fit of 18 of them (every 62nd and the last) is over the tolerance but not 8 of it and 5% of the chord (50): it
        // is reparameterized, and the try is over, split at one of them, for 18 times seven and not a thousand times.
        let (got, charged) = tries(&zigzag_points(1000, 2.0), 0.1);
        let Err(Unfit::At(split)) = got else { panic!("the zigzag does not fit one cubic") };
        assert_eq!(charged, 18 * 7);
        assert!(split > 0 && (split % 62 == 0 || split == 999), "split at {split}, a place of the coarse fit and not the first");
        // The same 18 points far off (200 up and down: over 8 tolerances and 5% of the chord) are given up on after the first pass.
        let (got, charged) = tries(&zigzag_points(1000, 200.0), 0.1);
        assert!(matches!(got, Err(Unfit::At(_))));
        assert_eq!(charged, 18, "no reparameterization of a fit that is that far off");
        // The line between: a fit off by 5 is past 8 tolerances and 5% of the chord (0.8 and 1.5, 2.3) and not past the 9.6 that adding 8 to the
        // tolerance would make: given up on (the tolerance is multiplied by 8, not added to it).
        let (got, charged) = tries(&zigzag_points(31, 5.0), 0.1);
        assert!(matches!(got, Err(Unfit::At(_))));
        assert_eq!(charged, 31, "given up on");
        // Under 32 points there is no coarse fit: one pass over 31 far points, seven over 31 that are not far.
        let (got, charged) = tries(&zigzag_points(31, 200.0), 0.1);
        assert!(matches!(got, Err(Unfit::At(_))));
        assert_eq!(charged, 31);
        let (got, charged) = tries(&zigzag_points(31, 2.0), 0.1);
        assert!(matches!(got, Err(Unfit::At(_))));
        assert_eq!(charged, 31 * 7, "within 8 tolerances and 5% of the chord (2.3): reparameterized, and no fit");
        let arc: Vec<Point> = (0..31).map(|i| Point::new(f64::from(i), 0.01 * f64::from(i) * f64::from(30 - i))).collect();
        let mut work = usize::MAX;
        let (up, down) = (Vec2::new(1.0, 0.3).normalize(), Vec2::new(1.0, -0.3).normalize());
        let got = try_fit(&arc, (up, down), 0.5, &mut paying(&mut work));
        assert!(got.is_ok(), "the arc fits");
        assert_eq!(usize::MAX - work, 31 * 7);
        // At exactly 32 points the coarse fit is made (17 points: seven passes) and passes on a straight run, and then the whole.
        let line: Vec<Point> = (0..32).map(|i| Point::new(f64::from(i), 0.0)).collect();
        let (got, charged) = tries(&line, 0.5);
        assert!(got.is_ok());
        assert_eq!(charged, 17 * 7 + 32 * 7);
        let (_, charged) = tries(&line[..31], 0.5);
        assert_eq!(charged, 31 * 7, "no coarse fit under 32");
    }
    /// A stretch whose fit needs more splits than the depth allows is refused at that depth ([`FitStop::Depth`]) and fitted at one that is
    /// enough: the depth counts the splits on the way down, one each.
    #[test]
    fn a_fit_that_needs_more_splits_than_the_depth_allows_is_refused_at_that_depth() {
        let pts = zigzag_points(200, 5.0);
        let (t0, t1) = (Vec2::new(1.0, 0.0), Vec2::new(1.0, 0.0));
        let mut work = usize::MAX;
        for depth in [1, 2, 3, 4] {
            assert_eq!(fit_cubics_to_depth(&pts, t0, t1, (0.1, 0.0), &mut work, depth), Err(FitStop::Depth), "depth {depth}");
        }
        assert!(fit_cubics_to_depth(&pts, t0, t1, (0.1, 0.0), &mut work, MAX_FIT_DEPTH).is_ok());
        // Each half counts, the one the splitting goes down and not the other: four zigzag points at the start of a long straight line need the
        // splits down the left half only (the right halves are lines), and at the end of one down the right half only.
        let zig_then_line: Vec<Point> = zigzag_points(4, 5.0).into_iter().chain((4..404).map(|i| Point::new(f64::from(i), 0.0))).collect();
        let line_then_zig: Vec<Point> =
            (0..400).map(|i| Point::new(f64::from(i), 0.0)).chain(zigzag_points(4, 5.0).into_iter().map(|p| Point::new(p.x + 400.0, p.y))).collect();
        for (name, pts) in [("a short zigzag then a line", &zig_then_line), ("a line then a short zigzag", &line_then_zig)] {
            assert_eq!(fit_cubics_to_depth(pts, t0, t1, (0.1, 0.0), &mut work, 3), Err(FitStop::Depth), "{name}");
            assert!(fit_cubics_to_depth(pts, t0, t1, (0.1, 0.0), &mut work, MAX_FIT_DEPTH).is_ok(), "{name}");
        }
    }
}
