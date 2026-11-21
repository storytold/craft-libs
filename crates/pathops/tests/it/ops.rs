// Carved from VectorCraft's crates/pathops (tests/it/ops.rs) at storytold/vectorcraft ff9deee46d1c59dc9ecef0b668c25e57cd80b55e.
// Copyright (c) 2026 ArtCraft Team and the VectorCraft contributors. MIT OR Apache-2.0.

use std::f64::consts::PI;

use craft_pathops::*;
use kurbo::{BezPath, ParamCurve, ParamCurveArclen, Point, Rect, Shape as _};

use crate::common::{all_closed, anchor_count, any_closed, distance_to, ends, has_curves, poly, polyline, rel_close as close, split, subpath_count};

const NZ: FillRule = FillRule::NonZero;

fn rect(x: f64, y: f64, w: f64, h: f64) -> BezPath {
    Rect::new(x, y, x + w, y + h).to_path(1e-6)
}

fn circle(cx: f64, cy: f64, r: f64) -> BezPath {
    kurbo::Circle::new((cx, cy), r).to_path(1e-4)
}

fn ar(p: &BezPath) -> f64 {
    area(p, NZ)
}

fn length(p: &BezPath) -> f64 {
    p.segments().map(|s| s.arclen(1e-6)).sum()
}

fn shape(p: BezPath, key: u64) -> Shape {
    Shape::new(p, NZ, key)
}

#[test]
fn union_of_rects() {
    let u = boolean(&rect(0.0, 0.0, 10.0, 10.0), NZ, &rect(5.0, 5.0, 10.0, 10.0), NZ, BoolOp::Union);
    assert!(close(ar(&u), 175.0, 1e-9));
    assert_eq!(subpath_count(&u), 1);
    assert_eq!(anchor_count(&u), 8);
}

#[test]
fn intersect_of_rects() {
    let i = boolean(&rect(0.0, 0.0, 10.0, 10.0), NZ, &rect(5.0, 5.0, 10.0, 10.0), NZ, BoolOp::Intersect);
    assert!(close(ar(&i), 25.0, 1e-9));
    assert_eq!(anchor_count(&i), 4);
}

#[test]
fn difference_of_rects() {
    let d = boolean(&rect(0.0, 0.0, 10.0, 10.0), NZ, &rect(5.0, 5.0, 10.0, 10.0), NZ, BoolOp::Difference);
    assert!(close(ar(&d), 75.0, 1e-9));
    assert_eq!(anchor_count(&d), 6);
}

#[test]
fn xor_of_rects() {
    let x = boolean(&rect(0.0, 0.0, 10.0, 10.0), NZ, &rect(5.0, 5.0, 10.0, 10.0), NZ, BoolOp::Xor);
    assert!(close(ar(&x), 150.0, 1e-9));
}

#[test]
fn disjoint_union_keeps_both() {
    let u = boolean(&rect(0.0, 0.0, 1.0, 1.0), NZ, &rect(5.0, 5.0, 1.0, 1.0), NZ, BoolOp::Union);
    assert_eq!(subpath_count(&u), 2);
    assert!(close(ar(&u), 2.0, 1e-9));
}

#[test]
fn a_minus_a_is_empty() {
    let c = circle(3.0, 4.0, 7.0);
    assert!(boolean(&c, NZ, &c, NZ, BoolOp::Difference).elements().is_empty());
}

#[test]
fn union_idempotent_rect() {
    let r = rect(1.0, 2.0, 3.0, 4.0);
    let u = boolean(&r, NZ, &r, NZ, BoolOp::Union);
    assert_eq!(anchor_count(&u), 4);
    assert!(close(ar(&u), 12.0, 1e-9));
}

#[test]
fn circle_union_is_curve_preserving_and_compact() {
    let u = boolean(&circle(0.0, 0.0, 10.0), NZ, &circle(10.0, 0.0, 10.0), NZ, BoolOp::Union);
    assert!(has_curves(&u));
    assert!(anchor_count(&u) <= 12, "anchors: {}", anchor_count(&u));
    // Lens area: 2 r² acos(d/2r) − (d/2)√(4r² − d²)
    let (r, d): (f64, f64) = (10.0, 10.0);
    let lens = 2.0 * r * r * (d / (2.0 * r)).acos() - d / 2.0 * (4.0 * r * r - d * d).sqrt();
    assert!(close(ar(&u), 2.0 * PI * r * r - lens, 1e-3));
}

