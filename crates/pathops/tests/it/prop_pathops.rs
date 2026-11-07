// Carved from VectorCraft's crates/pathops (tests/it/prop_pathops.rs) at storytold/vectorcraft ff9deee46d1c59dc9ecef0b668c25e57cd80b55e.
// Copyright (c) 2026 ArtCraft Team and the VectorCraft contributors. MIT OR Apache-2.0.

//! Path-operation properties on concave polygons, ellipses and random smooth curves (complements
//! `props.rs`, which uses rectangles and circles).

use crate::common;

use craft_pathops::*;
use kurbo::{BezPath, Point, Rect, Shape as _};
use proptest::prelude::*;

use crate::common::{
    arb_closed_shape, arb_convex_polygon, arb_ellipse, arb_smooth_curve, arb_star_polygon, directed_hausdorff, distance_to, ends, fill_contains,
    grid, hausdorff, polygon_area, polygon_path, rel_close,
};

const NZ: FillRule = FillRule::NonZero;

fn inside(p: &BezPath, q: Point) -> bool {
    fill_contains(p, NZ, q)
}
fn near_edge(p: &BezPath, q: Point, eps: f64) -> bool {
    distance_to(p, q) < eps
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
        let results: Vec<BezPath> = ops.iter().map(|op| boolean(&a, NZ, &b, NZ, *op)).collect();
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
        let b = p.bounding_box();
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
    /// anchors, see `bug_simplify_adds_anchors`.)
    #[test]
    fn simplify_error_within_tolerance(c in arb_smooth_curve(), tol in 0.2..5.0f64) {
        let s = simplify(&c, tol);
        let err = hausdorff(&c, &s, 24);
        prop_assert!(err <= tol * 1.1 + 1e-6, "simplify error {err} > tol {tol}");
        // Endpoints are preserved on open paths.
        let ((c0, c1), (s0, s1)) = (ends(&c), ends(&s));
        prop_assert!(c0.distance(s0) < 1e-9);
        prop_assert!(c1.distance(s1) < 1e-9);
    }

    /// Straight-line simplify produces only line segments within tolerance.
    #[test]
    fn simplify_straight_lines(c in arb_smooth_curve(), tol in 0.5..5.0f64) {
        let s = simplify_with(&c, &SimplifyOptions::default().tolerance(tol).straight_lines(true));
        prop_assert!(!common::has_curves(&s));
        prop_assert!(directed_hausdorff(&s, &c, 16) <= tol * 1.1 + 1e-6);
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
    let p = common::poly(&[(0.0, 0.0), (f64::NAN, 0.0), (10.0, 10.0), (0.0, 10.0)], true);
    let q = common::rectangle(Rect::new(5.0, 5.0, 15.0, 15.0));
    assert_eq!(try_boolean(&p, NZ, &q, NZ, BoolOp::Union, DEFAULT_PRECISION), Err(PathOpsError::NonFinite));
    assert!(boolean(&p, NZ, &q, NZ, BoolOp::Union).elements().is_empty());
    assert!(offset_path(&q, f64::NAN, Join::Round, 4.0).elements().is_empty());
    assert!(outline_stroke(&q, f64::INFINITY, Cap::Butt, Join::Round, 4.0).elements().is_empty());
}

#[test]
fn empty_inputs() {
    let e = BezPath::new();
    let q = common::rectangle(Rect::new(5.0, 5.0, 15.0, 15.0));
    assert!(rel_close(area(&boolean(&e, NZ, &q, NZ, BoolOp::Union), NZ), 100.0, 1e-9));
    assert!(boolean(&e, NZ, &q, NZ, BoolOp::Intersect).elements().is_empty());
    assert!(pathfinder(PathfinderOp::Divide, &[]).is_empty());
    assert!(regions(&[]).is_empty());
    assert!(simplify(&e, 1.0).elements().is_empty());
}

#[test]
fn bug_simplify_adds_anchors() {
    // Anchors as (point, in handle, out handle).
    let a: [[(f64, f64); 3]; 6] = [
        [(0.0, 100.0), (-4.964818950742777, 102.85147018094767), (4.964818950742777, 97.14852981905233)],
        [(29.788913704456665, 82.89117891431397), (19.859275802971112, 82.89117891431397), (39.71855160594222, 82.89117891431397)],
        [(59.57782740891333, 100.0), (49.64818950742777, 94.34401856927909), (69.50746531039889, 105.65598143072091)],
        [(89.36674111337, 116.82706749863948), (79.43710321188445, 116.82706749863948), (99.29637901485555, 116.82706749863948)],
        [(119.15565481782666, 100.0), (109.22601691634111, 102.80451124977324), (129.0852927193122, 97.19548875022676)],
        [(148.94456852228333, 100.0), (143.97974957154057, 100.0), (153.9093874730261, 100.0)],
    ];
    let mut c = BezPath::new();
    c.move_to(a[0][0]);
    for w in a.windows(2) {
        c.curve_to(w[0][2], w[1][1], w[1][0]);
    }
    let s = simplify(&c, 0.2);
    assert!(common::anchor_count(&s) <= common::anchor_count(&c), "{} anchors in, {} out", common::anchor_count(&c), common::anchor_count(&s));
}

#[test]
fn offset_large_delta_keeps_area() {
    for (r, d) in [(1.0, 5.0), (1.0, 3.0), (10.0, 30.0)] {
        let p = common::ellipse(Rect::new(0.0, 0.0, 2.0 * r, 2.0 * r));
        let got = area(&offset_path(&p, d, Join::Round, 4.0), NZ);
        let want = std::f64::consts::PI * (r + d) * (r + d);
        assert!(rel_close(got, want, 1e-2), "r={r} d={d}: {got} vs {want}");
    }
}
