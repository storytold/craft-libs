//! Paths for the unit tests: the shapes the suites draw, built as `BezPath`s.

use kurbo::{BezPath, Point, Rect};

/// The control-point distance of a quarter circle's cubic, as a fraction of the radius.
const KAPPA: f64 = 0.552_284_749_830_793_6;

/// A polyline through `points`, closed or open. A closed one ends with a segment back to its start,
/// as the crate writes closed subpaths.
pub(crate) fn polyline(points: &[(f64, f64)], closed: bool) -> BezPath {
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

/// A polyline through `points` of type `Point`.
pub(crate) fn polyline_pts(points: &[Point], closed: bool) -> BezPath {
    let v: Vec<(f64, f64)> = points.iter().map(|p| (p.x, p.y)).collect();
    polyline(&v, closed)
}

/// The rectangle with corners `(x0, y0)` and `(x1, y1)`.
pub(crate) fn rect(x0: f64, y0: f64, x1: f64, y1: f64) -> BezPath {
    polyline(&[(x0, y0), (x1, y0), (x1, y1), (x0, y1)], true)
}

/// The ellipse inscribed in `r`: four cubics from the left, top, right and bottom points.
pub(crate) fn ellipse(r: Rect) -> BezPath {
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

/// A straight line from `a` to `b`, open.
pub(crate) fn line(a: Point, b: Point) -> BezPath {
    polyline(&[(a.x, a.y), (b.x, b.y)], false)
}

/// The number of subpaths of `path`.
pub(crate) fn subpath_count(path: &BezPath) -> usize {
    crate::path::subs(path).len()
}

/// The number of anchors of `path`: the points its subpaths pass through, a closed subpath's start
/// counted once.
pub(crate) fn anchor_count(path: &BezPath) -> usize {
    crate::path::subs(path).iter().map(crate::path::Sub::anchors).sum()
}

/// The total length of `path`.
pub(crate) fn length(path: &BezPath) -> f64 {
    use kurbo::ParamCurveArclen;
    path.segments().map(|s| s.arclen(1e-6)).sum()
}

/// A lens: two cubics from `(x, y)` to `(x + 3, y)`, one bulging 3 above and one 3 below, closed.
pub(crate) fn lens(x: f64, y: f64) -> BezPath {
    let mut b = BezPath::new();
    b.move_to((x, y));
    b.curve_to((x, y + 4.0), (x + 3.0, y + 4.0), (x + 3.0, y));
    b.curve_to((x + 3.0, y - 4.0), (x, y - 4.0), (x, y));
    b.close_path();
    b
}

/// `n` lenses 5 apart at one height: a row of dots. Their bounds never touch, and all of them
/// cross the same horizontal lines.
pub(crate) fn row_of_lenses(n: usize) -> BezPath {
    let mut b = BezPath::new();
    for i in 0..n {
        b.extend(lens(i as f64 * 5.0, 20.0).iter());
    }
    b
}

/// `n` lenses in rows of about `sqrt(n)`, 5 apart across and 10 apart down.
pub(crate) fn grid_of_lenses(n: usize) -> BezPath {
    let k = ((n as f64).sqrt().ceil() as usize).max(1);
    let mut b = BezPath::new();
    for i in 0..n {
        b.extend(lens((i % k) as f64 * 5.0, 20.0 + (i / k) as f64 * 10.0).iter());
    }
    b
}

/// `n` lenses one above the other at one x: a column. Their heights never overlap.
pub(crate) fn column_of_lenses(n: usize) -> BezPath {
    let mut b = BezPath::new();
    for i in 0..n {
        b.extend(lens(0.0, 20.0 + i as f64 * 10.0).iter());
    }
    b
}

/// `m` thin parallelograms slanting one way across `m` slanting the other: 4m² crossings, none of
/// them level, and about 4m pieces on the sweep line at the middle.
pub(crate) fn diagonals(m: usize) -> BezPath {
    let mut b = BezPath::new();
    let span = m as f64 * 4.0;
    for i in 0..m {
        let x = i as f64 * 4.0 - span / 2.0;
        b.move_to((x, 0.0));
        b.line_to((x + 1.0, 0.0));
        b.line_to((x + span + 1.0, span));
        b.line_to((x + span, span));
        b.close_path();
    }
    for i in 0..m {
        let x = i as f64 * 4.0 + span / 2.0 + 0.37;
        b.move_to((x, 0.0));
        b.line_to((x + 1.0, 0.0));
        b.line_to((x + 1.0 - span, span));
        b.line_to((x - span, span));
        b.close_path();
    }
    b
}

/// A closed polygon of `n` edges (`n` odd), vertex `i` at the angle `2π (k i mod n) / n` on a circle of radius 100,
/// `k = (n - 1) / 2`: each edge goes to a vertex nearly opposite, and crosses most of the others (`n (k - 1)`
/// crossings in all, about n²/2).
pub(crate) fn star(n: usize) -> BezPath {
    let k = (n - 1) / 2;
    let mut bp = BezPath::new();
    for i in 0..n {
        let a = std::f64::consts::TAU * ((k * i) % n) as f64 / n as f64;
        let p = (100.0 * a.cos(), 100.0 * a.sin());
        if i == 0 {
            bp.move_to(p);
        } else {
            bp.line_to(p);
        }
    }
    bp.close_path();
    bp
}

/// A star polygon of `n` points (`n` odd) of radius 100 at `(dx, dy)`, each joined to the one nearly opposite, as [`star`]
/// but with the edges bowed sideways by `bow` (a cubic for each; 0 gives the lines of [`star`]).
pub(crate) fn bowed_star(n: usize, bow: f64, dx: f64, dy: f64) -> BezPath {
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

/// `count` stars of `n` points in one path, each 250 above and beside the last so that their heights do not overlap and their
/// bounds touch none of the others': each crosses itself in `n (k - 1)` places (`k = (n - 1) / 2`) and the sweep line
/// holds the pieces of one of them at a time.
pub(crate) fn stacked_stars(count: usize, n: usize, bow: f64) -> BezPath {
    let mut all = BezPath::new();
    for j in 0..count {
        all.extend(bowed_star(n, bow, j as f64 * 250.0, j as f64 * 250.0).iter());
    }
    all
}

/// One polygon zigzagging through `n` nearly parallel lines `l` long, line `i` from `(i dx, i d)` to `(l - i dx, -i d)` with
/// `dx = l / 4n` and `d = dfac` sweep tolerances (so every two of them cross, in one point when they are not `spread`, which
/// jitters the ends by up to a fifth of `d` and puts the crossings along the lines), at angles under 1e-10 where `n d` is
/// small beside `l`. With `cross` false the right ends are at `+i d`, and no two cross.
pub(crate) fn nearly_parallel_lines(n: usize, l: f64, dfac: f64, spread: bool, cross: bool) -> BezPath {
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
pub(crate) fn stacked_pies(count: usize, k: usize) -> BezPath {
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

/// A number from 0 up to 1 made of two integers, the same each time.
pub(crate) fn hash01(i: u64, k: u64) -> f64 {
    let mut h = i.wrapping_mul(0x9E37_79B9_7F4A_7C15) ^ k.wrapping_mul(0xD1B5_4A32_D192_ED03);
    h ^= h >> 31;
    h = h.wrapping_mul(0xBF58_476D_1CE4_E5B9);
    h ^= h >> 29;
    (h >> 11) as f64 / (1u64 << 53) as f64
}

/// A wheel of `k` triangles from one centre, whose sides are shared: each radius is twice, once for each of the two neighbours.
pub(crate) fn wheel(k: usize) -> BezPath {
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
pub(crate) fn jittered_pies(count: usize, k: usize, spread: f64) -> BezPath {
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
pub(crate) fn jittered_pencil(n: usize, len: f64, jitter: f64) -> BezPath {
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
pub(crate) fn cubic_fan(n: usize) -> BezPath {
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
pub(crate) fn arc_fan(n: usize) -> BezPath {
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
pub(crate) fn arc_windows(n: usize) -> BezPath {
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
pub(crate) fn stacked_polygons(k: usize, m: usize) -> BezPath {
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
pub(crate) fn stacked_segments(s: usize, m: usize) -> BezPath {
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
pub(crate) fn polygons_an_ulp_apart(k: usize, m: usize, radius: f64) -> BezPath {
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
pub(crate) fn polygons_a_few_tolerances_apart(k: usize, m: usize, step: f64, radius: f64) -> BezPath {
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
pub(crate) fn windows_sharing_a_tenth(n: usize) -> BezPath {
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

/// `n` segments from the point (0, 0) down the diagonal, the k-th to `(k, k)`, each its own closed subpath (a segment and back).
pub(crate) fn fan_along_a_line(n: usize) -> BezPath {
    let mut bp = BezPath::new();
    for k in 1..=n {
        bp.move_to((0.0, 0.0));
        bp.line_to((k as f64, k as f64));
        bp.close_path();
    }
    bp
}

/// `n` rings in a grid of about `sqrt(n)` by `sqrt(n)`, 25 apart: each a circle of radius 10 (four cubics) and a circle of radius `inner`
/// the other way round (four cubics), as a closed outline of a ring.
pub(crate) fn rings(n: usize, inner: f64) -> BezPath {
    let k = ((n as f64).sqrt().ceil() as usize).max(1);
    let mut bp = BezPath::new();
    for i in 0..n {
        let (cx, cy) = ((i % k) as f64 * 25.0, (i / k) as f64 * 25.0);
        for (r, reversed) in [(10.0, false), (inner, true)] {
            let q = r * KAPPA;
            let arcs = [
                [(cx - r, cy), (cx - r, cy - q), (cx - q, cy - r), (cx, cy - r)],
                [(cx, cy - r), (cx + q, cy - r), (cx + r, cy - q), (cx + r, cy)],
                [(cx + r, cy), (cx + r, cy + q), (cx + q, cy + r), (cx, cy + r)],
                [(cx, cy + r), (cx - q, cy + r), (cx - r, cy + q), (cx - r, cy)],
            ];
            bp.move_to(arcs[0][0]);
            if reversed {
                for a in arcs.iter().rev() {
                    bp.curve_to(a[2], a[1], a[0]);
                }
            } else {
                for a in &arcs {
                    bp.curve_to(a[1], a[2], a[3]);
                }
            }
            bp.close_path();
        }
    }
    bp
}

/// `n` S-shaped strokes in a grid, 30 apart: each a closed outline of two S-shaped cubics a stroke `w` apart, joined by short caps.
pub(crate) fn s_strokes(n: usize, w: f64) -> BezPath {
    let k = ((n as f64).sqrt().ceil() as usize).max(1);
    let mut bp = BezPath::new();
    for i in 0..n {
        let (x, y) = ((i % k) as f64 * 30.0, (i / k) as f64 * 30.0);
        let h = w / 2.0;
        bp.move_to((x - h, y));
        bp.curve_to((x + 15.0 - h, y + 5.0), (x - 15.0 - h, y + 15.0), (x - h, y + 20.0));
        bp.line_to((x + h, y + 20.0));
        bp.curve_to((x - 15.0 + h, y + 15.0), (x + 15.0 + h, y + 5.0), (x + h, y));
        bp.close_path();
    }
    bp
}

/// A mesh of `n` by `n` cells of side 1, each cut into two triangles, each triangle its own closed subpath.
pub(crate) fn mesh(n: usize) -> BezPath {
    let mut bp = BezPath::new();
    for i in 0..n {
        for j in 0..n {
            let (x, y) = (i as f64, j as f64);
            bp.extend(polyline(&[(x, y), (x + 1.0, y), (x + 1.0, y + 1.0)], true).iter());
            bp.extend(polyline(&[(x, y), (x + 1.0, y + 1.0), (x, y + 1.0)], true).iter());
        }
    }
    bp
}

/// A grid of `n` by `n` squares of side 1, each its own closed subpath.
pub(crate) fn grid_of_squares(n: usize) -> BezPath {
    let mut bp = BezPath::new();
    for i in 0..n {
        for j in 0..n {
            bp.extend(rect(i as f64, j as f64, i as f64 + 1.0, j as f64 + 1.0).iter());
        }
    }
    bp
}

/// `n` rectangles side by side on one baseline (their bottoms on y = 0), half a unit apart so that they overlap.
pub(crate) fn bars_on_a_baseline(n: usize) -> BezPath {
    let mut bp = BezPath::new();
    for i in 0..n {
        let x = i as f64 * 0.5;
        bp.extend(rect(x, 0.0, x + 10.0, 5.0 + i as f64 * 0.01).iter());
    }
    bp
}

/// `count` groups, 40 apart on a diagonal, of `g` near-identical closed loops of one cubic each (the loops 0.01 apart).
pub(crate) fn groups_of_loops(count: usize, g: usize) -> BezPath {
    let mut bp = BezPath::new();
    for k in 0..count {
        let (x0, y0) = (k as f64 * 40.0, k as f64 * 40.0);
        for i in 0..g {
            let p = Point::new(x0 + i as f64 * 0.01, y0);
            bp.move_to(p);
            bp.curve_to(p + kurbo::Vec2::new(30.0, 30.0), p + kurbo::Vec2::new(-30.0, 30.0), p);
            bp.close_path();
        }
    }
    bp
}

/// A circle of radius `r` at `(cx, cy)` cut into `n` cubics (each the cubic that fits its arc), closed.
pub(crate) fn cubic_circle(n: usize, r: f64, cx: f64, cy: f64) -> BezPath {
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

/// A path as the polygon of its flattening, to read the winding number of a point from, as an oracle for the winding of
/// the curves: the crossings of a ray to the left with straight edges, found by none of the arithmetic of the cubics. It
/// holds for a point farther from the path than the flattening's tolerance (and than the rounding of the crossing, which
/// the callers take care of by the distance they keep).
pub(crate) struct Flattened {
    edges: Vec<(Point, Point)>,
}

impl Flattened {
    /// `path` flattened to `tolerance`, each subpath closed.
    pub(crate) fn new(path: &BezPath, tolerance: f64) -> Self {
        let mut edges = Vec::new();
        let (mut last, mut start) = (Point::ZERO, Point::ZERO);
        kurbo::flatten(path.iter(), tolerance, |el| match el {
            kurbo::PathEl::MoveTo(p) => {
                if last != start {
                    edges.push((last, start));
                }
                (last, start) = (p, p);
            }
            kurbo::PathEl::LineTo(p) => {
                edges.push((last, p));
                last = p;
            }
            kurbo::PathEl::ClosePath => {
                if last != start {
                    edges.push((last, start));
                }
                last = start;
            }
            _ => {}
        });
        if last != start {
            edges.push((last, start));
        }
        Self { edges }
    }

    /// The winding number at `p`: each edge that crosses the height of `p` (its lower end in, its upper out) to the left of
    /// it counts -1 where it rises and +1 where it falls.
    pub(crate) fn winding(&self, p: Point) -> i32 {
        let mut w = 0;
        for &(a, b) in &self.edges {
            if (a.y <= p.y) != (b.y <= p.y) {
                let x = a.x + (p.y - a.y) / (b.y - a.y) * (b.x - a.x);
                if p.x >= x {
                    w += if b.y > a.y { -1 } else { 1 };
                }
            }
        }
        w
    }
}

/// How far `p` is from `path` (its segments themselves, to 1e-9).
pub(crate) fn distance_to_path(path: &BezPath, p: Point) -> f64 {
    use kurbo::ParamCurveNearest;
    path.segments().map(|s| s.nearest(p, 1e-9).distance_sq).fold(f64::INFINITY, f64::min).sqrt()
}

/// A generator of numbers from a seed (xorshift), for tests that draw many shapes and want the same ones every run.
pub(crate) struct Rng(pub(crate) u64);

impl Rng {
    /// The next number, in `0..1`.
    pub(crate) fn unit(&mut self) -> f64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        (self.0 >> 11) as f64 / (1u64 << 53) as f64
    }

    /// The next number, in `lo..hi`.
    pub(crate) fn range(&mut self, lo: f64, hi: f64) -> f64 {
        lo + (hi - lo) * self.unit()
    }
}

/// `m` copies of one arc, one exactly on another, each its own closed subpath (closed by its chord).
pub(crate) fn stacked_arcs(m: usize) -> BezPath {
    let mut bp = BezPath::new();
    let c = the_arc();
    for _ in 0..m {
        bp.move_to(c.p0);
        bp.curve_to(c.p1, c.p2, c.p3);
        bp.close_path();
    }
    bp
}