#[test]
fn circle_intersect_rect_keeps_curve() {
    let i = boolean(&circle(0.0, 0.0, 10.0), NZ, &rect(0.0, -20.0, 20.0, 40.0), NZ, BoolOp::Intersect);
    assert!(has_curves(&i));
    assert!(close(ar(&i), PI * 50.0, 1e-3));
    assert!(anchor_count(&i) <= 5, "anchors: {}", anchor_count(&i));
}

#[test]
fn hole_from_difference() {
    let d = boolean(&rect(0.0, 0.0, 10.0, 10.0), NZ, &rect(3.0, 3.0, 4.0, 4.0), NZ, BoolOp::Difference);
    assert_eq!(subpath_count(&d), 2);
    assert!(close(ar(&d), 84.0, 1e-9));
    // Consistently oriented: works with both fill rules.
    assert!(close(area(&d, FillRule::EvenOdd), 84.0, 1e-9));
}

#[test]
fn fill_rules_differ_for_nested_same_direction() {
    let mut p = rect(0.0, 0.0, 10.0, 10.0);
    p.extend(rect(3.0, 3.0, 4.0, 4.0).iter());
    assert!(close(area(&p, FillRule::NonZero), 100.0, 1e-9));
    assert!(close(area(&p, FillRule::EvenOdd), 84.0, 1e-9));
    let u = boolean(&p, FillRule::EvenOdd, &rect(20.0, 0.0, 1.0, 1.0), NZ, BoolOp::Union);
    assert!(close(ar(&u), 85.0, 1e-9));
}

#[test]
fn open_paths_are_closed_for_fill() {
    let tri = poly(&[(0.0, 0.0), (10.0, 0.0), (0.0, 10.0)], false);
    assert!(close(ar(&tri), 50.0, 1e-9));
}

#[test]
fn non_finite_input_errors() {
    let bad = poly(&[(f64::NAN, 0.0), (1.0, 0.0), (0.0, 1.0)], true);
    let r = try_boolean(&bad, NZ, &rect(0.0, 0.0, 1.0, 1.0), NZ, BoolOp::Union, DEFAULT_PRECISION);
    assert_eq!(r, Err(PathOpsError::NonFinite));
}

#[test]
fn normalize_figure_eight() {
    let p = poly(&[(0.0, 0.0), (10.0, 10.0), (10.0, 0.0), (0.0, 10.0)], true);
    let n = normalize(&p, NZ);
    assert_eq!(subpath_count(&n), 2);
    assert!(close(ar(&n), 50.0, 1e-9));
}

#[test]
fn boolean_n_and_unite_all_agree_with_the_binary_operations() {
    let (a, b, c) = (rect(0.0, 0.0, 10.0, 10.0), rect(5.0, 5.0, 10.0, 10.0), rect(8.0, 0.0, 10.0, 10.0));
    let inputs = [(&a, NZ), (&b, NZ), (&c, NZ)];
    let all = unite_all(&inputs);
    // a and c make an 18 × 10 bar (180), and b adds the 10 × 5 above it (50).
    assert!(close(ar(&all), 230.0, 1e-9), "{}", ar(&all));
    // Covered by exactly two of the three.
    let n = inputs.len();
    let two = boolean_n(&inputs, |c| c.len() == 2);
    let three = boolean_n(&inputs, |c| c.len() == n);
    let one = boolean_n(&inputs, |c| c.len() == 1);
    assert!(close(ar(&one) + ar(&two) + ar(&three), ar(&all), 1e-6));
    assert!(close(try_unite_all(&inputs).map(|p| ar(&p)).unwrap(), ar(&all), 1e-9));
    assert!(close(try_boolean_n(&inputs, |c| c.contains(&0)).map(|p| ar(&p)).unwrap(), 100.0, 1e-9));
    assert!(try_unite_all(&[(&poly(&[(f64::NAN, 0.0), (1.0, 0.0), (0.0, 1.0)], true), NZ)]).is_err());
}

fn stack() -> Vec<Shape> {
    vec![shape(rect(0.0, 0.0, 10.0, 10.0), 1), shape(rect(5.0, 5.0, 10.0, 10.0), 2)]
}

#[test]
fn pf_unite_takes_front_key() {
    let r = pathfinder(PathfinderOp::Unite, &stack());
    assert_eq!(r.len(), 1);
    assert_eq!(r[0].key, 2);
    assert!(close(ar(&r[0].path), 175.0, 1e-9));
}

