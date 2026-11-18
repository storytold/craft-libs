//! Hit testing primitives.

use std::borrow::Cow;

use kurbo::{BezPath, ParamCurveNearest, PathEl, Point, Rect, Shape};

use crate::path::{FillRule, PathData};

/// Does the filled area of `path` cover all of `r` shrunk by `tol` on every side? Answered for one
/// convex polygon (a single subpath of straight segments), which covers the rect when it contains
/// its four corners; any other path gives `false`, so a caller treats it as not covering.
pub fn polygon_covers_rect(path: &BezPath, r: Rect, tol: f64) -> bool {
    let mut pts: Vec<Point> = Vec::new();
    for (i, el) in path.elements().iter().enumerate() {
        match *el {
            PathEl::MoveTo(p) if i == 0 => pts.push(p),
            PathEl::LineTo(p) if i > 0 => pts.push(p),
            PathEl::ClosePath if i > 0 => {}
            _ => return false,
        }
    }
    if pts.len() > 1 && pts.first() == pts.last() {
        pts.pop();
    }
    if pts.len() < 3 {
        return false;
    }
    // Convex: every turn goes the same way (straight runs allowed).
    let n = pts.len();
    let mut sign = 0.0f64;
    let mut turning = 0.0f64;
    for i in 0..n {
        let (a, b, c) = (pts[i], pts[(i + 1) % n], pts[(i + 2) % n]);
        let cross = (b - a).cross(c - b);
        turning += cross.atan2((b - a).dot(c - b));
        if cross.abs() <= 1e-9 {
            continue;
        }
        if sign == 0.0 {
            sign = cross.signum();
        } else if cross.signum() != sign {
            return false;
        }
    }
    // A simple convex polygon turns through exactly one full revolution; a star winds more.
    if (turning.abs() - std::f64::consts::TAU).abs() > 1e-6 {
        return false;
    }
    let inner = r.inflate(-tol, -tol);
    sign != 0.0
        && [Point::new(inner.x0, inner.y0), Point::new(inner.x1, inner.y0), Point::new(inner.x1, inner.y1), Point::new(inner.x0, inner.y1)]
            .into_iter()
            .all(|c| fill_contains(path, FillRule::NonZero, c))
}

/// Is `p` inside the filled area of `path` under `rule`? Open subpaths are implicitly closed (as when filled).
pub fn fill_contains(path: &BezPath, rule: FillRule, p: Point) -> bool {
    let w = closed(path).winding(p);
    match rule {
        FillRule::NonZero => w != 0,
        FillRule::EvenOdd => w % 2 != 0,
    }
}

/// `path` with each open subpath closed by a straight line back to its start, the edge its fill
/// has. kurbo's winding leaves that edge out, so a point beside it counted as outside the fill:
/// an open path lost its fill clicks on that side (a reflection moved the edge to the other one).
fn closed(path: &BezPath) -> Cow<'_, BezPath> {
    let mut open = false;
    let mut needs = false;
    for el in path.elements() {
        match el {
            PathEl::MoveTo(_) => {
                needs |= open;
                open = true;
            }
            PathEl::ClosePath => open = false,
            _ => {}
        }
    }
    if !(needs || open) {
        return Cow::Borrowed(path);
    }
    let mut out = BezPath::new();
    let mut open = false;
    for &el in path.elements() {
        match el {
            PathEl::MoveTo(_) => {
                if open {
                    out.close_path();
                }
                open = true;
            }
            PathEl::ClosePath => open = false,
            _ => {}
        }
        out.push(el);
    }
    if open {
        out.close_path();
    }
    Cow::Owned(out)
}

/// Distance from `p` to the nearest point on the path outline.
pub fn distance_to_outline(path: &BezPath, p: Point) -> f64 {
    path.segments().map(|s| s.nearest(p, 1e-9).distance_sq).fold(f64::INFINITY, f64::min).sqrt()
}

/// Is `p` within `tol` of the path's stroke of width `width`?
pub fn stroke_contains(path: &BezPath, width: f64, tol: f64, p: Point) -> bool {
    let quick = path.bounding_box().inflate(width / 2.0 + tol, width / 2.0 + tol);
    quick.contains(p) && distance_to_outline(path, p) <= width / 2.0 + tol
}

