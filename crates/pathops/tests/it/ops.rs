use std::f64::consts::PI;

use craft_geom::{Anchor, AnchorKind, FillRule, PathData, SubPath};
use craft_pathops::*;
use kurbo::{ParamCurve, Point, Rect, Shape as _};

const NZ: FillRule = FillRule::NonZero;

fn rect(x: f64, y: f64, w: f64, h: f64) -> PathData {
    PathData::from_bezpath(&Rect::new(x, y, x + w, y + h).to_path(1e-6))
}

fn circle(cx: f64, cy: f64, r: f64) -> PathData {
    PathData::from_bezpath(&kurbo::Circle::new((cx, cy), r).to_path(1e-4))
}

fn ar(p: &PathData) -> f64 {
    area(p, NZ)
}

fn close(a: f64, b: f64, rel: f64) -> bool {
    (a - b).abs() <= rel * a.abs().max(b.abs()).max(1.0)
}

fn shape(p: PathData, key: u64) -> Shape {
    Shape::new(p, NZ, key)
}

fn has_curves(p: &PathData) -> bool {
    p.anchors().any(|(_, _, a)| a.has_in() || a.has_out())
}

#[test]
fn union_of_rects() {
    let u = boolean(&rect(0.0, 0.0, 10.0, 10.0), NZ, &rect(5.0, 5.0, 10.0, 10.0), NZ, BoolOp::Union);
    assert!(close(ar(&u), 175.0, 1e-9));
    assert_eq!(u.subpaths.len(), 1);
    assert_eq!(u.anchor_count(), 8);
}

#[test]
fn intersect_of_rects() {
    let i = boolean(&rect(0.0, 0.0, 10.0, 10.0), NZ, &rect(5.0, 5.0, 10.0, 10.0), NZ, BoolOp::Intersect);
    assert!(close(ar(&i), 25.0, 1e-9));
    assert_eq!(i.anchor_count(), 4);
}

#[test]
fn difference_of_rects() {
    let d = boolean(&rect(0.0, 0.0, 10.0, 10.0), NZ, &rect(5.0, 5.0, 10.0, 10.0), NZ, BoolOp::Difference);
    assert!(close(ar(&d), 75.0, 1e-9));
    assert_eq!(d.anchor_count(), 6);
}

#[test]
fn xor_of_rects() {
    let x = boolean(&rect(0.0, 0.0, 10.0, 10.0), NZ, &rect(5.0, 5.0, 10.0, 10.0), NZ, BoolOp::Xor);
    assert!(close(ar(&x), 150.0, 1e-9));
}

#[test]
fn disjoint_union_keeps_both() {
    let u = boolean(&rect(0.0, 0.0, 1.0, 1.0), NZ, &rect(5.0, 5.0, 1.0, 1.0), NZ, BoolOp::Union);
    assert_eq!(u.subpaths.len(), 2);
    assert!(close(ar(&u), 2.0, 1e-9));
}

#[test]
fn a_minus_a_is_empty() {
    let c = circle(3.0, 4.0, 7.0);
    assert!(boolean(&c, NZ, &c, NZ, BoolOp::Difference).is_empty());
}

#[test]
fn union_idempotent_rect() {
    let r = rect(1.0, 2.0, 3.0, 4.0);
    let u = boolean(&r, NZ, &r, NZ, BoolOp::Union);
    assert_eq!(u.anchor_count(), 4);
    assert!(close(ar(&u), 12.0, 1e-9));
}

#[test]
fn circle_union_is_curve_preserving_and_compact() {
    let u = boolean(&circle(0.0, 0.0, 10.0), NZ, &circle(10.0, 0.0, 10.0), NZ, BoolOp::Union);
    assert!(has_curves(&u));
    assert!(u.anchor_count() <= 12, "anchors: {}", u.anchor_count());
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
    assert!(i.anchor_count() <= 5, "anchors: {}", i.anchor_count());
}

#[test]
fn hole_from_difference() {
    let d = boolean(&rect(0.0, 0.0, 10.0, 10.0), NZ, &rect(3.0, 3.0, 4.0, 4.0), NZ, BoolOp::Difference);
    assert_eq!(d.subpaths.len(), 2);
    assert!(close(ar(&d), 84.0, 1e-9));
    // Consistently oriented: works with both fill rules.
    assert!(close(area(&d, FillRule::EvenOdd), 84.0, 1e-9));
}

#[test]
fn fill_rules_differ_for_nested_same_direction() {
    let mut p = rect(0.0, 0.0, 10.0, 10.0);
    p.subpaths.extend(rect(3.0, 3.0, 4.0, 4.0).subpaths);
    assert!(close(area(&p, FillRule::NonZero), 100.0, 1e-9));
    assert!(close(area(&p, FillRule::EvenOdd), 84.0, 1e-9));
    let u = boolean(&p, FillRule::EvenOdd, &rect(20.0, 0.0, 1.0, 1.0), NZ, BoolOp::Union);
    assert!(close(ar(&u), 85.0, 1e-9));
}

#[test]
fn open_paths_are_closed_for_fill() {
    let tri = PathData::single(SubPath::polyline(&[Point::new(0.0, 0.0), Point::new(10.0, 0.0), Point::new(0.0, 10.0)], false));
    assert!(close(ar(&tri), 50.0, 1e-9));
}

#[test]
fn non_finite_input_errors() {
    let bad = PathData::single(SubPath::polyline(&[Point::new(f64::NAN, 0.0), Point::new(1.0, 0.0), Point::new(0.0, 1.0)], true));
    let r = try_boolean(&bad, NZ, &rect(0.0, 0.0, 1.0, 1.0), NZ, BoolOp::Union, DEFAULT_PRECISION);
    assert_eq!(r, Err(PathOpsError::NonFinite));
}