#[test]
fn pf_minus_front_and_back() {
    let r = pathfinder(PathfinderOp::MinusFront, &stack());
    assert_eq!(r[0].key, 1);
    assert!(close(ar(&r[0].path), 75.0, 1e-9));
    let r = pathfinder(PathfinderOp::MinusBack, &stack());
    assert_eq!(r[0].key, 2);
    assert!(close(ar(&r[0].path), 75.0, 1e-9));
}

#[test]
fn pf_intersect_and_exclude_three() {
    let s = vec![shape(rect(0.0, 0.0, 10.0, 10.0), 1), shape(rect(5.0, 0.0, 10.0, 10.0), 2), shape(rect(8.0, 0.0, 10.0, 10.0), 3)];
    let i = pathfinder(PathfinderOp::Intersect, &s);
    assert!(close(ar(&i[0].path), 20.0, 1e-9));
    // Odd coverage: [0,5)=1, [5,8)=2, [8,10)=3, [10,15)=2, [15,18)=1 → 5+2+3 = 10 wide.
    let x = pathfinder(PathfinderOp::Exclude, &s);
    assert!(close(ar(&x[0].path), 100.0, 1e-9));
}

#[test]
fn pf_divide_faces() {
    let r = pathfinder(PathfinderOp::Divide, &stack());
    assert_eq!(r.len(), 3);
    let total: f64 = r.iter().map(|s| ar(&s.path)).sum();
    assert!(close(total, 175.0, 1e-9));
    let overlap = r.iter().find(|s| close(ar(&s.path), 25.0, 1e-9)).unwrap();
    assert_eq!(overlap.key, 2);
}

#[test]
fn pf_trim_removes_hidden() {
    let r = pathfinder(PathfinderOp::Trim, &stack());
    assert_eq!(r.len(), 2);
    assert!(close(ar(&r[0].path), 75.0, 1e-9));
    assert!(close(ar(&r[1].path), 100.0, 1e-9));
}

#[test]
fn pf_merge_same_key() {
    let mut s = stack();
    s[1].key = 1;
    let r = pathfinder(PathfinderOp::Merge, &s);
    assert_eq!(r.len(), 1);
    assert!(close(ar(&r[0].path), 175.0, 1e-9));
    let r = pathfinder(PathfinderOp::Merge, &stack());
    assert_eq!(r.len(), 2);
}

#[test]
fn pf_crop() {
    let s = vec![shape(rect(0.0, 0.0, 10.0, 10.0), 1), shape(rect(10.0, 0.0, 10.0, 10.0), 2), shape(rect(5.0, 0.0, 10.0, 5.0), 3)];
    let r = pathfinder(PathfinderOp::Crop, &s);
    assert_eq!(r.len(), 2);
    assert!(r.iter().all(|x| close(ar(&x.path), 25.0, 1e-9)));
    assert_eq!(r[0].key, 1);
}

#[test]
fn pf_outline_edges() {
    let r = pathfinder(PathfinderOp::Outline, &stack());
    assert_eq!(r.len(), 4, "{r:?}");
    assert!(r.iter().all(|s| !any_closed(&s.path)));
    let len: f64 = r.iter().map(|s| length(&s.path)).sum();
    assert!(close(len, 80.0, 1e-9));
}

/// VectorCraft issue 3870: a circle whose anchors sit on the square's edge it crosses gives the
/// four edges of the arrangement, meeting at those anchors, and no sliver between an anchor and the
/// crossing computed a hair from it.
#[test]
fn pf_outline_has_no_sliver_where_an_anchor_is_the_crossing() {
    let s = vec![shape(rect(0.0, 0.0, 100.0, 100.0), 1), shape(circle(100.0, 100.0, 50.0), 2)];
    let r = pathfinder(PathfinderOp::Outline, &s);
    assert_eq!(r.len(), 4, "{r:?}");
    assert!(r.iter().all(|s| length(&s.path) > 1.0), "no sliver: {r:?}");
}

#[test]
fn pf_outline_dedups_shared_edges() {
    // Two squares sharing an edge exactly.
    let s = vec![shape(rect(0.0, 0.0, 10.0, 10.0), 1), shape(rect(10.0, 0.0, 10.0, 10.0), 2)];
    let r = pathfinder(PathfinderOp::Outline, &s);
    let len: f64 = r.iter().map(|s| length(&s.path)).sum();
    assert!(close(len, 70.0, 1e-9), "len {len}");
}

