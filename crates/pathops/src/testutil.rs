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
