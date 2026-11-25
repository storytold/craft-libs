//! Geometry property tests: PathData ↔ BezPath, bounds, hit testing, anchors and shapes.
// Integration tests: unwrapping and panicking on failure is fine here, unlike in shipped code (AGENTS.md › Robustness).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use craft_geom::hit::{distance_to_outline, fill_contains, stroke_contains};
use craft_geom::{Affine, BezPath, FillRule, PathData, PathEl, Point, Rect, Shape, Vec2, shapes};
use craft_testkit::geom::{grid, hausdorff, polygon_area, rect_contains, sample, sample_path};
use craft_testkit::strategies::{arb_closed_shape, arb_convex_polygon, arb_path_data, arb_point, arb_rect, arb_star_polygon, polygon_path};
use proptest::prelude::*;

fn els_close(a: &BezPath, b: &BezPath, eps: f64) -> bool {
    let (a, b) = (a.elements(), b.elements());
    if a.len() != b.len() {
        return false;
    }
    let pc = |p: Point, q: Point| p.distance(q) <= eps;
    a.iter().zip(b).all(|(x, y)| match (x, y) {
        (PathEl::MoveTo(p), PathEl::MoveTo(q)) | (PathEl::LineTo(p), PathEl::LineTo(q)) => pc(*p, *q),
        (PathEl::CurveTo(p1, p2, p3), PathEl::CurveTo(q1, q2, q3)) => pc(*p1, *q1) && pc(*p2, *q2) && pc(*p3, *q3),
        (PathEl::QuadTo(p1, p2), PathEl::QuadTo(q1, q2)) => pc(*p1, *q1) && pc(*p2, *q2),
        (PathEl::ClosePath, PathEl::ClosePath) => true,
        _ => false,
    })
}

/// Winding number computed independently: ray-cast crossings of a flattened polyline.
fn crossing_winding(bp: &BezPath, p: Point) -> i32 {
    let mut w = 0;
    let mut pts: Vec<Vec<Point>> = vec![];
    kurbo::flatten(bp, 1e-4, |el| match el {
        PathEl::MoveTo(q) => pts.push(vec![q]),
        PathEl::LineTo(q) => pts.last_mut().unwrap().push(q),
        PathEl::ClosePath => {}
        other => panic!("unexpected {other:?}"),
    });
    for poly in pts {
        for i in 0..poly.len() {
            let (a, b) = (poly[i], poly[(i + 1) % poly.len()]);
            if a.y <= p.y {
                if b.y > p.y && (b - a).cross(p - a) > 0.0 {
                    w += 1;
                }
            } else if b.y <= p.y && (b - a).cross(p - a) < 0.0 {
                w -= 1;
            }
        }
    }
    w
}

