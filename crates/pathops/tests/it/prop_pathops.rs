//! Path-operation properties on concave polygons, ellipses and random smooth curves (complements
//! `props.rs`, which uses rectangles and circles).

use crate::common;

use craft_geom::{FillRule, PathData, Point, Rect};
use craft_pathops::*;
use craft_testkit::geom::{directed_hausdorff, grid, hausdorff, polygon_area, rel_close};
use craft_testkit::strategies::{arb_closed_shape, arb_convex_polygon, arb_ellipse, arb_rect, arb_smooth_curve, arb_star_polygon, polygon_path};
use proptest::prelude::*;

const NZ: FillRule = FillRule::NonZero;

fn inside(p: &PathData, q: Point) -> bool {
    craft_geom::hit::fill_contains(&p.to_bezpath(), NZ, q)
}
fn near_edge(p: &PathData, q: Point, eps: f64) -> bool {
    craft_geom::hit::distance_to_outline(&p.to_bezpath(), q) < eps
}

proptest! {
    #![proptest_config(common::config(128))]

    /// Area of a simple polygon equals its shoelace area; normalize keeps it.
    #[test]
    fn polygon_area_matches_shoelace(pts in arb_star_polygon()) {
        let p = polygon_path(&pts);
        let a = polygon_area(&pts);
        prop_assert!(rel_close(area(&p, NZ), a, 1e-6), "{} vs {a}", area(&p, NZ));
        prop_assert!(rel_close(area(&normalize(&p, NZ), NZ), a, 1e-4));
    }

    /// A = (A − B) + (A ∩ B); union/intersection commute; results never exceed their bounds.
    #[test]
    fn area_identities(a in arb_closed_shape(), b in arb_closed_shape()) {
        let (aa, ab) = (area(&a, NZ), area(&b, NZ));
        let d = area(&boolean(&a, NZ, &b, NZ, BoolOp::Difference), NZ);
        let i = area(&boolean(&a, NZ, &b, NZ, BoolOp::Intersect), NZ);
        let i2 = area(&boolean(&b, NZ, &a, NZ, BoolOp::Intersect), NZ);
        let u = area(&boolean(&a, NZ, &b, NZ, BoolOp::Union), NZ);
        let u2 = area(&boolean(&b, NZ, &a, NZ, BoolOp::Union), NZ);
        let x = area(&boolean(&a, NZ, &b, NZ, BoolOp::Xor), NZ);
        let tol = 2e-3;
        prop_assert!(rel_close(d + i, aa, tol), "d {d} + i {i} != a {aa}");
        prop_assert!(rel_close(i, i2, tol) && rel_close(u, u2, tol));
        prop_assert!(rel_close(x, u - i, tol), "xor {x} != u-i {}", u - i);
        prop_assert!(i <= aa.min(ab) * (1.0 + tol) + 1e-6);
        prop_assert!(u + 1e-6 >= aa.max(ab) * (1.0 - tol));
        prop_assert!(u <= (aa + ab) * (1.0 + tol) + 1e-6);
    }

    /// A ∪ A = A ∩ A = A, A − A = ∅.
    #[test]
    fn self_ops(a in arb_closed_shape()) {
        let aa = area(&a, NZ);
        prop_assert!(rel_close(area(&boolean(&a, NZ, &a, NZ, BoolOp::Union), NZ), aa, 1e-3));
        prop_assert!(rel_close(area(&boolean(&a, NZ, &a, NZ, BoolOp::Intersect), NZ), aa, 1e-3));
        prop_assert!(area(&boolean(&a, NZ, &a, NZ, BoolOp::Difference), NZ) < 1e-3 * aa.max(1.0));
    }

    /// Point membership of boolean results agrees with the set operation on probe points away
    /// from any edge.
    #[test]
    fn boolean_membership(a in arb_closed_shape(), b in arb_closed_shape()) {
        let ops = [BoolOp::Union, BoolOp::Intersect, BoolOp::Difference, BoolOp::Xor];
        let results: Vec<PathData> = ops.iter().map(|op| boolean(&a, NZ, &b, NZ, *op)).collect();
        for q in grid(Rect::new(0.0, 0.0, 230.0, 230.0), 23) {
            if near_edge(&a, q, 0.5) || near_edge(&b, q, 0.5) {
                continue;
            }
            let (ia, ib) = (inside(&a, q), inside(&b, q));
            let want = [ia || ib, ia && ib, ia && !ib, ia != ib];
            for (k, r) in results.iter().enumerate() {
                prop_assert_eq!(inside(r, q), want[k], "{:?} at {:?}", ops[k], q);
            }
        }
    }

    /// Divide faces partition the union: areas sum to the union area, and every probe point in
    /// the union lies in exactly one face; each face's sources are exactly the shapes covering it.
    #[test]
    fn divide_partitions_union(v in prop::collection::vec(arb_closed_shape(), 2..5)) {
        let shapes: Vec<Shape> = v.iter().enumerate().map(|(i, p)| Shape::new(p.clone(), NZ, i as u64)).collect();
        let faces = pathfinder(PathfinderOp::Divide, &shapes);
        let sum: f64 = faces.iter().map(|f| area(&f.path, NZ)).sum();
        let u = pathfinder(PathfinderOp::Unite, &shapes).first().map(|s| area(&s.path, NZ)).unwrap_or(0.0);
        prop_assert!(rel_close(sum, u, 3e-3), "faces {sum} vs union {u}");
        let regs = regions(&shapes);
        for q in grid(Rect::new(0.0, 0.0, 230.0, 230.0), 17) {
            if v.iter().any(|p| near_edge(p, q, 0.5)) {
                continue;
            }
            let covering: Vec<usize> = v.iter().enumerate().filter(|(_, p)| inside(p, q)).map(|(i, _)| i).collect();
            let hits: Vec<&Region> = regs.iter().filter(|r| r.contains(q)).collect();
            if covering.is_empty() {
                prop_assert!(hits.is_empty(), "uncovered point {q:?} in a region");
            } else {
                prop_assert_eq!(hits.len(), 1, "point {:?} in {} regions", q, hits.len());
                prop_assert_eq!(&hits[0].sources, &covering);
            }
        }
    }

    /// Offset monotonicity: a larger offset contains a smaller one (area of smaller − larger ≈ 0)
    /// and areas are ordered.
    #[test]
    fn offset_monotone(p in prop_oneof![arb_ellipse(), arb_convex_polygon().prop_map(|v| polygon_path(&v)), arb_star_polygon().prop_map(|v| polygon_path(&v))],
                       d1 in -3.0..8.0f64, dd in 0.5..6.0f64) {
        let d2 = d1 + dd;
        // Large offsets of small curved shapes are a known bug (bug_offset_large_delta_loses_area).
        let b = p.bounds().unwrap();
        prop_assume!(b.width().min(b.height()) >= 2.0 * d2.abs());
        for join in [Join::Round, Join::Miter, Join::Bevel] {
            let small = offset_path(&p, d1, join, 4.0);
            let big = offset_path(&p, d2, join, 4.0);
            let (sa, ba) = (area(&small, NZ), area(&big, NZ));
            prop_assert!(ba + 1e-6 >= sa, "{join:?}: area({d2}) {ba} < area({d1}) {sa}");
            let leak = area(&boolean(&small, NZ, &big, NZ, BoolOp::Difference), NZ);
            prop_assert!(leak <= 2e-3 * sa.max(1.0), "{join:?}: offset {d1} leaks {leak} outside offset {d2}");
        }
        // A positive offset contains the original.
        if d2 > 0.0 {
            let big = offset_path(&p, d2, Join::Round, 4.0);
            let leak = area(&boolean(&p, NZ, &big, NZ, BoolOp::Difference), NZ);
            prop_assert!(leak <= 2e-3 * area(&p, NZ).max(1.0));
        }
    }

    /// Round-join offset of a convex polygon follows Steiner's formula A + P·d + π·d².
    #[test]
    fn offset_convex_steiner(pts in arb_convex_polygon(), d in 0.5..10.0f64) {
        let p = polygon_path(&pts);
        let perim: f64 = (0..pts.len()).map(|i| pts[i].distance(pts[(i + 1) % pts.len()])).sum();
        let want = polygon_area(&pts) + perim * d + std::f64::consts::PI * d * d;
        let got = area(&offset_path(&p, d, Join::Round, 4.0), NZ);
        prop_assert!(rel_close(got, want, 5e-3), "{got} vs {want}");
    }

    /// Offsetting by 0 keeps the shape.
    #[test]
    fn offset_zero_identity(p in arb_closed_shape()) {
        let o = offset_path(&p, 0.0, Join::Miter, 4.0);
        prop_assert!(rel_close(area(&o, NZ), area(&p, NZ), 2e-3));
    }

    /// Simplify stays within tolerance (plus a small fitting slack). (It should also never add
    /// anchors — see `bug_simplify_adds_anchors`.)
    #[test]
    fn simplify_error_within_tolerance(c in arb_smooth_curve(), tol in 0.2..5.0f64) {
        let s = simplify(&c, tol);
        let (a, b) = (c.to_bezpath(), s.to_bezpath());
        let err = hausdorff(&a, &b, 24);
        prop_assert!(err <= tol * 1.1 + 1e-6, "simplify error {err} > tol {tol}");
        // Endpoints are preserved on open paths.
        let (c0, s0) = (&c.subpaths[0], &s.subpaths[0]);
        prop_assert!(c0.anchors[0].p.distance(s0.anchors[0].p) < 1e-9);
        prop_assert!(c0.anchors.last().unwrap().p.distance(s0.anchors.last().unwrap().p) < 1e-9);
    }

    /// Straight-line simplify produces only line segments within tolerance.
    #[test]
    fn simplify_straight_lines(c in arb_smooth_curve(), tol in 0.5..5.0f64) {
        let s = simplify_with(&c, &SimplifyOptions { tolerance: tol, straight_lines: true, ..Default::default() });
        for sp in &s.subpaths {
            for i in 0..sp.segment_count() {
                prop_assert!(sp.segment_is_line(i));
            }
        }
        prop_assert!(directed_hausdorff(&s.to_bezpath(), &c.to_bezpath(), 16) <= tol * 1.1 + 1e-6);
    }

    /// add_anchor_points doubles segment count without changing geometry.
    #[test]
    fn add_anchor_points_preserves(p in arb_closed_shape()) {
        let q = add_anchor_points(&p);
        let segs = |p: &PathData| p.subpaths.iter().map(|s| s.segment_count()).sum::<usize>();
        prop_assert_eq!(segs(&q), 2 * segs(&p));
        prop_assert!(hausdorff(&p.to_bezpath(), &q.to_bezpath(), 8) < 1e-6);
    }

    /// Split into grid: rows×cols cells whose total area is the rect minus gutters.
    #[test]
    fn split_into_grid_areas(r in arb_rect(), rows in 1usize..6, cols in 1usize..6, g in 0.0..2.0f64) {
        let gutter_w = g * (cols - 1) as f64;
        let gutter_h = g * (rows - 1) as f64;
        prop_assume!(r.width() > gutter_w + 1.0 && r.height() > gutter_h + 1.0);
        let cells = split_into_grid(r, rows, cols, (g, g));
        prop_assert_eq!(cells.len(), rows * cols);
        let total: f64 = cells.iter().map(|c| area(c, NZ)).sum();
        let want = (r.width() - gutter_w) * (r.height() - gutter_h);
        prop_assert!(rel_close(total, want, 1e-6), "{total} vs {want}");
    }

    /// Outline stroke of a closed convex polygon (round joins) has area ≈ perimeter × width + π(w/2)²·0
    /// (ring area = offset(+w/2) − offset(−w/2)).
    #[test]
    fn outline_stroke_ring_area(pts in arb_convex_polygon(), w in 0.5..4.0f64) {
        let p = polygon_path(&pts);
        let ring = area(&outline_stroke(&p, w, Cap::Butt, Join::Round, 4.0), NZ);
        let outer = area(&offset_path(&p, w / 2.0, Join::Round, 4.0), NZ);
        let inner = area(&offset_path(&p, -w / 2.0, Join::Round, 4.0), NZ);
        prop_assert!(rel_close(ring, outer - inner, 1e-2), "ring {ring} vs {}", outer - inner);
    }
}

