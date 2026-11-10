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
    // An even `n` joins each point to the one opposite and makes no star (and 0 has no point to join): `n` is odd.
    debug_assert!(n % 2 == 1, "a star has an odd number of points, not {n}");
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

/// One polygon zigzagging through `n` nearly parallel lines `l` long, line `i` from `(i dx, i d)` to `(l - i dx, -i d)` with
/// `dx = l / 4n` and `d = dfac` sweep tolerances (so every two of them cross, in one point when they are not `spread`, which
/// jitters the ends by up to a fifth of `d` and puts the crossings along the lines), at angles under 1e-10 where `n d` is
/// small beside `l`. With `cross` false the right ends are at `+i d`, and no two cross.
pub fn nearly_parallel_lines(n: usize, l: f64, dfac: f64, spread: bool, cross: bool) -> BezPath {
    let eps = (l * f64::EPSILON * 64.0).max(1e-6);
    let d = dfac * eps;
    let dx = l / (4.0 * n as f64);
    let jitter = |i: usize, k: u64| if spread { (hash01(i as u64, k) - 0.5) * 0.4 * d } else { 0.0 };
    let left = |i: usize| Point::new(i as f64 * dx, i as f64 * d + jitter(i, 1));
    let flip = if cross { -1.0 } else { 1.0 };
    let right = |i: usize| Point::new(l - i as f64 * dx, flip * (i as f64) * d + jitter(i, 2));
    let mut bp = BezPath::new();
    bp.move_to(left(0));
    for i in 0..n {
        if i.is_multiple_of(2) {
            if i > 0 {
                bp.line_to(left(i));
            }
            bp.line_to(right(i));
        } else {
            bp.line_to(right(i));
            bp.line_to(left(i));
        }
    }
    let last = if n.is_multiple_of(2) { left(n - 1) } else { right(n - 1) };
    bp.line_to(Point::new(-dx, last.y));
    bp.line_to(Point::new(-dx, left(0).y));
    bp.close_path();
    bp
}

/// `count` pies of `k` thin triangular slices (each a triangle from the centre, half the width of its place), the centres on a
/// diagonal 250 apart so that the heights of the pies do not overlap: `2k` ends at each centre.
pub fn stacked_pies(count: usize, k: usize) -> BezPath {
    let mut bp = BezPath::new();
    for p in 0..count {
        let c = Point::new(p as f64 * 250.0, p as f64 * 250.0);
        for i in 0..k {
            let a0 = std::f64::consts::TAU * i as f64 / k as f64;
            let a1 = std::f64::consts::TAU * (i as f64 + 0.5) / k as f64;
            bp.move_to(c);
            bp.line_to(c + kurbo::Vec2::new(a0.cos(), a0.sin()) * 100.0);
            bp.line_to(c + kurbo::Vec2::new(a1.cos(), a1.sin()) * 100.0);
            bp.close_path();
        }
    }
    bp
}

/// A circle of radius `r` at `(cx, cy)` cut into `n` cubics (each the cubic that fits its arc), closed.
pub fn cubic_circle(n: usize, r: f64, cx: f64, cy: f64) -> BezPath {
    let da = std::f64::consts::TAU / n as f64;
    let h = 4.0 / 3.0 * (da / 4.0).tan() * r;
    let at = |a: f64| Point::new(cx + r * a.cos(), cy + r * a.sin());
    let along = |a: f64| kurbo::Vec2::new(-a.sin(), a.cos());
    let mut bp = BezPath::new();
    bp.move_to(at(0.0));
    for i in 0..n {
        let (a0, a1) = (i as f64 * da, (i + 1) as f64 * da);
        bp.curve_to(at(a0) + along(a0) * h, at(a1) - along(a1) * h, if i + 1 == n { at(0.0) } else { at(a1) });
    }
    bp.close_path();
    bp
}

/// `n` pairs of circles of radius `r`, each of four cubics, the second of each pair `d` to the right of the first: curves that run along one another,
/// as the two copies of a shape do that are moved a little. The pairs lie in a square grid 4 radii apart, in one path.
pub fn circle_pairs(n: usize, r: f64, d: f64) -> BezPath {
    circle_pairs_apart(n, r, d, 0.0)
}