#[test]
fn regions_and_region_at() {
    let s = stack();
    let rs = regions(&s);
    assert_eq!(rs.len(), 3);
    let hit = region_at(&s, Point::new(7.0, 7.0)).unwrap();
    assert_eq!(hit.sources, vec![0, 1]);
    assert!(close(ar(&hit.path), 25.0, 1e-9));
    assert!(region_at(&s, Point::new(100.0, 100.0)).is_none());
    let refs: Vec<&Region> = rs.iter().collect();
    assert!(close(ar(&merge_regions(&refs)), 175.0, 1e-9));
}

#[test]
fn regions_split_an_island_inside_a_hole_from_the_frame_around_it() {
    // A frame (0..200 minus 50..150) with an island (80..120) in its hole, as Minus Front leaves
    // a rectangle behind a ring (VectorCraft issue 3132): the frame and the island are separate
    // faces.
    let mut p = rect(0.0, 0.0, 200.0, 200.0);
    p.extend(rect(50.0, 50.0, 100.0, 100.0).iter());
    p.extend(rect(80.0, 80.0, 40.0, 40.0).iter());
    let s = vec![Shape::new(p, FillRule::EvenOdd, 1)];
    let rs = regions(&s);
    let mut filled: Vec<(f64, Rect)> = rs.iter().filter(|r| r.sources == vec![0]).map(|r| (ar(&r.path).abs(), r.path.bounding_box())).collect();
    filled.sort_by(|a, b| b.0.total_cmp(&a.0));
    assert_eq!(filled.len(), 2, "{filled:?}");
    assert!(close(filled[0].0, 30000.0, 1e-9), "{filled:?}");
    assert_eq!(filled[0].1, Rect::new(0.0, 0.0, 200.0, 200.0));
    assert!(close(filled[1].0, 1600.0, 1e-9), "{filled:?}");
    assert_eq!(filled[1].1, Rect::new(80.0, 80.0, 120.0, 120.0));
    let frame = region_at(&s, Point::new(10.0, 10.0)).unwrap();
    assert!(!frame.contains(Point::new(100.0, 100.0)));
}

#[test]
fn offset_circle_grows() {
    let o = offset_path(&circle(0.0, 0.0, 10.0), 3.0, Join::Round, 4.0);
    assert!(close(ar(&o), PI * 169.0, 2e-3), "{}", ar(&o));
    assert!(anchor_count(&o) <= 16, "anchors {}", anchor_count(&o));
}

#[test]
fn offset_circle_insets() {
    let o = offset_path(&circle(0.0, 0.0, 10.0), -4.0, Join::Round, 4.0);
    assert!(close(ar(&o), PI * 36.0, 2e-3), "{}", ar(&o));
}

#[test]
fn offset_rect_joins() {
    let r = rect(0.0, 0.0, 20.0, 10.0);
    assert!(close(ar(&offset_path(&r, 2.0, Join::Miter, 4.0)), 24.0 * 14.0, 1e-6));
    assert_eq!(anchor_count(&offset_path(&r, 2.0, Join::Miter, 4.0)), 4);
    assert!(close(ar(&offset_path(&r, 2.0, Join::Bevel, 4.0)), 24.0 * 14.0 - 4.0 * 2.0, 1e-6));
    assert!(close(ar(&offset_path(&r, 2.0, Join::Round, 4.0)), 200.0 + 2.0 * 2.0 * 30.0 + PI * 4.0, 1e-3));
    assert!(close(ar(&offset_path(&r, -2.0, Join::Miter, 4.0)), 16.0 * 6.0, 1e-6));
    assert!(offset_path(&r, -6.0, Join::Miter, 4.0).elements().is_empty());
}

