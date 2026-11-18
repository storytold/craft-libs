//! Geometry assertions.

use craft_geom::{BezPath, ParamCurve, PathData, Point, Rect};
use kurbo::{ParamCurveNearest, Shape};

pub fn approx(a: f64, b: f64, eps: f64) -> bool {
    (a - b).abs() <= eps
}

/// Relative closeness with an absolute floor of 1.
pub fn rel_close(a: f64, b: f64, rel: f64) -> bool {
    (a - b).abs() <= rel * a.abs().max(b.abs()).max(1.0)
}

#[track_caller]
pub fn assert_approx(a: f64, b: f64, eps: f64) {
    assert!(approx(a, b, eps), "{a} != {b} (±{eps})");
}

#[track_caller]
pub fn assert_rect_approx(a: Rect, b: Rect, eps: f64) {
    assert!(approx(a.x0, b.x0, eps) && approx(a.y0, b.y0, eps) && approx(a.x1, b.x1, eps) && approx(a.y1, b.y1, eps), "{a:?} != {b:?} (±{eps})");
}

/// Does `outer` contain `inner` (with slack `eps`)?
pub fn rect_contains(outer: Rect, inner: Rect, eps: f64) -> bool {
    inner.x0 >= outer.x0 - eps && inner.y0 >= outer.y0 - eps && inner.x1 <= outer.x1 + eps && inner.y1 <= outer.y1 + eps
}

/// `n` evenly spaced parameter samples on every segment (endpoints included).
pub fn sample(bp: &BezPath, n: usize) -> Vec<Point> {
    let mut v = vec![];
    for seg in bp.segments() {
        for i in 0..=n {
            v.push(seg.eval(i as f64 / n as f64));
        }
    }
    v
}

/// Sample a `PathData`.
pub fn sample_path(p: &PathData, n: usize) -> Vec<Point> {
    sample(&p.to_bezpath(), n)
}

/// Distance from `p` to the nearest point of `bp`'s outline.
pub fn distance_to(bp: &BezPath, p: Point) -> f64 {
    bp.segments().map(|s| s.nearest(p, 1e-9).distance_sq).fold(f64::INFINITY, f64::min).sqrt()
}

/// One-sided Hausdorff distance: the largest distance from a sample of `a` to the outline of `b`.
pub fn directed_hausdorff(a: &BezPath, b: &BezPath, n: usize) -> f64 {
    sample(a, n).into_iter().map(|p| distance_to(b, p)).fold(0.0, f64::max)
}

/// Symmetric Hausdorff distance (sampled).
pub fn hausdorff(a: &BezPath, b: &BezPath, n: usize) -> f64 {
    directed_hausdorff(a, b, n).max(directed_hausdorff(b, a, n))
}

/// Assert that two paths trace the same outline within `tol`.
#[track_caller]
pub fn assert_paths_close(a: &PathData, b: &PathData, tol: f64) {
    let d = hausdorff(&a.to_bezpath(), &b.to_bezpath(), 16);
    assert!(d <= tol, "paths differ by {d} > {tol}");
}

/// Shoelace area of a polygon (absolute).
pub fn polygon_area(pts: &[Point]) -> f64 {
    let n = pts.len();
    (0..n).map(|i| pts[i].to_vec2().cross(pts[(i + 1) % n].to_vec2())).sum::<f64>().abs() / 2.0
}

/// Signed area enclosed by a path (kurbo's Green's theorem area, all subpaths).
pub fn signed_area(p: &PathData) -> f64 {
    p.to_bezpath().area()
}

/// Is every point finite?
pub fn all_finite(p: &PathData) -> bool {
    p.subpaths.iter().flat_map(|s| &s.anchors).all(|a| [a.p, a.h_in, a.h_out].iter().all(|q| q.x.is_finite() && q.y.is_finite()))
}

/// A grid of probe points over `r` (`n`×`n`, cell centres).
pub fn grid(r: Rect, n: usize) -> Vec<Point> {
    let mut v = vec![];
    for i in 0..n {
        for j in 0..n {
            v.push(Point::new(r.x0 + (i as f64 + 0.5) / n as f64 * r.width(), r.y0 + (j as f64 + 0.5) / n as f64 * r.height()));
        }
    }
    v
}