/// Does the path touch the rect: an anchor inside it, or a segment crossing its edges? Used for
/// marquee selection, which (as in Illustrator) needs the path itself: a rect wholly inside a
/// filled shape doesn't touch it, so a marquee drawn over a background picks only what is on it
/// (#1103).
pub fn intersects_rect(path: &PathData, r: Rect) -> bool {
    let Some(b) = path.bounds() else { return false };
    let degenerate = |x: Rect| x.width() == 0.0 || x.height() == 0.0;
    if b.intersect(r).area() <= 0.0 && !degenerate(b) && !degenerate(r) {
        return false;
    }
    // Any anchor inside?
    if path.anchors().any(|(_, _, a)| r.contains(a.p)) {
        return true;
    }
    // Any segment crossing the rect edges?
    let edges = [
        kurbo::Line::new((r.x0, r.y0), (r.x1, r.y0)),
        kurbo::Line::new((r.x1, r.y0), (r.x1, r.y1)),
        kurbo::Line::new((r.x1, r.y1), (r.x0, r.y1)),
        kurbo::Line::new((r.x0, r.y1), (r.x0, r.y0)),
    ];
    path.to_bezpath().segments().any(|seg| edges.iter().any(|e| !seg.intersect_line(*e).is_empty()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::shapes;

    #[test]
    fn polygon_covers_rect_only_for_convex_polygons_around_it() {
        let page = Rect::new(0.0, 0.0, 100.0, 100.0);
        let poly = |pts: &[(f64, f64)]| {
            let mut b = BezPath::new();
            b.move_to(pts[0]);
            for &p in &pts[1..] {
                b.line_to(p);
            }
            b.close_path();
            b
        };
        assert!(polygon_covers_rect(&poly(&[(0.0, 0.0), (100.0, 0.0), (100.0, 100.0), (0.0, 100.0)]), page, 0.01));
        assert!(polygon_covers_rect(&poly(&[(50.0, -60.0), (160.0, 50.0), (50.0, 160.0), (-60.0, 50.0)]), page, 0.01), "turned, around it");
        // The page's bounding box, but its corners are cut away.
        assert!(!polygon_covers_rect(&poly(&[(50.0, 0.0), (100.0, 50.0), (50.0, 100.0), (0.0, 50.0)]), page, 0.01));
        // Contains the corners but notches into the page: not convex.
        assert!(!polygon_covers_rect(&poly(&[(-1.0, -1.0), (101.0, -1.0), (101.0, 101.0), (50.0, 40.0), (-1.0, 101.0)]), page, 0.01));
        // Curves and several subpaths aren't judged.
        assert!(!polygon_covers_rect(&kurbo::Circle::new((50.0, 50.0), 200.0).to_path(0.1), page, 0.01));
        let mut two = poly(&[(0.0, 0.0), (100.0, 0.0), (100.0, 100.0), (0.0, 100.0)]);
        two.extend(poly(&[(10.0, 10.0), (20.0, 10.0), (20.0, 20.0)]));
        assert!(!polygon_covers_rect(&two, page, 0.01));
        assert!(!polygon_covers_rect(&BezPath::new(), page, 0.01));
    }

    #[test]
    fn fill_and_stroke() {
        let p = shapes::rectangle(Rect::new(0.0, 0.0, 10.0, 10.0)).to_bezpath();
        assert!(fill_contains(&p, FillRule::NonZero, Point::new(5.0, 5.0)));
        assert!(!fill_contains(&p, FillRule::NonZero, Point::new(15.0, 5.0)));
        assert!(stroke_contains(&p, 2.0, 0.0, Point::new(10.5, 5.0)));
        assert!(!stroke_contains(&p, 2.0, 0.0, Point::new(12.0, 5.0)));
    }

    #[test]
    fn evenodd_hole() {
        let mut outer = shapes::rectangle(Rect::new(0.0, 0.0, 10.0, 10.0));
        outer.subpaths.extend(shapes::rectangle(Rect::new(3.0, 3.0, 7.0, 7.0)).subpaths);
        let bp = outer.to_bezpath();
        assert!(!fill_contains(&bp, FillRule::EvenOdd, Point::new(5.0, 5.0)));
        assert!(fill_contains(&bp, FillRule::NonZero, Point::new(5.0, 5.0)));
        assert!(fill_contains(&bp, FillRule::EvenOdd, Point::new(1.0, 5.0)));
    }

    /// An open path fills as if closed by a straight line back to its start, and is hit there,
    /// whichever side that line falls on (a reflected open path lost its fill clicks).
    #[test]
    fn open_paths_fill_as_closed_on_either_side() {
        // A filled sliver left open by a deleted anchor; its closing line runs down its left side.
        let open = BezPath::from_svg(
            "M86.311 69.612C100.544 78.261 110.506 88.64 110.506 100.748C110.506 111.127 100.544 118.046 86.311 118.046C93.214 102.045 91.407 102.403 91.407 97.61",
        )
        .unwrap();
        let mut closed = open.clone();
        closed.close_path();
        for p in [Point::new(100.0, 100.0), Point::new(95.0, 85.0), Point::new(92.0, 112.0)] {
            assert!(closed.winding(p) != 0, "{p:?} is inside the closed sliver");
            assert!(fill_contains(&open, FillRule::NonZero, p), "{p:?}: the open sliver's fill");
            assert!(fill_contains(&open, FillRule::EvenOdd, p), "{p:?}: even-odd too");
        }
        assert!(!fill_contains(&open, FillRule::NonZero, Point::new(80.0, 100.0)), "left of the closing line");
        // Mirrored, the closing line is on the other side: still filled.
        let flipped = kurbo::Affine::new([-1.0, 0.0, 0.0, 1.0, 200.0, 0.0]) * open.clone();
        assert!(fill_contains(&flipped, FillRule::NonZero, Point::new(100.0, 100.0)));
        // Two open subpaths: each closes on its own.
        let mut two = open.clone();
        two.extend(kurbo::Affine::translate((0.0, 100.0)) * open.clone());
        assert!(fill_contains(&two, FillRule::NonZero, Point::new(100.0, 100.0)) && fill_contains(&two, FillRule::NonZero, Point::new(100.0, 200.0)));
    }

    #[test]
    fn marquee() {
        let p = shapes::ellipse(Rect::new(0.0, 0.0, 10.0, 10.0));
        assert!(intersects_rect(&p, Rect::new(-1.0, 4.0, 1.0, 6.0)));
        assert!(!intersects_rect(&p, Rect::new(20.0, 20.0, 30.0, 30.0)));
        // corner region of the bbox that misses the ellipse outline
        assert!(!intersects_rect(&p, Rect::new(0.0, 0.0, 0.5, 0.5)));
        // Wholly inside the shape, touching neither its anchors nor its outline (#1103).
        assert!(!intersects_rect(&p, Rect::new(4.0, 4.0, 6.0, 6.0)));
        // Around the whole shape.
        assert!(intersects_rect(&p, Rect::new(-1.0, -1.0, 11.0, 11.0)));
        // crossing a line
        let l = shapes::line(Point::new(0.0, 0.0), Point::new(10.0, 10.0));
        assert!(intersects_rect(&l, Rect::new(4.0, 0.0, 6.0, 10.0)));
    }

    #[test]
    fn marquee_degenerate_rect() {
        let p = shapes::rectangle(Rect::new(0.0, 0.0, 10.0, 10.0));
        // zero-height / zero-width marquee crossing the shape
        assert!(intersects_rect(&p, Rect::new(-5.0, 5.0, 15.0, 5.0)));
        assert!(intersects_rect(&p, Rect::new(5.0, -5.0, 5.0, 15.0)));
        // zero-height marquee missing it
        assert!(!intersects_rect(&p, Rect::new(20.0, 5.0, 30.0, 5.0)));
    }
}

#[cfg(test)]
mod regress_tests {
    use super::*;
    use kurbo::{CubicBez, ParamCurve};

    /// A cubic whose handles sit on its anchors on one side (cusp-like): points on the curve are
    /// at distance ~0 (proptest regression).
    #[test]
    fn nearest_on_degenerate_cubic() {
        let c = CubicBez::new(
            (131.69052599341336, 172.64518298885875),
            (104.00879084385166, 172.64518298885875),
            (145.71215277397286, 141.82358989609716),
            (131.4188160942415, 162.38678549385997),
        );
        let mut bp = BezPath::new();
        bp.move_to(c.p0);
        bp.curve_to(c.p1, c.p2, c.p3);
        let worst = (0..=200).map(|i| distance_to_outline(&bp, c.eval(i as f64 / 200.0))).fold(0.0, f64::max);
        assert!(worst < 1e-6, "{worst}");
    }

    #[test]
    fn polygon_covers_rect_rejects_self_intersecting_star() {
        let mut b = BezPath::new();
        for k in 0..5 {
            let a = (-90.0 + 144.0 * k as f64).to_radians();
            let p = Point::new(100.0 * a.cos(), 100.0 * a.sin());
            if k == 0 {
                b.move_to(p);
            } else {
                b.line_to(p);
            }
        }
        b.close_path();
        assert!(!polygon_covers_rect(&b, Rect::new(-55.0, -20.0, 55.0, 78.0), 0.01));
    }
}