#[test]
fn offset_of_an_open_path_whose_ends_meet_is_the_closed_shape_offset() {
    // VectorCraft issue 3773: an open path whose last anchor sits on its first offsets like the
    // closed shape, not as a ring around its outline.
    let r = poly(&[(0.0, 0.0), (20.0, 0.0), (20.0, 10.0), (0.0, 10.0), (0.0, 0.0)], false);
    let grown = offset_path(&r, 2.0, Join::Miter, 4.0);
    assert!(close(ar(&grown), 24.0 * 14.0, 1e-6), "{}", ar(&grown));
    assert_eq!(subpath_count(&grown), 1);
    assert_eq!(anchor_count(&grown), 4);
    assert!(close(ar(&offset_path(&r, -2.0, Join::Miter, 4.0)), 16.0 * 6.0, 1e-6));
    // A curved outline: the last segment arrives at the first anchor by its in-handle.
    let mut c = circle(0.0, 0.0, 10.0);
    let els = c.elements().to_vec();
    // The circle without its ClosePath is an open path that ends where it starts.
    c = BezPath::from_vec(els.into_iter().filter(|e| !matches!(e, kurbo::PathEl::ClosePath)).collect());
    let o = offset_path(&c, 3.0, Join::Round, 4.0);
    assert!(close(ar(&o), PI * 169.0, 2e-3), "{}", ar(&o));
    assert_eq!(subpath_count(&o), 1);
}

#[test]
fn outline_stroke_of_line() {
    let l = poly(&[(0.0, 0.0), (30.0, 40.0)], false);
    let o = outline_stroke(&l, 4.0, Cap::Butt, Join::Miter, 4.0);
    assert!(close(ar(&o), 200.0, 1e-6));
    let o = outline_stroke(&l, 4.0, Cap::Square, Join::Miter, 4.0);
    assert!(close(ar(&o), 216.0, 1e-6));
}

#[test]
fn outline_stroke_of_straight_line_with_round_caps_is_compact() {
    let l = poly(&[(0.0, 50.0), (100.0, 50.0)], false);
    let o = outline_stroke(&l, 10.0, Cap::Round, Join::Round, 4.0);
    assert!(close(ar(&o), 100.0 * 10.0 + PI * 5.0 * 5.0, 1e-3), "{}", ar(&o));
    let anchors = anchor_count(&o);
    assert!(anchors <= 8, "round-cap expand should stay compact, got {anchors} anchors");
    let dense: Vec<Point> = (0..=20).map(|i| Point::new(f64::from(i) * 5.0, 50.0)).collect();
    let dense_o = outline_stroke(&polyline(&dense, false), 10.0, Cap::Round, Join::Round, 4.0);
    assert_eq!(anchor_count(&dense_o), anchors, "colinear spine points should not add outline anchors");
}

#[test]
fn outline_stroke_of_square_is_ring() {
    let o = outline_stroke(&rect(0.0, 0.0, 10.0, 10.0), 2.0, Cap::Butt, Join::Miter, 10.0);
    assert_eq!(subpath_count(&o), 2);
    assert!(close(ar(&o), 144.0 - 64.0, 1e-6));
}

#[test]
fn outline_stroke_self_overlap_is_cleaned() {
    // A zig-zag whose stroke overlaps itself at the joins.
    let z = poly(&[(0.0, 0.0), (10.0, 0.0), (0.0, 1.0), (10.0, 2.0)], false);
    let o = outline_stroke(&z, 3.0, Cap::Round, Join::Round, 4.0);
    assert!(!o.elements().is_empty());
    // Normalised output: its signed area equals its fill area.
    let signed: f64 = o.area().abs();
    assert!(close(signed, ar(&o), 1e-6));
}

fn dense_circle(r: f64, n: usize) -> BezPath {
    let pts: Vec<Point> =
        (0..n).map(|i| Point::new(r * (i as f64 / n as f64 * 2.0 * PI).cos(), r * (i as f64 / n as f64 * 2.0 * PI).sin())).collect();
    polyline(&pts, true)
}

fn max_deviation(orig: &BezPath, simp: &BezPath) -> f64 {
    let mut worst: f64 = 0.0;
    for seg in orig.segments() {
        for i in 0..=8 {
            let p = seg.eval(f64::from(i) / 8.0);
            worst = worst.max(distance_to(simp, p));
        }
    }
    worst
}

/// The deviation both ways: no point of the original far from the result, and none of the result far from the original.
fn deviation_both_ways(orig: &BezPath, simp: &BezPath) -> f64 {
    max_deviation(orig, simp).max(max_deviation(simp, orig))
}

#[test]
fn simplify_dense_circle() {
    let c = dense_circle(50.0, 200);
    let s = simplify(&c, 0.5);
    assert!(anchor_count(&s) <= 12, "anchors {}", anchor_count(&s));
    assert!(deviation_both_ways(&c, &s) <= 0.5 + 1e-6, "{}", deviation_both_ways(&c, &s));
    assert!(all_closed(&s));
}

