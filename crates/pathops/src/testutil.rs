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