/// [`circle_pairs`] with the second circle of each pair `dx` to the right of the first and `dy` further along y: where `dy` is more than the tolerance of
/// the sweep the two start at different heights, and the pairs are not found among the segments that start together.
pub fn circle_pairs_apart(n: usize, r: f64, dx: f64, dy: f64) -> BezPath {
    let side = (n as f64).sqrt().ceil() as usize;
    let mut bp = BezPath::new();
    for i in 0..n {
        let (cx, cy) = ((i % side) as f64 * 4.0 * r, (i / side) as f64 * 4.0 * r);
        bp.extend(cubic_circle(4, r, cx, cy));
        bp.extend(cubic_circle(4, r, cx + dx, cy + dy));
    }
    bp
}

/// `n` segments from the point (0, 0) along one line, the k-th to `(k, k)` (kind 0, the diagonal), `(0, k)` (kind 1, the vertical)
/// or `(k, 0)` (any other, level), each its own open subpath: lengths 1 to `n`, and every two share their start and overlap.
pub fn fan_along_a_line(n: usize, kind: usize) -> BezPath {
    let mut bp = BezPath::new();
    for k in 1..=n {
        let k = k as f64;
        bp.move_to((0.0, 0.0));
        bp.line_to(match kind {
            0 => (k, k),
            1 => (0.0, k),
            _ => (k, 0.0),
        });
    }
    bp
}

/// A closed square of side 100 whose sides (level and upright) are each cut into `pieces` collinear segments that touch end to end.
pub fn square_with_cut_sides(pieces: usize) -> BezPath {
    let at = |side: usize, i: usize| {
        let t = 100.0 * i as f64 / pieces as f64;
        match side {
            0 => (t, 0.0),
            1 => (100.0, t),
            2 => (100.0 - t, 100.0),
            _ => (0.0, 100.0 - t),
        }
    };
    let mut bp = BezPath::new();
    bp.move_to((0.0, 0.0));
    for side in 0..4 {
        for i in 1..=pieces {
            bp.line_to(at(side, i));
        }
    }
    bp.close_path();
    bp
}

/// A closed diagonal strip, a parallelogram with its two long sides each cut into `pieces` collinear segments that touch end to end.
pub fn strip_with_cut_sides(pieces: usize) -> BezPath {
    let along = |from: (f64, f64), to: (f64, f64), i: usize| {
        let t = i as f64 / pieces as f64;
        (from.0 + (to.0 - from.0) * t, from.1 + (to.1 - from.1) * t)
    };
    let mut bp = BezPath::new();
    bp.move_to((0.0, 0.0));
    for i in 1..=pieces {
        bp.line_to(along((0.0, 0.0), (1000.0, 1000.0), i));
    }
    bp.line_to((1000.0, 1010.0));
    for i in 1..=pieces {
        bp.line_to(along((1000.0, 1010.0), (0.0, 10.0), i));
    }
    bp.close_path();
    bp
}

/// A number from 0 up to 1 made of two integers, the same each time.
pub fn hash01(i: u64, k: u64) -> f64 {
    let mut h = i.wrapping_mul(0x9E37_79B9_7F4A_7C15) ^ k.wrapping_mul(0xD1B5_4A32_D192_ED03);
    h ^= h >> 31;
    h = h.wrapping_mul(0xBF58_476D_1CE4_E5B9);
    h ^= h >> 29;
    (h >> 11) as f64 / (1u64 << 53) as f64
}

/// A wheel of `k` triangles from one centre, whose sides are shared: each radius is twice, once for each of the two neighbours.
pub fn wheel(k: usize) -> BezPath {
    let mut bp = BezPath::new();
    let at = |i: usize| {
        let a = std::f64::consts::TAU * i as f64 / k as f64;
        Point::new(100.0 * a.cos(), 100.0 * a.sin())
    };
    for i in 0..k {
        bp.move_to((0.0, 0.0));
        bp.line_to(at(i));
        bp.line_to(at(i + 1));
        bp.close_path();
    }
    bp
}