#[test]
fn simplify_keeps_square_corners() {
    let mut pts = Vec::new();
    for (a, b) in [((0.0, 0.0), (10.0, 0.0)), ((10.0, 0.0), (10.0, 10.0)), ((10.0, 10.0), (0.0, 10.0)), ((0.0, 10.0), (0.0, 0.0))] {
        for i in 0..5 {
            let t = f64::from(i) / 5.0;
            pts.push(Point::new(a.0 + (b.0 - a.0) * t, a.1 + (b.1 - a.1) * t));
        }
    }
    let sq = polyline(&pts, true);
    let s = simplify(&sq, 0.1);
    assert_eq!(anchor_count(&s), 4);
    assert!(!has_curves(&s));
}

#[test]
fn simplify_straight_lines_option() {
    let c = dense_circle(50.0, 200);
    let s = simplify_with(&c, &SimplifyOptions::default().tolerance(1.0).straight_lines(true));
    assert!(!has_curves(&s));
    assert!(anchor_count(&s) < 40);
    assert!(deviation_both_ways(&c, &s) <= 1.0 + 1e-6, "{}", deviation_both_ways(&c, &s));
}

#[test]
fn remove_redundant_collinear_and_curve() {
    let p = poly(&[(0.0, 0.0), (5.0, 0.0), (10.0, 0.0), (10.0, 10.0), (0.0, 10.0)], true);
    assert_eq!(anchor_count(&remove_redundant_points(&p, 1e-6)), 4);
    // A circle with an anchor at the middle of each of its segments.
    let c = circle(0.0, 0.0, 10.0);
    let mut halved = BezPath::new();
    for seg in c.segments() {
        if halved.elements().is_empty() {
            halved.move_to(seg.start());
        }
        match seg {
            kurbo::PathSeg::Cubic(cu) => {
                let (l, r) = (cu.subsegment(0.0..0.5), cu.subsegment(0.5..1.0));
                halved.curve_to(l.p1, l.p2, l.p3);
                halved.curve_to(r.p1, r.p2, r.p3);
            }
            other => halved.push(other.as_path_el()),
        }
    }
    halved.close_path();
    let n0 = anchor_count(&halved);
    let r = remove_redundant_points(&halved, 1e-3);
    assert!(anchor_count(&r) < n0, "{} vs {}", anchor_count(&r), n0);
    assert!(close(ar(&r), ar(&halved), 1e-4));
}

#[test]
fn join_two_open_paths() {
    let a = poly(&[(0.0, 0.0), (10.0, 0.0)], false);
    let b = poly(&[(20.0, 5.0), (10.0, 0.0)], false);
    let j = join(&[a.clone(), b], 1e-6);
    assert_eq!(subpath_count(&j), 1);
    assert_eq!(anchor_count(&j), 3);
    assert!(!any_closed(&j));
    let c = poly(&[(12.0, 0.0), (20.0, 0.0)], false);
    let j = join(&[a, c], 1e-6);
    assert_eq!(anchor_count(&j), 4);
}

#[test]
fn join_single_path_closes() {
    let a = poly(&[(0.0, 0.0), (10.0, 0.0), (5.0, 5.0)], false);
    let j = join(&[a], 1e-6);
    assert!(all_closed(&j));
    assert_eq!(anchor_count(&j), 3);
    let b = poly(&[(0.0, 0.0), (10.0, 0.0), (5.0, 5.0), (0.0, 0.0)], false);
    assert_eq!(anchor_count(&join(&[b], 1e-6)), 3);
}

#[test]
fn join_keeps_closed_subpaths_and_orders_the_rest() {
    let closed = rect(50.0, 50.0, 10.0, 10.0);
    let (a, b, c) = (poly(&[(0.0, 0.0), (10.0, 0.0)], false), poly(&[(10.0, 0.0), (10.0, 10.0)], false), poly(&[(10.0, 10.0), (0.0, 10.0)], false));
    let j = join(&[a, closed, b, c], 1e-6);
    // The closed square first, then one open path of four points.
    let parts = split(&j);
    assert_eq!(parts.len(), 2);
    assert!(all_closed(&parts[0]) && !any_closed(&parts[1]));
    assert_eq!(anchor_count(&parts[1]), 4);
    assert_eq!(ends(&parts[1]).0.distance(Point::new(0.0, 0.0)).min(ends(&parts[1]).1.distance(Point::new(0.0, 0.0))), 0.0);
}

