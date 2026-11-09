// Carved from VectorCraft's crates/pathops (tests/common/mod.rs, and the geometry helpers and proptest strategies of
// vectorcraft-testkit its suites used) at storytold/vectorcraft ff9deee46d1c59dc9ecef0b668c25e57cd80b55e.
// Copyright (c) 2026 ArtCraft Team and the VectorCraft contributors. MIT OR Apache-2.0.

//! Shared helpers for the pathops suites: the property-test configuration, the shapes the suites
//! draw, and the measures they check results with.
#![allow(dead_code)]

use kurbo::{BezPath, ParamCurve, ParamCurveNearest, PathEl, Point, Rect, Shape as _};
use proptest::prelude::*;
use proptest::test_runner::{Config, RngSeed};

/// Seed the property suites run with unless `PROPTEST_RNG_SEED` is set.
const SEED: u64 = 0x5eed_9a7f;

/// Reproducible property-test config: `cases` cases from a fixed seed, so a CI run can never fail
/// at random. Explore further locally with `PROPTEST_CASES=5000 PROPTEST_RNG_SEED=<n>`; keep
/// every minimal failure found that way as a test in `regressions.rs`.
pub fn config(cases: u32) -> Config {
    let mut c = Config { failure_persistence: None, ..Config::default() };
    if std::env::var_os("PROPTEST_CASES").is_none() {
        c.cases = cases;
    }
    if std::env::var_os("PROPTEST_RNG_SEED").is_none() {
        c.rng_seed = RngSeed::Fixed(SEED);
    }
    c
}

// ---------------------------------------------------------------- paths

/// The control-point distance of a quarter circle's cubic, as a fraction of the radius.
const KAPPA: f64 = 0.552_284_749_830_793_6;

/// A polyline through `points`, closed or open. A closed one ends with a segment back to its start.
pub fn polyline(points: &[Point], closed: bool) -> BezPath {
    let mut bp = BezPath::new();
    for (i, &p) in points.iter().enumerate() {
        if i == 0 {
            bp.move_to(p);
        } else {
            bp.line_to(p);
        }
    }
    if closed {
        if let Some(&first) = points.first() {
            bp.line_to(first);
        }
        bp.close_path();
    }
    bp
}

/// A polyline through `(x, y)` pairs.
pub fn poly(points: &[(f64, f64)], closed: bool) -> BezPath {
    polyline(&points.iter().map(|&p| Point::from(p)).collect::<Vec<_>>(), closed)
}

/// The rectangle `r`.
pub fn rectangle(r: Rect) -> BezPath {
    let r = r.abs();
    polyline(&[Point::new(r.x0, r.y0), Point::new(r.x1, r.y0), Point::new(r.x1, r.y1), Point::new(r.x0, r.y1)], true)
}

/// The ellipse inscribed in `r`: four cubics from the left, top, right and bottom points.
pub fn ellipse(r: Rect) -> BezPath {
    let r = r.abs();
    let c = r.center();
    let (rx, ry) = (r.width() / 2.0, r.height() / 2.0);
    let (kx, ky) = (rx * KAPPA, ry * KAPPA);
    let mut bp = BezPath::new();
    bp.move_to((c.x - rx, c.y));
    bp.curve_to((c.x - rx, c.y - ky), (c.x - kx, c.y - ry), (c.x, c.y - ry));
    bp.curve_to((c.x + kx, c.y - ry), (c.x + rx, c.y - ky), (c.x + rx, c.y));
    bp.curve_to((c.x + rx, c.y + ky), (c.x + kx, c.y + ry), (c.x, c.y + ry));
    bp.curve_to((c.x - kx, c.y + ry), (c.x - rx, c.y + ky), (c.x - rx, c.y));
    bp.close_path();
    bp
}

/// A straight open line.
pub fn line(a: Point, b: Point) -> BezPath {
    polyline(&[a, b], false)
}

/// A closed path through anchors `[x, y, in_x, in_y, out_x, out_y]`: each segment leaves its anchor
/// by the out handle and arrives by the next anchor's in handle, and is a line where neither is
/// away from its anchor.
pub fn curved(anchors: &[[f64; 6]]) -> BezPath {
    let mut bp = BezPath::new();
    let at = |a: &[f64; 6]| Point::new(a[0], a[1]);
    let (h_in, h_out) = (|a: &[f64; 6]| Point::new(a[2], a[3]), |a: &[f64; 6]| Point::new(a[4], a[5]));
    if let Some(first) = anchors.first() {
        bp.move_to(at(first));
    }
    for (i, a) in anchors.iter().enumerate() {
        let b = &anchors[(i + 1) % anchors.len()];
        if h_out(a).distance(at(a)) <= 1e-9 && h_in(b).distance(at(b)) <= 1e-9 {
            bp.line_to(at(b));
        } else {
            bp.curve_to(h_out(a), h_in(b), at(b));
        }
    }
    bp.close_path();
    bp
}