#[test]
fn non_finite_input_is_an_error() {
    let mut p = craft_geom::shapes::rectangle(Rect::new(0.0, 0.0, 10.0, 10.0));
    p.subpaths[0].anchors[1].p.x = f64::NAN;
    let q = craft_geom::shapes::rectangle(Rect::new(5.0, 5.0, 15.0, 15.0));
    assert_eq!(try_boolean(&p, NZ, &q, NZ, BoolOp::Union, DEFAULT_PRECISION), Err(PathOpsError::NonFinite));
    assert!(boolean(&p, NZ, &q, NZ, BoolOp::Union).is_empty());
    assert!(offset_path(&q, f64::NAN, Join::Round, 4.0).is_empty());
    assert!(outline_stroke(&q, f64::INFINITY, Cap::Butt, Join::Round, 4.0).is_empty());
}

#[test]
fn empty_inputs() {
    let e = PathData::default();
    let q = craft_geom::shapes::rectangle(Rect::new(5.0, 5.0, 15.0, 15.0));
    assert!(rel_close(area(&boolean(&e, NZ, &q, NZ, BoolOp::Union), NZ), 100.0, 1e-9));
    assert!(boolean(&e, NZ, &q, NZ, BoolOp::Intersect).is_empty());
    assert!(pathfinder(PathfinderOp::Divide, &[]).is_empty());
    assert!(regions(&[]).is_empty());
    assert!(simplify(&e, 1.0).is_empty());
}