proptest! {
    #![proptest_config(ProptestConfig { cases: 256, failure_persistence: None, ..ProptestConfig::default() })]

    /// PathData → BezPath → PathData → BezPath is the identity on the Bézier geometry.
    #[test]
    fn pathdata_bezpath_roundtrip(p in arb_path_data()) {
        let bp = p.to_bezpath();
        let back = PathData::from_bezpath(&bp);
        prop_assert!(els_close(&bp, &back.to_bezpath(), 1e-9), "{bp:?}\nvs\n{:?}", back.to_bezpath());
        prop_assert_eq!(back.subpaths.len(), p.subpaths.len());
        for (a, b) in p.subpaths.iter().zip(&back.subpaths) {
            prop_assert_eq!(a.closed, b.closed);
            prop_assert_eq!(a.anchors.len(), b.anchors.len());
        }
    }

    /// kurbo shapes survive BezPath → PathData → BezPath.
    #[test]
    fn kurbo_shapes_roundtrip(r in arb_rect(), rad in 0.0..20.0f64) {
        for bp in [r.to_path(1e-6), kurbo::RoundedRect::from_rect(r, rad).to_path(1e-6), kurbo::Ellipse::from_rect(r).to_path(1e-6)] {
            let pd = PathData::from_bezpath(&bp);
            prop_assert!(hausdorff(&bp, &pd.to_bezpath(), 8) < 1e-6);
            prop_assert!((pd.to_bezpath().area() - bp.area()).abs() < 1e-6 * bp.area().abs().max(1.0));
        }
    }

    /// Tight bounds contain every sampled curve point and are attained (within a sampling slack).
    #[test]
    fn bounds_contain_samples(p in arb_path_data()) {
        let b = p.bounds().unwrap();
        let pts = sample_path(&p, 64);
        for q in &pts {
            prop_assert!(rect_contains(b, Rect::from_points(*q, *q), 1e-9), "{q:?} outside {b:?}");
        }
        let sb = pts.iter().fold(Rect::from_points(pts[0], pts[0]), |r, q| r.union_pt(*q));
        prop_assert!(rect_contains(sb, b, 0.5), "bounds {b:?} not tight vs samples {sb:?}");
        // Control bounds contain the tight bounds.
        prop_assert!(rect_contains(p.control_bounds().unwrap(), b, 1e-9));
    }

    /// Bounds commute with translation and uniform scale.
    #[test]
    fn bounds_transform_covariant(p in arb_path_data(), d in arb_point(-100.0, 100.0), s in 0.1..5.0f64) {
        let a = Affine::translate(d.to_vec2()) * Affine::scale(s);
        let b1 = p.transformed(a).bounds().unwrap();
        let b0 = p.bounds().unwrap();
        let want = Rect::new(b0.x0 * s + d.x, b0.y0 * s + d.y, b0.x1 * s + d.x, b0.y1 * s + d.y);
        let eps = 1e-7 * (1.0 + want.width().abs() + want.height().abs() + d.x.abs() + d.y.abs());
        prop_assert!(rect_contains(b1, want, eps) && rect_contains(want, b1, eps), "{b1:?} vs {want:?}");
    }

    /// fill_contains agrees with an independent crossing-number winding on random points.
    #[test]
    fn fill_contains_matches_winding(p in arb_closed_shape(), q in arb_point(0.0, 220.0)) {
        let bp = p.to_bezpath();
        // Skip points on (or extremely near) the outline, where flattening decides.
        prop_assume!(distance_to_outline(&bp, q) > 1e-2);
        let w = crossing_winding(&bp, q);
        prop_assert_eq!(fill_contains(&bp, FillRule::NonZero, q), w != 0);
        prop_assert_eq!(fill_contains(&bp, FillRule::EvenOdd, q), w % 2 != 0);
    }

    /// Even-odd ⊆ non-zero; reversing a path never changes containment.
    #[test]
    fn fill_rules_and_reverse(p in arb_path_data(), q in arb_point(0.0, 200.0)) {
        let bp = p.to_bezpath();
        prop_assume!(distance_to_outline(&bp, q) > 1e-3);
        if fill_contains(&bp, FillRule::EvenOdd, q) {
            prop_assert!(fill_contains(&bp, FillRule::NonZero, q));
        }
        let mut r = p.clone();
        r.reverse();
        let rb = r.to_bezpath();
        for rule in [FillRule::NonZero, FillRule::EvenOdd] {
            prop_assert_eq!(fill_contains(&bp, rule, q), fill_contains(&rb, rule, q));
        }
        // Reversal flips signed area, and reversing twice is the identity.
        prop_assert!((rb.area() + bp.area()).abs() < 1e-6 * bp.area().abs().max(1.0));
        r.reverse();
        prop_assert_eq!(r, p);
    }

    /// Convex polygon containment equals the half-plane test.
    #[test]
    fn convex_polygon_hit(pts in arb_convex_polygon(), q in arb_point(0.0, 200.0)) {
        let bp = polygon_path(&pts).to_bezpath();
        prop_assume!(distance_to_outline(&bp, q) > 1e-6);
        let n = pts.len();
        let sign = (pts[1] - pts[0]).cross(pts[2] - pts[1]).signum();
        let inside = (0..n).all(|i| (pts[(i + 1) % n] - pts[i]).cross(q - pts[i]) * sign > 0.0);
        prop_assert_eq!(fill_contains(&bp, FillRule::NonZero, q), inside);
    }

    /// Points sampled on the outline are within any positive stroke; distance is ~0 there.
    #[test]
    fn stroke_contains_outline(p in arb_path_data(), w in 0.1..10.0f64) {
        let bp = p.to_bezpath();
        for q in sample(&bp, 5) {
            prop_assert!(distance_to_outline(&bp, q) < 1e-6);
            prop_assert!(stroke_contains(&bp, w, 0.0, q));
        }
    }

    /// A point offset along the normal by more than half the width is outside the stroke of an
    /// isolated line.
    #[test]
    fn stroke_excludes_far_points(a in arb_point(0.0, 100.0), d in arb_point(10.0, 100.0), w in 0.5..10.0f64, t in 0.1..0.9f64) {
        let b = a + d.to_vec2();
        let line = shapes::line(a, b).to_bezpath();
        let dir = (b - a).normalize();
        let n = Vec2::new(-dir.y, dir.x);
        let m = a.lerp(b, t);
        prop_assert!(stroke_contains(&line, w, 0.0, m + n * (w / 2.0 * 0.9)));
        prop_assert!(!stroke_contains(&line, w, 0.0, m + n * (w / 2.0 * 1.1 + 1e-6)));
    }

    /// Inserting an anchor never changes the curve, adds exactly one anchor and keeps closure.
    #[test]
    fn insert_anchor_preserves_shape(p in arb_path_data(), t in 0.05..0.95f64, pick in 0usize..100) {
        let si = pick % p.subpaths.len();
        let seg_count = p.subpaths[si].segment_count();
        prop_assume!(seg_count > 0);
        let seg = pick % seg_count;
        let mut q = p.clone();
        q.subpaths[si].insert_anchor(seg, t);
        prop_assert_eq!(q.anchor_count(), p.anchor_count() + 1);
        prop_assert_eq!(q.subpaths[si].closed, p.subpaths[si].closed);
        prop_assert!(hausdorff(&p.to_bezpath(), &q.to_bezpath(), 16) < 1e-6);
        prop_assert!((q.length() - p.length()).abs() < 1e-3 * p.length().max(1.0));
    }

    /// Cutting at anchors keeps every segment as it was, in order (a closed subpath's from its
    /// first cut), in open pieces of two or more anchors. A smoothed corner gets handles on a line
    /// through it.
    #[test]
    fn cut_at_keeps_the_segments(p in arb_path_data(), cuts in proptest::collection::btree_set(0usize..12, 0..4), pick in 0usize..100) {
        let sp = &p.subpaths[pick % p.subpaths.len()];
        let n = sp.anchors.len();
        let pieces = sp.cut_at(&cuts);
        let segs = |s: &craft_geom::SubPath| (0..s.segment_count()).map(|i| s.segment(i)).collect::<Vec<_>>();
        let mut want = segs(sp);
        if pieces.len() > 1 || pieces.first().is_some_and(|q| q.closed != sp.closed) {
            prop_assert!(pieces.iter().all(|q| !q.closed && q.anchors.len() >= 2));
            if sp.closed {
                want.rotate_left(cuts.iter().copied().find(|&c| c < n).unwrap_or(0));
            }
        } else {
            prop_assert_eq!(&pieces, &vec![sp.clone()]);
        }
        prop_assert_eq!(pieces.iter().flat_map(segs).collect::<Vec<_>>(), want);
        let mut s = sp.clone();
        let i = pick % n.max(1);
        let corner = s.anchors.get(i).is_some_and(|a| a.kind == craft_geom::AnchorKind::Corner);
        if s.smooth_anchor(i) && corner {
            let a = s.anchors[i];
            let (u, v) = (a.h_in - a.p, a.h_out - a.p);
            prop_assert!(u.cross(v).abs() <= 1e-6 * (1.0 + u.hypot() * v.hypot()) && u.dot(v) <= 0.0, "{a:?}");
        }
    }

    /// Polygon area via the shoelace formula equals the path's signed area magnitude and
    /// SubPath::area.
    #[test]
    fn polygon_area_agrees(pts in arb_star_polygon()) {
        let p = polygon_path(&pts);
        let a = polygon_area(&pts);
        prop_assert!((p.to_bezpath().area().abs() - a).abs() < 1e-6 * a.max(1.0));
        prop_assert!((p.subpaths[0].area().abs() - a).abs() < 1e-6 * a.max(1.0));
    }

    /// Serde round trip of PathData is lossless in-memory (JSON Value).
    #[test]
    fn pathdata_serde_roundtrip(p in arb_path_data()) {
        let v = serde_json::to_value(&p).unwrap();
        let back: PathData = serde_json::from_value(v).unwrap();
        prop_assert_eq!(back, p);
    }

    /// Nearest point really is on the path and no sampled point is closer.
    #[test]
    fn nearest_is_nearest(p in arb_path_data(), q in arb_point(-20.0, 220.0)) {
        let (_, _, _, pt, d) = p.nearest(q).unwrap();
        prop_assert!((pt.distance(q) - d).abs() < 1e-6);
        let best_sample = sample_path(&p, 32).into_iter().map(|s| s.distance(q)).fold(f64::INFINITY, f64::min);
        prop_assert!(d <= best_sample + 1e-6, "nearest {d} > sampled {best_sample}");
    }

    /// Regular polygons and stars: anchor counts, all anchors on their radius, closed.
    #[test]
    fn polygon_and_star_shapes(c in arb_point(0.0, 100.0), r in 1.0..100.0f64, n in 3u32..20, rot in -360.0..360.0f64, k in 0.1..0.9f64) {
        let p = shapes::polygon(c, r, n, rot);
        prop_assert!(p.is_closed());
        prop_assert_eq!(p.anchor_count(), n as usize);
        for (_, _, a) in p.anchors() {
            prop_assert!((a.p.distance(c) - r).abs() < 1e-9 * r.max(1.0));
        }
        let s = shapes::star(c, r, r * k, n, rot);
        prop_assert_eq!(s.anchor_count(), 2 * n as usize);
        let ds: Vec<f64> = s.anchors().map(|(_, _, a)| a.p.distance(c)).collect();
        prop_assert!(ds.iter().all(|d| (d - r).abs() < 1e-9 * r || (d - r * k).abs() < 1e-9 * r));
    }

    /// Rectangles and ellipses: bounds equal the defining rect; area matches.
    #[test]
    fn rect_and_ellipse_shapes(r in arb_rect(), rad in 0.0..30.0f64) {
        let p = shapes::rectangle(r);
        prop_assert_eq!(p.bounds().unwrap(), r);
        prop_assert!((p.to_bezpath().area().abs() - r.area()).abs() < 1e-9 * r.area());
        let e = shapes::ellipse(r);
        let eb = e.bounds().unwrap();
        prop_assert!(rect_contains(eb, r, 1e-9) && rect_contains(r, eb, 1e-9));
        let ea = std::f64::consts::PI * r.width() * r.height() / 4.0;
        prop_assert!((e.to_bezpath().area().abs() - ea).abs() < 1e-3 * ea, "{} vs {ea}", e.to_bezpath().area());
        let rr = shapes::rounded_rectangle(r, rad);
        prop_assert!(rect_contains(r, rr.bounds().unwrap(), 1e-9));
        prop_assert!(rr.to_bezpath().area().abs() <= r.area() + 1e-9);
    }

    /// Length of a rectangle is its perimeter; of a line, its distance.
    #[test]
    fn lengths(r in arb_rect(), a in arb_point(0.0, 100.0), b in arb_point(0.0, 100.0)) {
        prop_assert!((shapes::rectangle(r).length() - 2.0 * (r.width() + r.height())).abs() < 1e-6);
        prop_assert!((shapes::line(a, b).length() - a.distance(b)).abs() < 1e-6);
    }
}

#[test]
fn from_bezpath_merges_duplicate_closing_point() {
    let mut bp = BezPath::new();
    bp.move_to((0.0, 0.0));
    bp.line_to((10.0, 0.0));
    bp.line_to((10.0, 10.0));
    bp.line_to((0.0, 0.0));
    bp.close_path();
    let p = PathData::from_bezpath(&bp);
    assert_eq!(p.anchor_count(), 3);
    assert!(p.is_closed());
}

#[test]
fn empty_and_degenerate_paths() {
    let e = PathData::default();
    assert!(e.is_empty());
    assert!(e.bounds().is_none());
    assert_eq!(e.length(), 0.0);
    assert!(e.nearest(Point::ZERO).is_none());
    let one = PathData::from_bezpath(&{
        let mut b = BezPath::new();
        b.move_to((5.0, 5.0));
        b
    });
    assert_eq!(one.anchor_count(), 1);
    assert_eq!(one.bounds(), Some(Rect::new(5.0, 5.0, 5.0, 5.0)));
    assert!(!fill_contains(&one.to_bezpath(), FillRule::NonZero, Point::new(5.0, 5.0)));
    let _ = grid(Rect::new(0.0, 0.0, 1.0, 1.0), 2);
}