#[test]
fn normalize_figure_eight() {
    let p = PathData::single(SubPath::polyline(&[Point::new(0.0, 0.0), Point::new(10.0, 10.0), Point::new(10.0, 0.0), Point::new(0.0, 10.0)], true));
    let n = normalize(&p, NZ);
    assert_eq!(n.subpaths.len(), 2);
    assert!(close(ar(&n), 50.0, 1e-9));
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
    assert!(r.iter().all(|s| s.path.subpaths.iter().all(|sp| !sp.closed)));
    let len: f64 = r.iter().map(|s| s.path.length()).sum();
    assert!(close(len, 80.0, 1e-9));
}

/// #3870: a circle whose anchors sit on the square's edge it crosses gives the four edges of the
/// arrangement, meeting at those anchors, and no sliver between an anchor and the crossing
/// computed a hair from it.
#[test]
fn pf_outline_has_no_sliver_where_an_anchor_is_the_crossing() {
    let s = vec![shape(rect(0.0, 0.0, 100.0, 100.0), 1), shape(circle(100.0, 100.0, 50.0), 2)];
    let r = pathfinder(PathfinderOp::Outline, &s);
    assert_eq!(r.len(), 4, "{r:?}");
    assert!(r.iter().all(|s| s.path.length() > 1.0), "no sliver: {r:?}");
}

#[test]
fn pf_outline_dedups_shared_edges() {
    // Two squares sharing an edge exactly.
    let s = vec![shape(rect(0.0, 0.0, 10.0, 10.0), 1), shape(rect(10.0, 0.0, 10.0, 10.0), 2)];
    let r = pathfinder(PathfinderOp::Outline, &s);
    let len: f64 = r.iter().map(|s| s.path.length()).sum();
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
    // a rectangle behind a ring (#3132): the frame and the island are separate faces.
    let mut p = rect(0.0, 0.0, 200.0, 200.0);
    p.subpaths.extend(rect(50.0, 50.0, 100.0, 100.0).subpaths);
    p.subpaths.extend(rect(80.0, 80.0, 40.0, 40.0).subpaths);
    let s = vec![Shape::new(p, FillRule::EvenOdd, 1)];
    let rs = regions(&s);
    let mut filled: Vec<(f64, Rect)> = rs.iter().filter(|r| r.sources == vec![0]).map(|r| (ar(&r.path).abs(), r.path.bounds().unwrap())).collect();
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
    assert!(o.anchor_count() <= 16, "anchors {}", o.anchor_count());
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
    assert_eq!(offset_path(&r, 2.0, Join::Miter, 4.0).anchor_count(), 4);
    assert!(close(ar(&offset_path(&r, 2.0, Join::Bevel, 4.0)), 24.0 * 14.0 - 4.0 * 2.0, 1e-6));
    assert!(close(ar(&offset_path(&r, 2.0, Join::Round, 4.0)), 200.0 + 2.0 * 2.0 * 30.0 + PI * 4.0, 1e-3));
    assert!(close(ar(&offset_path(&r, -2.0, Join::Miter, 4.0)), 16.0 * 6.0, 1e-6));
    assert!(offset_path(&r, -6.0, Join::Miter, 4.0).is_empty());
}

#[test]
fn offset_of_an_open_path_whose_ends_meet_is_the_closed_shape_offset() {
    // Issue #3773: an open path whose last anchor sits on its first offsets like the closed shape,
    // not as a ring around its outline.
    let corners = [Point::new(0.0, 0.0), Point::new(20.0, 0.0), Point::new(20.0, 10.0), Point::new(0.0, 10.0), Point::new(0.0, 0.0)];
    let r = PathData::single(SubPath::polyline(&corners, false));
    let grown = offset_path(&r, 2.0, Join::Miter, 4.0);
    assert!(close(ar(&grown), 24.0 * 14.0, 1e-6), "{}", ar(&grown));
    assert_eq!(grown.subpaths.len(), 1);
    assert_eq!(grown.anchor_count(), 4);
    assert!(close(ar(&offset_path(&r, -2.0, Join::Miter, 4.0)), 16.0 * 6.0, 1e-6));
    // A curved outline: the last anchor's in-handle shapes the closing curve.
    let mut sp = circle(0.0, 0.0, 10.0).subpaths.remove(0);
    let first = sp.anchors[0];
    sp.anchors.push(craft_geom::Anchor::with_handles(first.p, first.h_in, first.p));
    sp.anchors[0].h_in = first.p;
    sp.closed = false;
    let o = offset_path(&PathData::single(sp), 3.0, Join::Round, 4.0);
    assert!(close(ar(&o), PI * 169.0, 2e-3), "{}", ar(&o));
    assert_eq!(o.subpaths.len(), 1);
}

#[test]
fn a_negative_offset_outlines_an_open_path_as_a_positive_one_does() {
    // Issue #6427 (A4): an inset of an open path was empty. Open subpaths are outlined with a
    // stroke of width 2|delta| whichever way the offset goes.
    let line = PathData::single(SubPath::polyline(&[Point::new(0.0, 0.0), Point::new(100.0, 0.0)], false));
    for delta in [2.0, -2.0] {
        let o = offset_path(&line, delta, Join::Miter, 4.0);
        assert!(close(ar(&o), 400.0, 1e-6), "delta {delta}: {}", ar(&o));
        assert_eq!(o.subpaths.len(), 1, "delta {delta}");
    }
    // With a closed shape beside it, the shape is inset and the line still outlined.
    let mut both = rect(0.0, 0.0, 20.0, 10.0);
    both.subpaths.push(SubPath::polyline(&[Point::new(100.0, 0.0), Point::new(150.0, 0.0)], false));
    let o = offset_path(&both, -2.0, Join::Miter, 4.0);
    assert!(close(ar(&o), 16.0 * 6.0 + 50.0 * 4.0, 1e-6), "{}", ar(&o));
    assert_eq!(o.subpaths.len(), 2);
}