/// [`stacked_pies`] whose slices have their apexes moved at random, each within a square `spread` across.
pub fn jittered_pies(count: usize, k: usize, spread: f64) -> BezPath {
    let mut bp = BezPath::new();
    for p in 0..count {
        let c = Point::new(p as f64 * 250.0, p as f64 * 250.0);
        for i in 0..k {
            let a0 = std::f64::consts::TAU * i as f64 / k as f64;
            let a1 = std::f64::consts::TAU * (i as f64 + 0.5) / k as f64;
            let apex = c + kurbo::Vec2::new((hash01(i as u64, 7) - 0.5) * spread, (hash01(i as u64, 9) - 0.5) * spread);
            bp.move_to(apex);
            bp.line_to(c + kurbo::Vec2::new(a0.cos(), a0.sin()) * 100.0);
            bp.line_to(c + kurbo::Vec2::new(a1.cos(), a1.sin()) * 100.0);
            bp.close_path();
        }
    }
    bp
}

/// `n` lines `len` long through the neighbourhood of (1000, 1000), at angles 0.3 + 1e-3 k, each moved sideways at random by up to
/// `jitter` (so they cross one another within a few tolerances when `jitter` is a few), as one zigzag polygon.
pub fn jittered_pencil(n: usize, len: f64, jitter: f64) -> BezPath {
    let mut bp = BezPath::new();
    for i in 0..n {
        let a = 0.3 + 1e-3 * i as f64;
        let v = kurbo::Vec2::new(a.cos(), a.sin()) * len;
        let side = kurbo::Vec2::new(-a.sin(), a.cos()) * ((hash01(i as u64, 3) - 0.5) * jitter);
        let c = Point::new(1000.0, 1000.0) + side;
        let (s, e) = if i % 2 == 0 { (c - v, c + v) } else { (c + v, c - v) };
        if i == 0 {
            bp.move_to(s);
        } else {
            bp.line_to(s);
        }
        bp.line_to(e);
    }
    bp.close_path();
    bp
}

/// `n` straight cubics from (0, 0) along the diagonal, the k-th to (k, k), each its own closed subpath (by the line back).
pub fn cubic_fan(n: usize) -> BezPath {
    let mut bp = BezPath::new();
    for k in 1..=n {
        let q = Point::new(k as f64, k as f64);
        let o = Point::ORIGIN;
        bp.move_to(o);
        bp.curve_to(o.lerp(q, 1.0 / 3.0), o.lerp(q, 2.0 / 3.0), q);
        bp.close_path();
    }
    bp
}

/// The arc the fans and windows are pieces of.
fn the_arc() -> kurbo::CubicBez {
    kurbo::CubicBez::new((0.0, 0.0), (100.0, 150.0), (300.0, 150.0), (400.0, 0.0))
}

/// `n` pieces of one arc from its start, the k-th to parameter k / n, each its own subpath (closed by its chord).
pub fn arc_fan(n: usize) -> BezPath {
    use kurbo::ParamCurve;
    let mut bp = BezPath::new();
    for k in 1..=n {
        let s = the_arc().subsegment(0.0..k as f64 / n as f64);
        bp.move_to(s.p0);
        bp.curve_to(s.p1, s.p2, s.p3);
        bp.close_path();
    }
    bp
}

/// `n` windows of one arc, the k-th from parameter k / 2n to k / 2n + 0.5: every two overlap along it and none share an end.
pub fn arc_windows(n: usize) -> BezPath {
    use kurbo::ParamCurve;
    let mut bp = BezPath::new();
    for k in 0..n {
        let t0 = k as f64 / (2.0 * n as f64);
        let s = the_arc().subsegment(t0..t0 + 0.5);
        bp.move_to(s.p0);
        bp.curve_to(s.p1, s.p2, s.p3);
        bp.close_path();
    }
    bp
}

/// `m` copies of one regular polygon of `k` corners (radius 100), one exactly on another, each its own closed subpath.
pub fn stacked_polygons(k: usize, m: usize) -> BezPath {
    let mut bp = BezPath::new();
    let corners: Vec<Point> = (0..k)
        .map(|i| {
            let a = std::f64::consts::TAU * i as f64 / k as f64 + 0.1;
            Point::new(100.0 * a.cos() + 300.0, 100.0 * a.sin() + 300.0)
        })
        .collect();
    for _ in 0..m {
        for (i, p) in corners.iter().enumerate() {
            if i == 0 {
                bp.move_to(*p);
            } else {
                bp.line_to(*p);
            }
        }
        bp.close_path();
    }
    bp
}