/// The subpaths of `path` as element ranges, each with whether it was closed.
pub fn subpath_ranges(path: &BezPath) -> Vec<(std::ops::Range<usize>, bool)> {
    let els = path.elements();
    let mut out = vec![];
    let mut start = 0;
    for (i, el) in els.iter().enumerate() {
        match el {
            PathEl::MoveTo(_) if i > start => {
                out.push((start..i, false));
                start = i;
            }
            PathEl::ClosePath => {
                out.push((start..i + 1, true));
                start = i + 1;
            }
            _ => {}
        }
    }
    if start < els.len() {
        out.push((start..els.len(), false));
    }
    out.retain(|(r, _)| r.len() > 1);
    out
}

/// Each subpath of `path` as a path of its own.
pub fn split(path: &BezPath) -> Vec<BezPath> {
    subpath_ranges(path).into_iter().map(|(r, _)| BezPath::from_vec(path.elements()[r].to_vec())).collect()
}

/// The number of subpaths.
pub fn subpath_count(path: &BezPath) -> usize {
    subpath_ranges(path).len()
}

/// Whether every subpath is closed.
pub fn all_closed(path: &BezPath) -> bool {
    subpath_ranges(path).iter().all(|(_, closed)| *closed)
}

/// Whether any subpath is closed.
pub fn any_closed(path: &BezPath) -> bool {
    subpath_ranges(path).iter().any(|(_, closed)| *closed)
}

/// The number of anchors: the points the subpaths pass through, a closed subpath's start counted once.
pub fn anchor_count(path: &BezPath) -> usize {
    let els = path.elements();
    subpath_ranges(path)
        .into_iter()
        .map(|(r, closed)| {
            let draws = els[r.clone()].iter().filter(|e| !matches!(e, PathEl::MoveTo(_) | PathEl::ClosePath)).count();
            let (start, end) = (els[r.start].end_point(), els[r.clone()].iter().rev().find_map(|e| e.end_point()));
            let ends_on_start = start.zip(end).is_some_and(|(s, e)| s.distance(e) < 1e-7);
            draws + 1 - usize::from(closed && ends_on_start)
        })
        .sum()
}

/// Whether the path has a segment that is a curve.
pub fn has_curves(path: &BezPath) -> bool {
    path.elements().iter().any(|e| matches!(e, PathEl::QuadTo(..) | PathEl::CurveTo(..)))
}

/// The first and last points of the path.
pub fn ends(path: &BezPath) -> (Point, Point) {
    let els = path.elements();
    let first = els.iter().find_map(|e| e.end_point()).expect("a point");
    let last = els.iter().rev().find_map(|e| e.end_point()).expect("a point");
    (first, last)
}

// ---------------------------------------------------------------- measures

pub fn rel_close(a: f64, b: f64, rel: f64) -> bool {
    (a - b).abs() <= rel * a.abs().max(b.abs()).max(1.0)
}