#[test]
fn outline_stroke_of_line() {
    let l = PathData::single(SubPath::polyline(&[Point::new(0.0, 0.0), Point::new(30.0, 40.0)], false));
    let o = outline_stroke(&l, 4.0, Cap::Butt, Join::Miter, 4.0);
    assert!(close(ar(&o), 200.0, 1e-6));
    let o = outline_stroke(&l, 4.0, Cap::Square, Join::Miter, 4.0);
    assert!(close(ar(&o), 216.0, 1e-6));
}

#[test]
fn outline_stroke_of_straight_line_with_round_caps_is_compact() {
    let l = PathData::single(SubPath::polyline(&[Point::new(0.0, 50.0), Point::new(100.0, 50.0)], false));
    let o = outline_stroke(&l, 10.0, Cap::Round, Join::Round, 4.0);
    assert!(close(ar(&o), 100.0 * 10.0 + PI * 5.0 * 5.0, 1e-3), "{}", ar(&o));
    let anchors = o.subpaths.iter().map(|s| s.anchors.len()).sum::<usize>();
    assert!(anchors <= 8, "round-cap expand should stay compact, got {anchors} anchors");
    let dense: Vec<Point> = (0..=20).map(|i| Point::new(i as f64 * 5.0, 50.0)).collect();
    let dense_path = PathData::single(SubPath::polyline(&dense, false));
    let dense_o = outline_stroke(&dense_path, 10.0, Cap::Round, Join::Round, 4.0);
    let dense_anchors = dense_o.subpaths.iter().map(|s| s.anchors.len()).sum::<usize>();
    assert_eq!(dense_anchors, anchors, "colinear spine points should not add outline anchors");
}

#[test]
fn outline_stroke_of_square_is_ring() {
    let o = outline_stroke(&rect(0.0, 0.0, 10.0, 10.0), 2.0, Cap::Butt, Join::Miter, 10.0);
    assert_eq!(o.subpaths.len(), 2);
    assert!(close(ar(&o), 144.0 - 64.0, 1e-6));
}

#[test]
fn outline_stroke_self_overlap_is_cleaned() {
    // A zig-zag whose stroke overlaps itself at the joins.
    let z = PathData::single(SubPath::polyline(&[Point::new(0.0, 0.0), Point::new(10.0, 0.0), Point::new(0.0, 1.0), Point::new(10.0, 2.0)], false));
    let o = outline_stroke(&z, 3.0, Cap::Round, Join::Round, 4.0);
    assert!(!o.is_empty());
    // Normalised output: its signed area equals its fill area.
    let signed: f64 = o.subpaths.iter().map(|s| s.area()).sum::<f64>().abs();
    assert!(close(signed, ar(&o), 1e-6));
}

fn dense_circle(r: f64, n: usize) -> PathData {
    let pts: Vec<Point> =
        (0..n).map(|i| Point::new(r * (i as f64 / n as f64 * 2.0 * PI).cos(), r * (i as f64 / n as f64 * 2.0 * PI).sin())).collect();
    PathData::single(SubPath::polyline(&pts, true))
}

fn max_deviation(orig: &PathData, simp: &PathData) -> f64 {
    let bp = orig.to_bezpath();
    let mut worst: f64 = 0.0;
    for seg in bp.segments() {
        for i in 0..=8 {
            let p = seg.eval(i as f64 / 8.0);
            worst = worst.max(simp.nearest(p).unwrap().4);
        }
    }
    worst
}

#[test]
fn simplify_dense_circle() {
    let c = dense_circle(50.0, 200);
    let s = simplify(&c, 0.5);
    assert!(s.anchor_count() <= 12, "anchors {}", s.anchor_count());
    assert!(max_deviation(&c, &s) <= 0.5 + 1e-6);
    assert!(s.subpaths[0].closed);
}

#[test]
fn simplify_keeps_square_corners() {
    let mut pts = Vec::new();
    for (a, b) in [((0.0, 0.0), (10.0, 0.0)), ((10.0, 0.0), (10.0, 10.0)), ((10.0, 10.0), (0.0, 10.0)), ((0.0, 10.0), (0.0, 0.0))] {
        for i in 0..5 {
            let t = i as f64 / 5.0;
            pts.push(Point::new(a.0 + (b.0 - a.0) * t, a.1 + (b.1 - a.1) * t));
        }
    }
    let sq = PathData::single(SubPath::polyline(&pts, true));
    let s = simplify(&sq, 0.1);
    assert_eq!(s.anchor_count(), 4);
    assert!(!has_curves(&s));
}

/// The largest turn, in degrees, at an anchor of `sp` that is not an end of an open subpath: the
/// angle between the direction a segment arrives along and the one the next leaves along.
fn largest_join_turn(sp: &SubPath) -> f64 {
    let dir = |v: kurbo::Vec2, w: kurbo::Vec2, chord: kurbo::Vec2| [v, w, chord].into_iter().find(|d| d.hypot() > 1e-9).unwrap_or(chord);
    let n = sp.segment_count();
    let first = usize::from(!sp.closed);
    (first..n)
        .map(|i| {
            let (prev, next) = (sp.segment((i + n - 1) % n), sp.segment(i));
            let arrive = dir(prev.p3 - prev.p2, prev.p3 - prev.p1, prev.p3 - prev.p0);
            let leave = dir(next.p1 - next.p0, next.p2 - next.p0, next.p3 - next.p0);
            arrive.cross(leave).atan2(arrive.dot(leave)).abs().to_degrees()
        })
        .fold(0.0, f64::max)
}

