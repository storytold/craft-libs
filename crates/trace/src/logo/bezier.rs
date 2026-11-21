//! Least-squares cubic Bézier fitting (Schneider's algorithm): fit one cubic to a run of points,
//! improve its parametrisation, and split where the error is largest until every piece is within
//! the tolerance. After P. J. Schneider, "An algorithm for automatically fitting digitized curves"
//! (Graphics Gems, 1990), written from the description there.

use super::geom::{Cubic, bez};
use craft_geom::Point;
use kurbo::{CubicBez, ParamCurveNearest, Vec2};

/// From this depth a run still over the tolerance is cut in the middle, which bounds the recursion
/// at this depth plus the logarithm of the run's length.
const MAX_DEPTH: usize = 14;

fn chord(p: &[Point]) -> Vec<f64> {
    let mut d = vec![0.0];
    for w in p.windows(2) {
        d.push(d.last().copied().unwrap_or(0.0) + w[0].distance(w[1]));
    }
    let total = d.last().copied().unwrap_or(0.0);
    if total > 0.0 {
        d.iter_mut().for_each(|v| *v /= total);
    }
    d
}

fn fit_one(p: &[Point], u: &[f64], t1: Vec2, t2: Vec2) -> Option<Cubic> {
    let (p0, p3) = (*p.first()?, *p.last()?);
    let (mut c00, mut c01, mut c11, mut x0, mut x1) = (0.0, 0.0, 0.0, 0.0, 0.0);
    for (q, &t) in p.iter().zip(u) {
        let s = 1.0 - t;
        let (b0, b1, b2, b3) = (s * s * s, 3.0 * t * s * s, 3.0 * t * t * s, t * t * t);
        let (a1, a2) = (t1 * b1, t2 * b2);
        c00 += a1.dot(a1);
        c01 += a1.dot(a2);
        c11 += a2.dot(a2);
        let tmp = *q - (p0.to_vec2() * (b0 + b1) + p3.to_vec2() * (b2 + b3)).to_point();
        x0 += a1.dot(tmp);
        x1 += a2.dot(tmp);
    }
    let det = c00 * c11 - c01 * c01;
    let seg = p0.distance(p3);
    let (mut al, mut ar) = if det.abs() > 1e-12 { ((x0 * c11 - x1 * c01) / det, (c00 * x1 - c01 * x0) / det) } else { (0.0, 0.0) };
    if !al.is_finite() || !ar.is_finite() || al <= 1e-6 * seg || ar <= 1e-6 * seg {
        al = seg / 3.0;
        ar = seg / 3.0;
    }
    Some([p0, p0 + t1 * al, p3 + t2 * ar, p3])
}

fn reparam(c: &Cubic, p: &[Point], u: &[f64]) -> Vec<f64> {
    p.iter()
        .zip(u)
        .map(|(q, &t)| {
            let s = 1.0 - t;
            let d = bez(c, t) - *q;
            let (v0, v1, v2) = (c[1] - c[0], c[2] - c[1], c[3] - c[2]);
            let d1 = (v0 * (s * s) + v1 * (2.0 * s * t) + v2 * (t * t)) * 3.0;
            let d2 = ((v1 - v0) * s + (v2 - v1) * t) * 6.0;
            let den = d1.dot(d1) + d.dot(d2);
            if den.abs() > 1e-12 { (t - d.dot(d1) / den).clamp(0.0, 1.0) } else { t }
        })
        .collect()
}

fn max_err(c: &Cubic, p: &[Point], u: &[f64]) -> (f64, usize) {
    let mut best = (0.0, p.len() / 2);
    for (i, (q, &t)) in p.iter().zip(u).enumerate() {
        let e = bez(c, t).distance(*q);
        if e > best.0 {
            best = (e, i);
        }
    }
    best
}

/// The largest distance from a point of `p` to the nearest point of `c`, which `max_err`'s distance
/// at a point's parameter can only overstate.
fn max_distance(c: &Cubic, p: &[Point]) -> f64 {
    let curve = CubicBez::new(c[0], c[1], c[2], c[3]);
    p.iter().map(|&q| curve.nearest(q, 1e-6).distance_sq).fold(0.0, f64::max).sqrt()
}

