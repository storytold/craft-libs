//! Proptest strategies for geometry: points, rectangles, polygons, ellipses and whole paths.
//!
//! The engine-command strategies that sat beside these in VectorCraft's testkit stay in
//! VectorCraft: they generate editing-command sequences against its engine, which is app
//! scope, not kernel scope (RFC-0030, decision 5).

use craft_geom::{Anchor, PathData, Point, Rect, SubPath};
use proptest::prelude::*;

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

/// A convex polygon (regular polygon with jittered radius kept convex by using a single radius per
/// polygon and jittered angles only).
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

pub fn polygon_path(pts: &[Point]) -> PathData {
    PathData::single(SubPath::polyline(pts, true))
}

/// An axis-aligned ellipse as a path.
pub fn arb_ellipse() -> impl Strategy<Value = PathData> {
    arb_rect().prop_map(craft_geom::shapes::ellipse)
}

/// Rectangles, ellipses and star-shaped polygons.
pub fn arb_closed_shape() -> impl Strategy<Value = PathData> {
    prop_oneof![arb_rect().prop_map(craft_geom::shapes::rectangle), arb_ellipse(), arb_star_polygon().prop_map(|p| polygon_path(&p)),]
}

/// A random cubic Bézier path (open or closed, possibly several subpaths, arbitrary handles).
pub fn arb_path_data() -> impl Strategy<Value = PathData> {
    let anchor = (arb_point(0.0, 200.0), arb_point(-30.0, 30.0), arb_point(-30.0, 30.0), any::<bool>())
        .prop_map(|(p, i, o, corner)| if corner { Anchor::corner(p) } else { Anchor::with_handles(p, p + i.to_vec2(), p + o.to_vec2()) });
    let sub = (prop::collection::vec(anchor, 2..8), any::<bool>()).prop_map(|(a, closed)| SubPath::new(a, closed));
    prop::collection::vec(sub, 1..3).prop_map(PathData::new)
}

/// A smooth open curve through points on a gently varying function (for simplify tests): densely
/// sampled polyline-like cubic chain with C1 handles.
pub fn arb_smooth_curve() -> impl Strategy<Value = PathData> {
    (prop::collection::vec(-20.0f64..20.0, 4..9), 10.0f64..30.0).prop_map(|(ys, step)| {
        // Catmull-Rom through (i*step, y_i), converted to cubic handles.
        let pts: Vec<Point> = ys.iter().enumerate().map(|(i, y)| Point::new(i as f64 * step, 100.0 + y)).collect();
        let n = pts.len();
        let tangent = |i: usize| {
            let a = pts[i.saturating_sub(1)];
            let b = pts[(i + 1).min(n - 1)];
            (b - a) / 6.0
        };
        let anchors = (0..n).map(|i| Anchor::with_handles(pts[i], pts[i] - tangent(i), pts[i] + tangent(i))).collect();
        PathData::single(SubPath::new(anchors, false))
    })
}