/// A refitted curve becomes a line only when the line keeps the smooth joins on both sides of it:
/// with no corners kept, the refit of a jagged line or of an arc running into a line turns by at most
/// 0.5° at every anchor. Points that all lie on one line still become one line.
#[test]
fn simplify_turns_a_curve_into_a_line_only_where_the_joins_stay_smooth() {
    let mut seed = 7u64;
    let mut noise = || {
        seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        ((seed >> 11) as f64 / (1u64 << 53) as f64) * 2.0 - 1.0
    };
    let jagged: Vec<Point> = (0..=60).map(|i| Point::new(100.0 + f64::from(i) * 5.0, 200.0 + 6.0 * noise())).collect();
    let mut arc_into_line: Vec<Point> =
        (0..=40).map(|i| Point::new(50.0 * (f64::from(i) / 80.0 * PI).cos(), 50.0 * (f64::from(i) / 80.0 * PI).sin())).collect();
    arc_into_line.extend((1..=20).map(|i| Point::new(-f64::from(i) * 2.0, 50.0)));
    for pts in [&jagged, &arc_into_line] {
        let path = PathData::single(SubPath::polyline(pts, false));
        for tolerance in [1.0, 2.5, 5.0] {
            let s = simplify_with(&path, &SimplifyOptions { tolerance, corner_angle_deg: 179.0, ..Default::default() });
            let turn = largest_join_turn(&s.subpaths[0]);
            assert!(turn <= 0.5 + 1e-9, "{tolerance} pt: an anchor turns {turn}°: {s:?}");
            assert!(max_deviation(&path, &s) <= tolerance + 1e-6, "{tolerance} pt: {}", max_deviation(&path, &s));
        }
    }
    let in_line: Vec<Point> = [0.0, 3.0, 10.0, 11.0, 40.0, 41.5, 90.0, 100.0].map(|x| Point::new(x, 0.5 * x)).to_vec();
    let s = simplify_with(&PathData::single(SubPath::polyline(&in_line, false)), &SimplifyOptions { tolerance: 1.0, ..Default::default() });
    assert_eq!(s.anchor_count(), 2, "{s:?}");
    assert!(!has_curves(&s), "{s:?}");
}

/// A smoothed loop with no corner is fitted across its start, and the join there also turns by at
/// most 0.5°. Each loop is a flat side that bends by 0.6° at the loop's start, closed by an arc of
/// `n` lines `height` tall, with `inner` anchors along each half of the side.
#[test]
fn simplify_keeps_the_join_at_the_start_of_a_smoothed_loop_smooth() {
    for (half, height, n, inner, tolerance, smooth) in
        [(40.0, 20.0, 8, 3, 2.5, 2.0), (80.0, 60.0, 8, 3, 5.0, 0.5), (160.0, 200.0, 24, 3, 10.0, 2.0), (160.0, 20.0, 24, 8, 10.0, 5.0)]
    {
        let rise = half * 0.3f64.to_radians().tan();
        let along = |i: i32| Point::new(half * f64::from(i) / f64::from(inner), -rise * f64::from(i.abs()) / f64::from(inner));
        let mut pts: Vec<Point> = (0..=inner).map(along).collect();
        pts.extend((1..n).map(|i| {
            let a = f64::from(i) / f64::from(n) * PI;
            Point::new(half * a.cos(), -rise - height * a.sin())
        }));
        pts.extend((-inner..0).map(along));
        let path = PathData::single(SubPath::polyline(&pts, true));
        let s = simplify_with(&path, &SimplifyOptions { tolerance, corner_angle_deg: 179.0, smooth, ..Default::default() });
        let sp = &s.subpaths[0];
        assert!(sp.closed && sp.anchors.len() < pts.len(), "{tolerance} pt: {s:?}");
        assert!(sp.segment_is_line(0), "{tolerance} pt: the first segment is a curve: {s:?}");
        let turn = largest_join_turn(sp);
        assert!(turn <= 0.5 + 1e-9, "{tolerance} pt: an anchor turns {turn}°: {s:?}");
    }
}

/// The degrees between the x axis and the direction `sp` leaves its first anchor along.
fn start_angle(sp: &SubPath) -> f64 {
    let c = sp.segment(0);
    let d = [c.p1 - c.p0, c.p2 - c.p0, c.p3 - c.p0].into_iter().find(|d| d.hypot() > 1e-9).unwrap_or_default();
    d.y.atan2(d.x).to_degrees()
}

/// With `free_start` and `free_end`, a smoothed open subpath leaves its first anchor and arrives at
/// its last one along the smoothed path, not along a jagged first or last segment, and its end points
/// stay. Only a flagged end turns, and a closed subpath, Object › Path › Smooth's selected segments
/// (`simplify_part`) and a refit without smoothing give the same result with the flags as without.
#[test]
fn free_ends_turn_only_open_smoothed_ends() {
    let pts: Vec<Point> = (0..=40).map(|i| Point::new(5.0 * f64::from(i), if i == 1 || i == 39 { 1.5 } else { 0.0 })).collect();
    let path = PathData::single(SubPath::polyline(&pts, false));
    let base = SimplifyOptions { tolerance: 2.5, corner_angle_deg: 179.0, smooth: 4.0, smooth_moved_only: true, ..Default::default() };
    let both = SimplifyOptions { free_start: true, free_end: true, ..base };
    let s = simplify_with(&path, &both);
    let sp = &s.subpaths[0];
    assert_eq!((sp.anchors[0].p, sp.anchors[sp.anchors.len() - 1].p), (pts[0], pts[40]), "{s:?}");
    let mut back = sp.clone();
    back.reverse();
    assert!(start_angle(sp).abs() <= 5.0 && (start_angle(&back).abs() - 180.0).abs() <= 5.0, "{s:?}");
    let start_only = simplify_with(&path, &SimplifyOptions { free_start: true, ..base });
    let plain = simplify_with(&path, &base);
    assert!(start_angle(&start_only.subpaths[0]).abs() <= 5.0, "{start_only:?}");
    assert!(start_angle(&plain.subpaths[0]) > 16.0, "{plain:?}");
    let (mut a, mut b) = (start_only.subpaths[0].clone(), plain.subpaths[0].clone());
    a.reverse();
    b.reverse();
    assert!((start_angle(&a) - start_angle(&b)).abs() <= 1e-9, "the last anchor keeps its direction: {start_only:?}");
    // Flags that must have no effect.
    let ring = PathData::single(SubPath::polyline(&pts[..39], true));
    assert_eq!(simplify_with(&ring, &both), simplify_with(&ring, &base));
    let unsmoothed = SimplifyOptions { smooth: 0.0, ..base };
    assert_eq!(simplify_with(&path, &SimplifyOptions { free_start: true, free_end: true, ..unsmoothed }), simplify_with(&path, &unsmoothed));
    let picked: std::collections::BTreeSet<(usize, usize)> = (0..20).map(|i| (0, i)).collect();
    assert_eq!(simplify_part(&path, &picked, &both).path, simplify_part(&path, &picked, &base).path);
}