/// Joining a gap wider than the tolerance puts a straight segment across it; closer ends become one point.
#[test]
fn join_bridges_a_gap_with_a_line_and_merges_close_ends() {
    let (a, b) = (poly(&[(0.0, 0.0), (10.0, 0.0)], false), poly(&[(10.5, 0.0), (20.0, 0.0)], false));
    let far = join(&[a.clone(), b.clone()], 0.1);
    assert_eq!(anchor_count(&far), 4, "a bridge across the gap");
    let near = join(&[a, b], 1.0);
    assert_eq!(anchor_count(&near), 3, "the two ends are one point");
    assert_eq!(subpath_count(&near), 1);
}

#[test]
fn unite_many_circles() {
    let mut seed = 0x1234_5678_u64;
    let mut rnd = || {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        (seed % 1_000_000) as f64 / 1_000_000.0
    };
    let circles: Vec<Shape> = (0..200).map(|i| shape(circle(rnd() * 500.0, rnd() * 500.0, 5.0 + rnd() * 30.0), i)).collect();
    let r = pathfinder(PathfinderOp::Unite, &circles);
    assert_eq!(r.len(), 1);
    let total: f64 = circles.iter().map(|s| ar(&s.path)).sum();
    let united = ar(&r[0].path);
    let biggest = circles.iter().map(|s| ar(&s.path)).fold(0.0, f64::max);
    assert!(united >= biggest && united <= total, "{biggest} <= {united} <= {total}");
}

/// An open path that ends where it starts is outlined as the closed shape it traces: its outline
/// has no piece of no length at the point where it closes (VectorCraft's Outline made one there).
#[test]
fn pf_outline_of_an_open_ring_has_no_piece_of_no_length() {
    let ring = poly(&[(0.0, 0.0), (40.0, 0.0), (40.0, 40.0), (0.0, 0.0)], false);
    let s = vec![shape(ring, 1), shape(rect(20.0, -10.0, 10.0, 60.0), 2)];
    let r = pathfinder(PathfinderOp::Outline, &s);
    assert!(!r.is_empty());
    assert!(r.iter().all(|s| length(&s.path) > 1.0), "{r:?}");
}

/// A path whose ends are within the tolerance is closed by moving its last segment onto its start:
/// the last segment of a polyline stays straight (VectorCraft left it a curve, bent by the old
/// end's position).
#[test]
fn join_closes_a_path_whose_ends_are_within_the_tolerance_with_straight_segments() {
    let open = poly(&[(0.0, 0.0), (10.0, 0.0), (10.0, 10.0), (0.1, 0.2)], false);
    let j = join(&[open], 1.0);
    assert!(all_closed(&j));
    assert!(!has_curves(&j), "{j:?}");
    assert_eq!(anchor_count(&j), 3);
    // The tolerance decides: farther apart, the path closes with a segment of its own.
    let j = join(&[poly(&[(0.0, 0.0), (10.0, 0.0), (10.0, 10.0), (0.1, 0.2)], false)], 0.01);
    assert!(all_closed(&j));
    assert_eq!(anchor_count(&j), 4);
}

/// A quadratic is swept as the cubic it raises to, so the operations see lines and cubics only.
#[test]
fn quadratic_segments_are_swept_as_their_cubics() {
    let quad = BezPath::from_svg("M0,0 Q50,100 100,0 Q50,-30 0,0 Z").unwrap();
    let mut raised = BezPath::new();
    raised.move_to((0.0, 0.0));
    for (from, c, to) in [((0.0, 0.0), (50.0, 100.0), (100.0, 0.0)), ((100.0, 0.0), (50.0, -30.0), (0.0, 0.0))] {
        let q = kurbo::QuadBez::new(from, c, to).raise();
        raised.curve_to(q.p1, q.p2, q.p3);
    }
    raised.close_path();
    let other = rect(20.0, -20.0, 50.0, 60.0);
    for op in [BoolOp::Union, BoolOp::Intersect, BoolOp::Difference, BoolOp::Xor] {
        assert_eq!(boolean(&quad, NZ, &other, NZ, op), boolean(&raised, NZ, &other, NZ, op), "{op:?}");
    }
    assert_eq!(offset_path(&quad, 3.0, Join::Round, 4.0), offset_path(&raised, 3.0, Join::Round, 4.0));
    assert!(has_curves(&boolean(&quad, NZ, &other, NZ, BoolOp::Union)));
}