fn rec(p: &[Point], t1: Vec2, t2: Vec2, tol: f64, depth: usize, out: &mut Vec<Cubic>) {
    let (Some(&a), Some(&b)) = (p.first(), p.last()) else { return };
    if p.len() == 2 {
        let d = a.distance(b) / 3.0;
        out.push([a, a + t1 * d, b + t2 * d, b]);
        return;
    }
    let mut u = chord(p);
    let Some(mut c) = fit_one(p, &u, t1, t2) else { return };
    for _ in 0..4 {
        let (e, _) = max_err(&c, p, &u);
        if e < tol {
            out.push(c);
            return;
        }
        if e > tol * 6.0 {
            break;
        }
        u = reparam(&c, p, &u);
        match fit_one(p, &u, t1, t2) {
            Some(n) => c = n,
            None => break,
        }
    }
    let (e, split) = max_err(&c, p, &u);
    let short = p.len() < 6;
    let capped = depth >= MAX_DEPTH;
    if e < tol || ((short || capped) && max_distance(&c, p) <= tol) {
        out.push(c);
        return;
    }
    let s = if short {
        split.clamp(1, p.len() - 2)
    } else if capped {
        p.len() / 2
    } else {
        split.clamp(2, p.len() - 3)
    };
    let tc = match (p.get(s - 1), p.get(s + 1)) {
        (Some(&x), Some(&y)) if x.distance(y) > 1e-12 => (x - y) / x.distance(y),
        _ => t1,
    };
    rec(p.get(..=s).unwrap_or(p), t1, tc, tol, depth + 1, out);
    rec(p.get(s..).unwrap_or(p), -tc, t2, tol, depth + 1, out);
}

/// Cubics through `p` (first and last points are interpolated) with end tangents `t1` (pointing
/// along the run at its start) and `t2` (pointing back along the run at its end), each within
/// `tol` of the points.
pub(crate) fn fit_cubics(p: &[Point], t1: Vec2, t2: Vec2, tol: f64) -> Vec<Cubic> {
    let mut out = Vec::new();
    if p.len() >= 2 && tol > 0.0 && p.iter().all(|q| q.x.is_finite() && q.y.is_finite()) {
        rec(p, t1, t2, tol, 0, &mut out);
    }
    out
}