/// A free end whose smoothed path turns back by more than 90° from its first segment keeps that
/// segment's direction: a path that leaves its start along x for 2 pt, then turns by 170°.
#[test]
fn a_free_end_that_turns_back_keeps_its_first_direction() {
    let back = kurbo::Vec2::from_angle(170f64.to_radians());
    let mut pts = vec![Point::ORIGIN, Point::new(2.0, 0.0)];
    pts.extend((1..=10).map(|i| Point::new(2.0, 0.0) + back * (3.0 * f64::from(i))));
    let path = PathData::single(SubPath::polyline(&pts, false));
    let opts =
        SimplifyOptions { tolerance: 2.5, corner_angle_deg: 179.0, smooth: 4.0, smooth_moved_only: true, free_start: true, ..Default::default() };
    let s = simplify_with(&path, &opts);
    let angle = start_angle(&s.subpaths[0]);
    assert!(angle.abs() <= 1.0, "the path leaves its start at {angle}°: {s:?}");
}

#[test]
fn simplify_straight_lines_option() {
    let c = dense_circle(50.0, 200);
    let s = simplify_with(&c, &SimplifyOptions { tolerance: 1.0, straight_lines: true, ..Default::default() });
    assert!(!has_curves(&s));
    assert!(s.anchor_count() < 40);
    assert!(max_deviation(&c, &s) <= 1.0 + 1e-6);
}

/// A closed subpath whose refit is one curve stays a closed subpath that encloses an area, in a
/// compound path and on its own, with curves and with Convert to Straight Lines.
#[test]
fn simplify_keeps_a_closed_subpath_that_refits_to_one_curve() {
    let small = |cx: f64, cy: f64| circle(cx, cy, 2.0).subpaths.remove(0);
    let square = rect(0.0, 0.0, 1000.0, 1000.0).subpaths.remove(0);
    let compound = PathData::new(vec![square, small(500.0, 500.0)]);
    let lone = PathData::single(small(0.0, 0.0));
    // A straight edge and a curve that bows 2.25 pt from it.
    let bow = PathData::single(SubPath::new(
        vec![
            Anchor::with_handles(Point::new(0.0, 0.0), Point::new(0.0, 3.0), Point::new(0.0, 0.0)),
            Anchor::with_handles(Point::new(100.0, 0.0), Point::new(100.0, 0.0), Point::new(100.0, 3.0)),
        ],
        true,
    ));
    // A straight edge and an S curve whose midpoints all lie on that edge.
    let s_curve = |h: f64, x: f64| {
        PathData::single(SubPath::new(
            vec![
                Anchor::with_handles(Point::new(0.0, 0.0), Point::new(0.0, 0.0), Point::new(0.0, h)),
                Anchor::with_handles(Point::new(100.0, 0.0), Point::new(x, -h), Point::new(100.0, 0.0)),
            ],
            true,
        ))
    };
    let (s_low, s_high) = (s_curve(3.0, 0.0), s_curve(13.5, 100.0));
    for straight_lines in [false, true] {
        for tolerance in [4.0, 10.0, 32.0] {
            let opts = SimplifyOptions { tolerance, straight_lines, ..Default::default() };
            for path in [&compound, &lone, &bow, &s_low] {
                let s = simplify_with(path, &opts);
                assert_eq!(s.subpaths.len(), path.subpaths.len(), "{tolerance} pt, straight lines {straight_lines}: {s:?}");
                assert!(s.subpaths.iter().all(|sp| sp.closed && sp.anchors.len() >= 2 && sp.area().abs() > 1.0), "{s:?}");
                assert!(max_deviation(path, &s) <= tolerance + 1e-6, "{tolerance} pt: {}", max_deviation(path, &s));
                let lines = s.subpaths.iter().all(|sp| (0..sp.segment_count()).all(|i| sp.segment_is_line(i)));
                assert!(lines || !straight_lines, "{tolerance} pt: {s:?}");
            }
        }
    }
    // Lobes on both sides of the edge, 3.9 pt from it, stay within the tolerance.
    for straight_lines in [false, true] {
        let s = simplify_with(&s_high, &SimplifyOptions { tolerance: 4.0, straight_lines, ..Default::default() });
        assert!(max_deviation(&s_high, &s) <= 4.0 + 1e-6, "straight lines {straight_lines}: {}", max_deviation(&s_high, &s));
    }
    // Curves whose handles lie on the line between their anchors become lines.
    let thin = PathData::single(SubPath::new(
        vec![
            Anchor::with_handles(Point::new(0.0, 0.0), Point::new(20.0, 0.0), Point::new(30.0, 0.0)),
            Anchor::with_handles(Point::new(100.0, 0.0), Point::new(70.0, 0.0), Point::new(80.0, 0.0)),
        ],
        true,
    ));
    let s = simplify_with(&thin, &SimplifyOptions { tolerance: 4.0, straight_lines: true, ..Default::default() });
    assert!(s.subpaths.iter().all(|sp| (0..sp.segment_count()).all(|i| sp.segment_is_line(i))), "{s:?}");
    // Anchors in line on a closed path are still removed.
    let in_line = PathData::single(SubPath::new([0.0, 25.0, 50.0, 100.0].map(|x| Anchor::corner(Point::new(x, 0.0))).to_vec(), true));
    let s = simplify_with(&in_line, &SimplifyOptions { tolerance: 4.0, ..Default::default() });
    assert!(s.subpaths[0].anchors.len() < 4, "{s:?}");
    let many = PathData::single(SubPath::new((0..5000).map(|i| Anchor::corner(Point::new(f64::from(i), 0.0))).collect(), true));
    for tolerance in [4.0, f64::INFINITY, f64::NAN] {
        for path in [&in_line, &thin, &many] {
            simplify_with(path, &SimplifyOptions { tolerance, straight_lines: true, ..Default::default() });
        }
    }
    // A long, thin rectangle keeps its corners.
    let long = rect(0.0, 0.0, 1e9, 0.5);
    let s = simplify_with(&long, &SimplifyOptions { tolerance: 0.1, straight_lines: true, ..Default::default() });
    assert!(max_deviation(&long, &s) <= 0.1 + 1e-6, "{s:?}");
}

