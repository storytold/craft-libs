// Carved from VectorCraft's crates/pathops (tests/it/props.rs) at storytold/vectorcraft ff9deee46d1c59dc9ecef0b668c25e57cd80b55e.
// Copyright (c) 2026 ArtCraft Team and the VectorCraft contributors. MIT OR Apache-2.0.

use crate::common;

use std::f64::consts::PI;

use craft_pathops::*;
use kurbo::{BezPath, ParamCurve, Point, Rect, Shape as _};
use proptest::prelude::*;

use crate::common::{anchor_count, distance_to, rel_close as close};

const NZ: FillRule = FillRule::NonZero;

fn arb_shape() -> impl Strategy<Value = BezPath> {
    prop_oneof![
        (0.0..50.0f64, 0.0..50.0f64, 1.0..40.0f64, 1.0..40.0f64).prop_map(|(x, y, w, h)| Rect::new(x, y, x + w, y + h).to_path(1e-6)),
        (0.0..50.0f64, 0.0..50.0f64, 1.0..25.0f64).prop_map(|(x, y, r)| kurbo::Circle::new((x, y), r).to_path(1e-4)),
    ]
}

proptest! {
    #![proptest_config(common::config(48))]

    #[test]
    fn inclusion_exclusion(a in arb_shape(), b in arb_shape()) {
        let u = area(&boolean(&a, NZ, &b, NZ, BoolOp::Union), NZ);
        let i = area(&boolean(&a, NZ, &b, NZ, BoolOp::Intersect), NZ);
        let (aa, ab) = (area(&a, NZ), area(&b, NZ));
        prop_assert!(close(u + i, aa + ab, 2e-3), "u={u} i={i} a={aa} b={ab}");
    }

    #[test]
    fn grid_rects_with_shared_edges(v in proptest::collection::vec((0u8..6, 0u8..6, 1u8..5, 1u8..5), 2..6)) {
        // Integer rectangles: lots of coincident edges and touching corners.
        let shapes: Vec<Shape> = v.iter().enumerate().map(|(i, &(x, y, w, h))| {
            let r = Rect::new(x as f64, y as f64, (x + w) as f64, (y + h) as f64);
            Shape::new(r.to_path(1e-6), NZ, i as u64)
        }).collect();
        let faces = pathfinder(PathfinderOp::Divide, &shapes);
        let sum: f64 = faces.iter().map(|f| area(&f.path, NZ)).sum();
        let u = pathfinder(PathfinderOp::Unite, &shapes);
        let ua = u.first().map(|s| area(&s.path, NZ)).unwrap_or(0.0);
        // Exact cell count on the integer grid.
        let mut cells = 0;
        for cx in 0..12u8 { for cy in 0..12u8 {
            if v.iter().any(|&(x, y, w, h)| cx >= x && cx < x + w && cy >= y && cy < y + h) { cells += 1; }
        }}
        prop_assert!(close(ua, cells as f64, 1e-9), "union {ua} cells {cells}");
        prop_assert!(close(sum, cells as f64, 1e-9), "divide {sum} cells {cells}");
        let trim: f64 = pathfinder(PathfinderOp::Trim, &shapes).iter().map(|f| area(&f.path, NZ)).sum();
        prop_assert!(close(trim, cells as f64, 1e-9));
    }

    #[test]
    fn difference_and_xor_consistent(a in arb_shape(), b in arb_shape()) {
        let d1 = area(&boolean(&a, NZ, &b, NZ, BoolOp::Difference), NZ);
        let d2 = area(&boolean(&b, NZ, &a, NZ, BoolOp::Difference), NZ);
        let x = area(&boolean(&a, NZ, &b, NZ, BoolOp::Xor), NZ);
        prop_assert!(close(d1 + d2, x, 2e-3));
    }

    #[test]
    fn self_difference_empty(a in arb_shape()) {
        prop_assert!(boolean(&a, NZ, &a, NZ, BoolOp::Difference).elements().is_empty());
    }

    #[test]
    fn unite_idempotent(a in arb_shape()) {
        let u = boolean(&a, NZ, &a, NZ, BoolOp::Union);
        prop_assert!(close(area(&u, NZ), area(&a, NZ), 1e-3));
        prop_assert!(anchor_count(&u) <= anchor_count(&a) + 4, "{} vs {}", anchor_count(&u), anchor_count(&a));
        let uu = boolean(&u, NZ, &u, NZ, BoolOp::Union);
        prop_assert!(close(area(&uu, NZ), area(&u, NZ), 1e-6));
    }

    #[test]
    fn divide_sums_to_union(a in arb_shape(), b in arb_shape(), c in arb_shape()) {
        let shapes = vec![Shape::new(a, NZ, 1), Shape::new(b, NZ, 2), Shape::new(c, NZ, 3)];
        let faces = pathfinder(PathfinderOp::Divide, &shapes);
        let sum: f64 = faces.iter().map(|f| area(&f.path, NZ)).sum();
        let u = pathfinder(PathfinderOp::Unite, &shapes);
        let ua = u.first().map(|s| area(&s.path, NZ)).unwrap_or(0.0);
        prop_assert!(close(sum, ua, 2e-3), "sum={sum} union={ua}");
    }

    #[test]
    fn offset_circle_area(r in 5.0..60.0f64, d in -4.0..10.0f64) {
        let c = kurbo::Circle::new((0.0, 0.0), r).to_path(1e-5);
        let o = offset_path(&c, d, Join::Round, 4.0);
        let expect = PI * (r + d) * (r + d);
        prop_assert!(close(area(&o, NZ), expect, 3e-3), "got {} expect {}", area(&o, NZ), expect);
    }

    #[test]
    fn outline_stroke_line_area(x in -50.0..50.0f64, y in -50.0..50.0f64, len in 1.0..100.0f64, ang in 0.0..6.0f64, w in 0.5..10.0f64) {
        let a = Point::new(x, y);
        let b = Point::new(x + len * ang.cos(), y + len * ang.sin());
        let l = common::line(a, b);
        let o = outline_stroke(&l, w, Cap::Butt, Join::Miter, 4.0);
        prop_assert!(close(area(&o, NZ), len * w, 1e-6));
    }

    #[test]
    fn simplify_within_tolerance(
        amps in proptest::collection::vec(0.0..8.0f64, 3),
        tol in 0.05..2.0f64,
    ) {
        // A wobbly closed curve sampled as a dense polyline.
        let n = 240;
        let pts: Vec<Point> = (0..n).map(|i| {
            let t = i as f64 / n as f64 * 2.0 * PI;
            let r = 60.0 + amps[0] * (2.0 * t).sin() + amps[1] * (3.0 * t).cos() + amps[2] * (5.0 * t).sin();
            Point::new(r * t.cos(), r * t.sin())
        }).collect();
        let p = common::polyline(&pts, true);
        let s = simplify(&p, tol);
        prop_assert!(anchor_count(&s) < n);
        for seg in p.segments() {
            for k in 0..=4 {
                let q = seg.eval(k as f64 / 4.0);
                let d = distance_to(&s, q);
                prop_assert!(d <= tol * 1.01 + 1e-6, "deviation {d} > {tol}");
            }
        }
        // And no part of the result is far from the original either.
        for seg in s.segments() {
            for k in 0..=4 {
                let q = seg.eval(f64::from(k) / 4.0);
                let d = distance_to(&p, q);
                prop_assert!(d <= tol * 1.01 + 1e-6, "result {d} from the original, over {tol}");
            }
        }
    }
}