/// Unit direction from `p[i]` towards `p[i + k]` (`sign > 0`) or `p[i - k]` (`sign < 0`).
pub(crate) fn tangent(p: &[Point], i: usize, k: usize, sign: i32) -> Vec2 {
    let j = if sign > 0 { (i + k).min(p.len().saturating_sub(1)) } else { i.saturating_sub(k) };
    match (p.get(i), p.get(j)) {
        (Some(&a), Some(&b)) if a.distance(b) > 1e-12 => (b - a) / a.distance(b),
        _ => Vec2::new(1.0, 0.0),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::logo::geom::sample_cubics;

    fn dev(cs: &[Cubic], pts: &[Point]) -> f64 {
        let s = sample_cubics(cs, 0.01);
        pts.iter().map(|p| s.iter().map(|q| q.distance(*p)).fold(f64::MAX, f64::min)).fold(0.0, f64::max)
    }

    #[test]
    fn a_circle_arc_is_one_cubic() {
        let pts: Vec<Point> = (0..200).map(|i| Point::new(20.0 * (i as f64 * 0.006).cos(), 20.0 * (i as f64 * 0.006).sin())).collect();
        let cs = fit_cubics(&pts, tangent(&pts, 0, 4, 1), tangent(&pts, 199, 4, -1), 0.05);
        assert_eq!(cs.len(), 1);
        assert!(dev(&cs, &pts) < 0.05);
    }

    #[test]
    fn a_full_circle_stays_within_tolerance_with_few_pieces() {
        let n = 1200;
        let mut pts: Vec<Point> = (0..=n)
            .map(|i| {
                Point::new(
                    50.0 + 20.0 * (i as f64 / n as f64 * std::f64::consts::TAU).cos(),
                    50.0 + 20.0 * (i as f64 / n as f64 * std::f64::consts::TAU).sin(),
                )
            })
            .collect();
        pts[n] = pts[0];
        let t = (pts[1] - pts[n - 1]) / pts[1].distance(pts[n - 1]);
        let cs = fit_cubics(&pts, t, -t, 0.05);
        assert!(cs.len() <= 12, "{} pieces", cs.len());
        assert!(dev(&cs, &pts) < 0.06, "deviation {}", dev(&cs, &pts));
    }

    #[test]
    fn a_wavy_curve_is_fitted_within_tolerance() {
        let pts: Vec<Point> = (0..600).map(|i| Point::new(i as f64 * 0.05, 3.0 * (i as f64 * 0.02).sin())).collect();
        let cs = fit_cubics(&pts, tangent(&pts, 0, 4, 1), tangent(&pts, 599, 4, -1), 0.04);
        assert!(dev(&cs, &pts) < 0.05);
        assert!(cs.len() < 20);
    }

    #[test]
    fn pieces_join_end_to_end() {
        let pts: Vec<Point> = (0..400).map(|i| Point::new(i as f64 * 0.1, (i as f64 * 0.05).sin() * 4.0)).collect();
        let cs = fit_cubics(&pts, tangent(&pts, 0, 4, 1), tangent(&pts, 399, 4, -1), 0.02);
        for w in cs.windows(2) {
            assert!(w[0][3].distance(w[1][0]) < 1e-9);
        }
    }

    /// The largest distance from a point of `pts` to the nearest point of the fit.
    fn exact_dev(cs: &[Cubic], pts: &[Point]) -> f64 {
        pts.iter().map(|&q| cs.iter().map(|c| max_distance(c, &[q])).fold(f64::MAX, f64::min)).fold(0.0, f64::max)
    }

    /// Fits `pts` and checks that the pieces run from its first point to its last, each within `tol`.
    fn assert_within(pts: &[Point], tol: f64) -> Vec<Cubic> {
        let n = pts.len();
        let cs = fit_cubics(pts, tangent(pts, 0, 4, 1), tangent(pts, n - 1, 4, -1), tol);
        assert!(exact_dev(&cs, pts) <= tol, "{} points, tolerance {tol}: deviation {}", n, exact_dev(&cs, pts));
        assert_eq!((cs[0][0], cs[cs.len() - 1][3]), (pts[0], pts[n - 1]));
        for w in cs.windows(2) {
            assert!(w[0][3].distance(w[1][0]) < 1e-9);
        }
        cs
    }

    #[test]
    fn a_short_run_over_the_tolerance_is_cut_until_it_is_within() {
        let spike = [(0.0, 0.0), (0.12, 0.0), (0.24, 0.5), (0.36, 0.0), (0.48, 0.0)].map(|(x, y)| Point::new(x, y));
        assert!(assert_within(&spike, 0.1).len() > 1);
        let four = [(0.0, 0.0), (0.2, 0.6), (0.4, -0.6), (0.6, 0.0)].map(|(x, y)| Point::new(x, y));
        assert!(assert_within(&four, 0.1).len() > 1);
        let three = [(0.0, 0.0), (0.2, 0.6), (0.4, 0.0)].map(|(x, y)| Point::new(x, y));
        assert!(assert_within(&three, 0.1).len() > 1);
    }

    /// A zigzag that grows towards its end: the largest error is always next to the end, so each
    /// split takes only a few points off the run.
    fn growing_zigzag(n: usize) -> Vec<Point> {
        (0..n).map(|i| Point::new(i as f64 * 0.1, if i % 2 == 0 { 1.0 } else { -1.0 } * 0.02 * 1.02f64.powi(i as i32))).collect()
    }

    #[test]
    fn a_run_that_reaches_the_depth_limit_is_still_cut_until_it_is_within() {
        assert_within(&growing_zigzag(300), 0.05);
    }

    #[test]
    fn a_very_long_run_of_small_splits_is_fitted_without_deep_recursion() {
        let pts: Vec<Point> =
            (0..200_000).map(|i| Point::new(i as f64 * 0.1, if i % 2 == 0 { 1.0 } else { -1.0 } * (1.0 + i as f64 * 1e-3))).collect();
        let cs = fit_cubics(&pts, tangent(&pts, 0, 4, 1), tangent(&pts, pts.len() - 1, 4, -1), 0.05);
        assert_eq!((cs[0][0], cs[cs.len() - 1][3]), (pts[0], pts[pts.len() - 1]));
    }

    #[test]
    fn random_runs_stay_within_the_tolerance() {
        let mut state = 0x9e37_79b9_7f4a_7c15u64;
        let mut next = move || {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            state
        };
        for k in 0..300 {
            let n = 3 + (next() % 200) as usize;
            let tol = [0.04, 0.08, 0.14, 0.3, 0.8][(next() % 5) as usize];
            let jitter = [0.02, 0.1, 0.5, 2.0][k % 4];
            let (mut x, mut y, mut a) = (0.0f64, 0.0f64, 0.0f64);
            let mut pts = Vec::with_capacity(n);
            for _ in 0..n {
                a += ((next() % 2001) as f64 / 1000.0 - 1.0) * [0.05, 0.4, 1.5][k % 3];
                x += a.cos() * 0.12;
                y += a.sin() * 0.12 + ((next() % 2001) as f64 / 1000.0 - 1.0) * jitter;
                pts.push(Point::new(x, y));
            }
            assert_within(&pts, tol);
        }
    }

    #[test]
    fn degenerate_input_gives_nothing_or_a_line_not_a_panic() {
        let t = Vec2::new(1.0, 0.0);
        assert!(fit_cubics(&[], t, t, 0.1).is_empty());
        assert!(fit_cubics(&[Point::new(0.0, 0.0)], t, t, 0.1).is_empty());
        assert_eq!(fit_cubics(&[Point::new(0.0, 0.0), Point::new(3.0, 0.0)], t, -t, 0.1).len(), 1);
        assert!(fit_cubics(&[Point::new(f64::NAN, 0.0), Point::new(1.0, 1.0), Point::new(2.0, 0.0)], t, t, 0.1).is_empty());
        assert!(fit_cubics(&[Point::new(0.0, 0.0), Point::new(1.0, 0.0), Point::new(2.0, 0.0)], t, t, 0.0).is_empty());
        // Duplicate points do not divide by zero.
        let _ = fit_cubics(&[Point::new(1.0, 1.0); 10], t, t, 0.1);
    }
}