/// An S curve closed by a straight edge of 20,000 anchors, whose curve shows only when it is split in
/// four, keeps its outline within the tolerance. With corner anchors on the edge, only the curve is
/// split. With smooth anchors whose handles lie along the edge, four pieces of every segment pass the
/// limit on pieces: the loop stays as it is, or with straight lines follows lines between samples.
#[test]
fn simplify_keeps_a_flat_loop_with_many_anchors_on_its_edge() {
    let n = 20_000;
    let step = 100.0 / f64::from(n);
    let curve_anchors = vec![
        Anchor::with_handles(Point::new(0.0, 0.0), Point::new(0.0, 0.0), Point::new(0.0, 3.0)),
        Anchor::with_handles(Point::new(100.0, 0.0), Point::new(0.0, -3.0), Point::new(100.0, 0.0)),
    ];
    let curve = PathData::single(SubPath::new(curve_anchors.clone(), false));
    for smooth_edge in [false, true] {
        let mut anchors = curve_anchors.clone();
        anchors.extend((1..n).map(|i| {
            let p = Point::new(100.0 - f64::from(i) * step, 0.0);
            if smooth_edge {
                Anchor::with_handles(p, Point::new(p.x + step / 3.0, 0.0), Point::new(p.x - step / 3.0, 0.0))
            } else {
                Anchor::corner(p)
            }
        }));
        let path = PathData::single(SubPath::new(anchors, true));
        for straight_lines in [false, true] {
            let s = simplify_with(&path, &SimplifyOptions { tolerance: 4.0, straight_lines, ..Default::default() });
            let at = format!("smooth edge {smooth_edge}, straight lines {straight_lines}");
            assert_eq!(s.subpaths.len(), 1, "{at}");
            let sp = &s.subpaths[0];
            assert!(sp.closed && sp.area().abs() > 1.0, "{at}: area {}", sp.area());
            assert!(max_deviation(&curve, &s) <= 4.0 + 1e-6, "{at}: {}", max_deviation(&curve, &s));
            assert!(!straight_lines || (0..sp.segment_count()).all(|i| sp.segment_is_line(i)), "{at}: a curve is left");
        }
    }
}

/// The Simplify slider's levels: 1 pt at 50, doubled for every 10 levels down and halved for every
/// 10 up; Auto-Simplify takes the lowest level whose tolerance is at most 1/400 of the size, and
/// level 100 for a size under 12.5 pt.
#[test]
fn simplify_levels_and_auto_simplify() {
    assert_eq!(simplify_level_tolerance(SIMPLIFY_LEVEL), 1.0);
    assert_eq!((simplify_level_tolerance(0.0), simplify_level_tolerance(100.0)), (32.0, 1.0 / 32.0));
    assert_eq!((simplify_level_tolerance(-5.0), simplify_level_tolerance(500.0), simplify_level_tolerance(f64::NAN)), (32.0, 1.0 / 32.0, 1.0));
    assert_eq!((auto_simplify_level(400.0), auto_simplify_level(100.0), auto_simplify_level(300.0)), (50.0, 70.0, 55.0));
    assert!(simplify_level_tolerance(auto_simplify_level(300.0)) <= 300.0 / 400.0);
    assert_eq!((auto_simplify_level(12.5), auto_simplify_level(10.0)), (100.0, 100.0));
    assert_eq!((auto_simplify_level(0.0), auto_simplify_level(f64::INFINITY), auto_simplify_level(1e9)), (100.0, 100.0, 0.0));
}

#[test]
fn smooth_rounds_corners() {
    let s = smooth(&rect(0.0, 0.0, 10.0, 10.0), 1.0);
    assert!(s.anchors().all(|(_, _, a)| a.has_in() && a.has_out()));
    let same = smooth(&rect(0.0, 0.0, 10.0, 10.0), 0.0);
    assert_eq!(same, rect(0.0, 0.0, 10.0, 10.0));
}