/// The distance from `p` to the nearest point of `bp`'s outline.
pub fn distance_to(bp: &BezPath, p: Point) -> f64 {
    bp.segments().map(|s| s.nearest(p, 1e-9).distance_sq).fold(f64::INFINITY, f64::min).sqrt()
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

/// The largest distance from a sampled point of `a` to the outline of `b`.
pub fn directed_hausdorff(a: &BezPath, b: &BezPath, n: usize) -> f64 {
    sample(a, n).into_iter().map(|p| distance_to(b, p)).fold(0.0, f64::max)
}

/// The symmetric Hausdorff distance (sampled).
pub fn hausdorff(a: &BezPath, b: &BezPath, n: usize) -> f64 {
    directed_hausdorff(a, b, n).max(directed_hausdorff(b, a, n))
}

/// The shoelace area of a polygon (absolute).
pub fn polygon_area(pts: &[Point]) -> f64 {
    let n = pts.len();
    (0..n).map(|i| pts[i].to_vec2().cross(pts[(i + 1) % n].to_vec2())).sum::<f64>().abs() / 2.0
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

/// Whether `p` is inside the filled area of `path` under `rule`, every subpath closed.
pub fn fill_contains(path: &BezPath, rule: craft_pathops::FillRule, p: Point) -> bool {
    let mut closed = BezPath::new();
    for sub in split(path) {
        closed.extend(sub.iter());
        if !matches!(sub.elements().last(), Some(PathEl::ClosePath)) {
            closed.close_path();
        }
    }
    let w = closed.winding(p);
    match rule {
        craft_pathops::FillRule::NonZero => w != 0,
        craft_pathops::FillRule::EvenOdd => w % 2 != 0,
    }
}

// ---------------------------------------------------------------- strategies

pub fn arb_point(lo: f64, hi: f64) -> impl Strategy<Value = Point> {
    (lo..hi, lo..hi).prop_map(|(x, y)| Point::new(x, y))
}

/// A non-degenerate rectangle inside `[0, 200]²`.
pub fn arb_rect() -> impl Strategy<Value = Rect> {
    (0.0..150.0f64, 0.0..150.0f64, 2.0..80.0f64, 2.0..80.0f64).prop_map(|(x, y, w, h)| Rect::new(x, y, x + w, y + h))
}

/// A simple (non self-intersecting) star-shaped polygon: points at increasing angles around a
/// centre with random radii. Every angular step stays below half a turn: a wider one puts the
/// centre outside and can make the polygon cross itself.
pub fn arb_star_polygon() -> impl Strategy<Value = Vec<Point>> {
    let steps = prop::collection::vec((0.2f64..1.0, 5.0f64..40.0), 3..12).prop_filter("an angular step of half a turn or more", |v| {
        let total: f64 = v.iter().map(|(a, _)| a).sum();
        v.iter().all(|(a, _)| 2.0 * a < total)
    });
    (arb_point(40.0, 160.0), steps).prop_map(|(c, v)| {
        let total: f64 = v.iter().map(|(a, _)| a).sum();
        let mut ang = 0.0;
        v.iter()
            .map(|&(a, r)| {
                ang += a / total * std::f64::consts::TAU;
                Point::new(c.x + r * ang.cos(), c.y + r * ang.sin())
            })
            .collect()
    })
}

/// A convex polygon (a polygon on one radius with jittered angles only).
pub fn arb_convex_polygon() -> impl Strategy<Value = Vec<Point>> {
    (arb_point(40.0, 160.0), 5.0f64..40.0, prop::collection::vec(0.3f64..1.0, 3..10)).prop_map(|(c, r, v)| {
        let total: f64 = v.iter().sum();
        let mut ang = 0.0;
        v.iter()
            .map(|a| {
                ang += a / total * std::f64::consts::TAU;
                Point::new(c.x + r * ang.cos(), c.y + r * ang.sin())
            })
            .collect()
    })
}

pub fn polygon_path(pts: &[Point]) -> BezPath {
    polyline(pts, true)
}

/// An axis-aligned ellipse as a path.
pub fn arb_ellipse() -> impl Strategy<Value = BezPath> {
    arb_rect().prop_map(ellipse)
}

/// Rectangles, ellipses and star-shaped polygons.
pub fn arb_closed_shape() -> impl Strategy<Value = BezPath> {
    prop_oneof![arb_rect().prop_map(rectangle), arb_ellipse(), arb_star_polygon().prop_map(|p| polygon_path(&p)),]
}

/// A smooth open curve through points on a gently varying function (for simplify tests): a
/// Catmull-Rom chain through `(i * step, y_i)` as cubics with C1 handles.
pub fn arb_smooth_curve() -> impl Strategy<Value = BezPath> {
    (prop::collection::vec(-20.0f64..20.0, 4..9), 10.0f64..30.0).prop_map(|(ys, step)| {
        let pts: Vec<Point> = ys.iter().enumerate().map(|(i, y)| Point::new(i as f64 * step, 100.0 + y)).collect();
        let n = pts.len();
        let tangent = |i: usize| {
            let a = pts[i.saturating_sub(1)];
            let b = pts[(i + 1).min(n - 1)];
            (b - a) / 6.0
        };
        let mut bp = BezPath::new();
        bp.move_to(pts[0]);
        for i in 0..n - 1 {
            bp.curve_to(pts[i] + tangent(i), pts[i + 1] - tangent(i + 1), pts[i + 1]);
        }
        bp
    })
}

/// A star polygon of `n` points (`n` odd) of radius 100 at `(dx, dy)`, each joined to the one nearly opposite, so that it
/// crosses itself in `n (k - 1)` places (`k = (n - 1) / 2`), with the edges bowed sideways by `bow` (a cubic for each; 0 gives
/// lines).
pub fn star_at(n: usize, bow: f64, dx: f64, dy: f64) -> BezPath {
    let k = (n - 1) / 2;
    let at = |i: usize| {
        let a = std::f64::consts::TAU * ((k * i) % n) as f64 / n as f64;
        Point::new(100.0 * a.cos() + dx, 100.0 * a.sin() + dy)
    };
    let mut bp = BezPath::new();
    bp.move_to(at(0));
    for i in 0..n {
        let (a, b) = (at(i), at((i + 1) % n));
        if bow == 0.0 {
            bp.line_to(b);
        } else {
            let d = b - a;
            let side = kurbo::Vec2::new(-d.y, d.x) * (bow / d.hypot().max(1e-9));
            bp.curve_to(a + d / 3.0 + side, a + d * (2.0 / 3.0) + side, b);
        }
    }
    bp.close_path();
    bp
}

/// `count` such stars in one path, each 250 above and beside the last, so that their heights do not overlap and the sweep
/// line holds the pieces of one of them at a time.
pub fn stacked_stars(count: usize, n: usize, bow: f64) -> BezPath {
    let mut all = BezPath::new();
    for j in 0..count {
        all.extend(star_at(n, bow, j as f64 * 250.0, j as f64 * 250.0).iter());
    }
    all
}