#[test]
fn bug_simplify_adds_anchors() {
    use craft_geom::{Anchor, SubPath};
    let a = |p: (f64, f64), i: (f64, f64), o: (f64, f64)| Anchor::with_handles(p.into(), i.into(), o.into());
    let c = PathData::single(SubPath::new(
        vec![
            a((0.0, 100.0), (-4.964818950742777, 102.85147018094767), (4.964818950742777, 97.14852981905233)),
            a((29.788913704456665, 82.89117891431397), (19.859275802971112, 82.89117891431397), (39.71855160594222, 82.89117891431397)),
            a((59.57782740891333, 100.0), (49.64818950742777, 94.34401856927909), (69.50746531039889, 105.65598143072091)),
            a((89.36674111337, 116.82706749863948), (79.43710321188445, 116.82706749863948), (99.29637901485555, 116.82706749863948)),
            a((119.15565481782666, 100.0), (109.22601691634111, 102.80451124977324), (129.0852927193122, 97.19548875022676)),
            a((148.94456852228333, 100.0), (143.97974957154057, 100.0), (153.9093874730261, 100.0)),
        ],
        false,
    ));
    let s = simplify(&c, 0.2);
    assert!(s.anchor_count() <= c.anchor_count(), "{} anchors in, {} out", c.anchor_count(), s.anchor_count());
}

#[test]
fn offset_large_delta_keeps_area() {
    for (r, d) in [(1.0, 5.0), (1.0, 3.0), (10.0, 30.0)] {
        let p = craft_geom::shapes::ellipse(Rect::new(0.0, 0.0, 2.0 * r, 2.0 * r));
        let got = area(&offset_path(&p, d, Join::Round, 4.0), NZ);
        let want = std::f64::consts::PI * (r + d) * (r + d);
        assert!(rel_close(got, want, 1e-2), "r={r} d={d}: {got} vs {want}");
    }
}