/// Object › Path › Smooth: a rectangle, whose corners no curve rounds within the tolerance, stays as
/// it is even at maximum smoothing; a polygon of 16 sides becomes a closed curve of fewer anchors once the tolerance
/// passes its 0.96 pt sagitta, smooth all round.
#[test]
fn smooth_level_keeps_corners_it_cannot_round_and_curves_polygons() {
    let r = rect(0.0, 0.0, 100.0, 50.0);
    assert_eq!(smooth_level(&r, 100.0), r);
    let pts: Vec<Point> = (0..16).map(|i| Point::new(50.0 * (f64::from(i) * PI / 8.0).cos(), 50.0 * (f64::from(i) * PI / 8.0).sin())).collect();
    let polygon = PathData::single(SubPath::polyline(&pts, true));
    let s = smooth_level(&polygon, SMOOTH_AUTO_LEVEL);
    assert!(s.subpaths.len() == 1 && s.subpaths[0].closed && s.anchor_count() < 8, "{}", s.anchor_count());
    assert!(s.subpaths[0].anchors.iter().all(|a| a.kind == AnchorKind::Smooth), "no kink, its start included: {s:?}");
    assert_eq!(smooth_level(&polygon, 0.0), polygon, "0.1 pt is less than the sagitta");
}

/// The largest distance from a point of `a` to `b` (sampled on `a`, exact on `b`).
fn one_way_distance(a: &PathData, b: &PathData) -> f64 {
    use kurbo::ParamCurveNearest;
    let mut worst = 0.0_f64;
    for sa in &a.subpaths {
        for i in 0..sa.segment_count() {
            let c = sa.segment(i);
            for k in 0..=64 {
                let q = c.eval(f64::from(k) / 64.0);
                let d = b
                    .subpaths
                    .iter()
                    .flat_map(|sb| (0..sb.segment_count()).map(move |j| sb.segment(j).nearest(q, 1e-9).distance_sq))
                    .fold(f64::INFINITY, f64::min);
                worst = worst.max(d.sqrt());
            }
        }
    }
    worst
}

/// Object › Path › Smooth at level 0 keeps a closed path within 0.1 % of its longer side, a sharp
/// corner where the path starts included (a drop with a 90° tip at its first anchor).
#[test]
fn smooth_level_0_keeps_a_closed_paths_start_corner_within_tolerance() {
    let n = 24;
    let step = 1.5 * PI / f64::from(n);
    let at = |t: f64| {
        let (r, dr) = (100.0 * (2.0 * t / 3.0).sin(), 200.0 / 3.0 * (2.0 * t / 3.0).cos());
        (Point::new(r * t.cos(), r * t.sin()), kurbo::Vec2::new(dr * t.cos() - r * t.sin(), dr * t.sin() + r * t.cos()))
    };
    let anchors = (0..n)
        .map(|i| {
            let (p, d_out) = at(f64::from(i) * step);
            let d_in = if i == 0 { at(1.5 * PI).1 } else { d_out };
            craft_geom::Anchor::with_handles(p, p - d_in * (step / 3.0), p + d_out * (step / 3.0))
        })
        .collect();
    let p = PathData::single(SubPath::new(anchors, true));
    let b = p.bounds().unwrap();
    let limit = 0.001 * b.width().max(b.height());
    let s = smooth_level(&p, 0.0);
    let moved = one_way_distance(&p, &s).max(one_way_distance(&s, &p));
    assert!(moved <= limit, "level 0 moved the path by {moved} pt, more than {limit} pt: {:?}", s.subpaths[0].anchors[0]);
}

#[test]
fn remove_redundant_collinear_and_curve() {
    let p = PathData::single(SubPath::polyline(
        &[Point::new(0.0, 0.0), Point::new(5.0, 0.0), Point::new(10.0, 0.0), Point::new(10.0, 10.0), Point::new(0.0, 10.0)],
        true,
    ));
    assert_eq!(remove_redundant_points(&p, 1e-6).anchor_count(), 4);
    let c = add_anchor_points(&circle(0.0, 0.0, 10.0));
    let n0 = c.anchor_count();
    let r = remove_redundant_points(&c, 1e-3);
    assert!(r.anchor_count() < n0, "{} vs {}", r.anchor_count(), n0);
    assert!(close(ar(&r), ar(&c), 1e-4));
}

#[test]
fn add_anchor_points_doubles() {
    let c = circle(0.0, 0.0, 10.0);
    let a = add_anchor_points(&c);
    assert_eq!(a.anchor_count(), 2 * c.anchor_count());
    assert!(close(ar(&a), ar(&c), 1e-9));
    let r = add_anchor_points(&rect(0.0, 0.0, 10.0, 10.0));
    assert!(r.anchors().any(|(_, _, a)| a.p == Point::new(5.0, 0.0)));
}

/// The one-pass build gives what inserting a midpoint into each segment (last first) gives, open or closed.
#[test]
fn add_anchor_points_matches_inserting() {
    let open = PathData::single(SubPath::new(circle(0.0, 0.0, 10.0).subpaths[0].anchors.to_vec(), false));
    for src in [circle(0.0, 0.0, 10.0), rect(0.0, 0.0, 10.0, 10.0), open] {
        let mut want = src.clone();
        for sp in &mut want.subpaths {
            for seg in (0..sp.segment_count()).rev() {
                sp.insert_anchor(seg, 0.5);
            }
        }
        assert_eq!(add_anchor_points(&src), want);
    }
}

#[test]
fn average_axes() {
    let p = PathData::single(SubPath::polyline(&[Point::new(0.0, 0.0), Point::new(10.0, 4.0), Point::new(5.0, 8.0)], false));
    let h = average(&p, &[(0, 0), (0, 1)], AverageAxis::Horizontal);
    assert_eq!(h.subpaths[0].anchors[0].p, Point::new(0.0, 2.0));
    assert_eq!(h.subpaths[0].anchors[1].p, Point::new(10.0, 2.0));
    let b = average(&p, &[(0, 0), (0, 1), (0, 2)], AverageAxis::Both);
    assert!(b.subpaths[0].anchors.iter().all(|a| a.p.distance(Point::new(5.0, 4.0)) < 1e-12));
    let v = average(&p, &[(0, 0), (0, 1)], AverageAxis::Vertical);
    assert_eq!(v.subpaths[0].anchors[0].p.x, 5.0);
}