/// `s` stacks, 250 apart along a diagonal, of `m` copies of a segment (each a segment and back, its own closed subpath).
pub fn stacked_segments(s: usize, m: usize) -> BezPath {
    let mut bp = BezPath::new();
    for t in 0..s {
        let off = t as f64 * 250.0;
        for _ in 0..m {
            bp.move_to((off + 100.0 * 0.1f64.cos(), off + 100.0 * 0.1f64.sin()));
            bp.line_to((off - 100.0 * 0.1f64.cos(), off - 100.0 * 0.1f64.sin()));
            bp.close_path();
        }
    }
    bp
}

/// `m` copies of one arc, one exactly on another, each its own closed subpath (closed by its chord).
pub fn stacked_arcs(m: usize) -> BezPath {
    let mut bp = BezPath::new();
    let c = the_arc();
    for _ in 0..m {
        bp.move_to(c.p0);
        bp.curve_to(c.p1, c.p2, c.p3);
        bp.close_path();
    }
    bp
}

/// The corners of the regular polygon of `k` corners of the given radius (turned 0.1, three radii from the origin) the copies below are of.
fn polygon_corners(k: usize, radius: f64) -> Vec<Point> {
    (0..k)
        .map(|i| {
            let a = std::f64::consts::TAU * i as f64 / k as f64 + 0.1;
            Point::new(radius * a.cos() + 3.0 * radius, radius * a.sin() + 3.0 * radius)
        })
        .collect()
}

/// `m` copies of one regular polygon of `k` corners and the given radius, each its own closed subpath, the `c`-th with every coordinate `c`
/// units in the last place from where the first has it: copies that are none to a count that takes the bits of their points for them.
pub fn polygons_an_ulp_apart(k: usize, m: usize, radius: f64) -> BezPath {
    let mut bp = BezPath::new();
    for c in 0..m {
        for (i, p) in polygon_corners(k, radius).iter().enumerate() {
            let q = Point::new(f64::from_bits(p.x.to_bits() + c as u64), f64::from_bits(p.y.to_bits() + c as u64));
            if i == 0 {
                bp.move_to(q);
            } else {
                bp.line_to(q);
            }
        }
        bp.close_path();
    }
    bp
}

/// `m` copies of one regular polygon of `k` corners and the given radius, the `c`-th moved by `(c mod 7)` by `(c mod 5)` steps of `step`
/// tolerances of the sweep (a millionth each, at these sizes): copies a few tolerances apart.
pub fn polygons_a_few_tolerances_apart(k: usize, m: usize, step: f64, radius: f64) -> BezPath {
    let mut bp = BezPath::new();
    for c in 0..m {
        let by = kurbo::Vec2::new((c % 7) as f64 * step * 1e-6, (c % 5) as f64 * step * 1e-6);
        for (i, p) in polygon_corners(k, radius).iter().enumerate() {
            if i == 0 {
                bp.move_to(*p + by);
            } else {
                bp.line_to(*p + by);
            }
        }
        bp.close_path();
    }
    bp
}

/// `n` windows of the arc the fans are pieces of, the `k`-th from `0.4 - 0.4 u` to `0.5 + 0.4 u` with `u = (k + 1) / n`: every two of them
/// lie along one another for a tenth of the arc (at least), which is under a quarter of each window, each its own closed subpath.
pub fn windows_sharing_a_tenth(n: usize) -> BezPath {
    use kurbo::ParamCurve;
    let mut bp = BezPath::new();
    for k in 0..n {
        let u = (k as f64 + 1.0) / n as f64;
        let s = the_arc().subsegment((0.4 - 0.4 * u)..(0.5 + 0.4 * u));
        bp.move_to(s.p0);
        bp.curve_to(s.p1, s.p2, s.p3);
        bp.close_path();
    }
    bp
}