#[test]
fn join_two_open_paths() {
    let a = PathData::single(SubPath::polyline(&[Point::new(0.0, 0.0), Point::new(10.0, 0.0)], false));
    let b = PathData::single(SubPath::polyline(&[Point::new(20.0, 5.0), Point::new(10.0, 0.0)], false));
    let j = join(&[a.clone(), b], 1e-6);
    assert_eq!(j.subpaths.len(), 1);
    assert_eq!(j.anchor_count(), 3);
    assert!(!j.subpaths[0].closed);
    let c = PathData::single(SubPath::polyline(&[Point::new(12.0, 0.0), Point::new(20.0, 0.0)], false));
    let j = join(&[a, c], 1e-6);
    assert_eq!(j.anchor_count(), 4);
}

#[test]
fn join_single_path_closes() {
    let a = PathData::single(SubPath::polyline(&[Point::new(0.0, 0.0), Point::new(10.0, 0.0), Point::new(5.0, 5.0)], false));
    let j = join(&[a], 1e-6);
    assert!(j.subpaths[0].closed);
    assert_eq!(j.anchor_count(), 3);
    let b = PathData::single(SubPath::polyline(&[Point::new(0.0, 0.0), Point::new(10.0, 0.0), Point::new(5.0, 5.0), Point::new(0.0, 0.0)], false));
    assert_eq!(join(&[b], 1e-6).anchor_count(), 3);
}

#[test]
fn grid_split() {
    let g = split_into_grid(Rect::new(0.0, 0.0, 100.0, 50.0), 2, 3, (7.0, 5.0));
    assert_eq!(g.len(), 6);
    let w = (100.0 - 10.0) / 3.0;
    let h = (50.0 - 7.0) / 2.0;
    assert!(g.iter().all(|c| close(ar(c), w * h, 1e-9)));
    assert_eq!(g[5].bounds().unwrap().x1, 100.0);
    assert!(split_into_grid(Rect::new(0.0, 0.0, 10.0, 10.0), 0, 3, (1.0, 1.0)).is_empty());
    assert!(split_into_grid(Rect::new(0.0, 0.0, 10.0, f64::NAN), 2, 3, (1.0, 1.0)).is_empty());
}

#[test]
#[ignore]
fn timing_unite_1000_circles() {
    let mut seed = 0x1234_5678_u64;
    let mut rnd = || {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        (seed % 1_000_000) as f64 / 1_000_000.0
    };
    let circles: Vec<Shape> = (0..1000).map(|i| shape(circle(rnd() * 1000.0, rnd() * 1000.0, 5.0 + rnd() * 30.0), i)).collect();
    let t = std::time::Instant::now();
    let r = pathfinder(PathfinderOp::Unite, &circles);
    println!("unite 1000 circles: {} ms, {} anchors", t.elapsed().as_millis(), r[0].path.anchor_count());
    assert!(!r.is_empty());
}

#[test]
fn remove_anchor_refits_the_curve_round_it() {
    use craft_geom::Anchor;
    let (p0, p3) = (Point::new(0.0, 0.0), Point::new(100.0, 0.0));
    let original =
        SubPath::new(vec![Anchor::with_handles(p0, p0, Point::new(30.0, 80.0)), Anchor::with_handles(p3, Point::new(70.0, 80.0), p3)], false);
    let mut sp = original.clone();
    let i = sp.insert_anchor(0, 0.4);
    assert!(remove_anchor(&mut sp, i));
    assert_eq!(sp.anchors.len(), 2);
    assert!(sp.anchors[0].h_out.distance(original.anchors[0].h_out) < 0.5, "{:?}", sp.anchors[0].h_out);
    assert!(sp.anchors[1].h_in.distance(original.anchors[1].h_in) < 0.5, "{:?}", sp.anchors[1].h_in);
    // A circle stays closed and round with one anchor fewer.
    let mut c = circle(0.0, 0.0, 50.0).subpaths.remove(0);
    let n = c.anchors.len();
    assert!(remove_anchor(&mut c, 1));
    assert!(c.closed && c.anchors.len() == n - 1);
    let a = ar(&PathData::single(c));
    assert!(close(a, PI * 2500.0, 0.03), "{a}");
}

#[test]
fn remove_anchor_between_straight_sides_draws_a_straight_one() {
    let mut sp = rect(0.0, 0.0, 100.0, 80.0).subpaths.remove(0);
    let n = sp.anchors.len();
    assert!(remove_anchor(&mut sp, 0));
    assert!(sp.closed && sp.anchors.len() == n - 1);
    assert!(!sp.anchors.iter().any(|a| a.has_in() || a.has_out()), "{sp:?}");
    let mut line = SubPath::polyline(&[Point::new(0.0, 0.0), Point::new(40.0, 30.0), Point::new(100.0, 0.0)], false);
    assert!(remove_anchor(&mut line, 1));
    assert!(line.anchors.len() == 2 && line.segment_is_line(0));
}

#[test]
fn remove_anchor_at_an_end_drops_its_segment_and_rejects_bad_input() {
    let mut sp = SubPath::polyline(&[Point::new(0.0, 0.0), Point::new(10.0, 0.0), Point::new(20.0, 5.0)], false);
    assert!(!remove_anchor(&mut sp, 3));
    assert!(remove_anchor(&mut sp, 0));
    assert_eq!(sp.anchors[0].p, Point::new(10.0, 0.0));
    // Non-finite points never panic.
    let mut bad = SubPath::polyline(&[Point::new(0.0, 0.0), Point::new(f64::NAN, 1.0), Point::new(f64::INFINITY, 0.0)], false);
    bad.anchors[0].h_out = Point::new(5.0, f64::NAN);
    assert!(remove_anchor(&mut bad, 1));
    assert_eq!(bad.anchors.len(), 2);
}
