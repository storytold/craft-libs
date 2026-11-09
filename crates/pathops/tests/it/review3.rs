//! Claims made in the third review of the pull request, each as the smallest test that fails when
//! the claim is true.

use craft_pathops::*;
use kurbo::{BezPath, ParamCurve, ParamCurveNearest, PathEl, Point, Rect, Shape as _};

use crate::common::{poly, rectangle, rel_close as close};

const NZ: FillRule = FillRule::NonZero;

fn open(pts: &[(f64, f64)]) -> BezPath {
    poly(pts, false)
}

fn closed(pts: &[(f64, f64)]) -> BezPath {
    poly(pts, true)
}

fn length(path: &BezPath) -> f64 {
    use kurbo::ParamCurveArclen;
    path.segments().map(|s| s.arclen(1e-9)).sum()
}

/// The default limits with `set` applied (`Limits` is `#[non_exhaustive]`, so it is not built with `..`).
fn limits(set: impl FnOnce(&mut Limits)) -> Limits {
    let mut l = Limits::default();
    set(&mut l);
    l
}

/// One cubic from `p[0]`, closed or not.
fn cubic(p: [(f64, f64); 4], shut: bool) -> BezPath {
    let mut path = BezPath::new();
    path.move_to(p[0]);
    path.curve_to(p[1], p[2], p[3]);
    if shut {
        path.close_path();
    }
    path
}

/// The operands of the regression the windings check was made for: their union is 4.375, and the
/// sweep alone gives 5.5.
fn operands_5478() -> (BezPath, BezPath) {
    (closed(&[(3.0, 1.0), (3.0, 3.0), (0.0, 1.0)]), closed(&[(3.0, 0.0), (0.0, 2.0), (3.0, 2.0), (0.0, 0.0), (0.0, 1.0)]))
}

// ---- booleans

#[test]
fn operands_that_are_only_points_do_not_switch_off_the_check_of_the_others() {
    let (a, b) = operands_5478();
    // Closed subpaths of two lines of no length: they draw a point, and the sweep gets no segment of them.
    let dots: Vec<BezPath> = (0..7)
        .map(|i| {
            let p = (10.0 + f64::from(i), 10.0);
            let mut dot = BezPath::new();
            dot.move_to(p);
            dot.line_to(p);
            dot.line_to(p);
            dot.close_path();
            dot
        })
        .collect();
    let mut inputs: Vec<(&BezPath, FillRule)> = vec![(&a, NZ), (&b, NZ)];
    inputs.extend(dots.iter().map(|d| (d, NZ)));
    let longest = std::cell::Cell::new(0usize);
    let union = |c: &[usize]| {
        longest.set(longest.get().max(c.len()));
        assert!(c.windows(2).all(|w| w[0] < w[1]), "the covering inputs ascend: {c:?}");
        c.contains(&0) || c.contains(&1)
    };
    let got = try_boolean_n(&inputs, union).expect("the union");
    assert!(close(area(&got, NZ), 4.375, 1e-6), "the union is {}, right is 4.375", area(&got, NZ));
    assert_eq!(longest.get(), 2, "the predicate is given the inputs that cover a face, not a flag for each of the nine");
    assert!(close(area(&boolean_n(&inputs, |c| c.contains(&0) || c.contains(&1)), NZ), 4.375, 1e-6));
}

#[test]
fn the_predicate_of_a_rectangle_is_asked_once_for_each_of_its_two_windings() {
    let square = rectangle(Rect::new(0.0, 0.0, 10.0, 10.0));
    let calls = std::cell::Cell::new(0usize);
    let got = try_boolean_n(&[(&square, NZ)], |c| {
        calls.set(calls.get() + 1);
        c.contains(&0)
    })
    .expect("the square");
    assert!(close(area(&got, NZ), 100.0, 1e-9));
    // Four segments, eight sides, and two windings: outside and inside.
    assert_eq!(calls.get(), 2);
}

// ---- loops of one cubic

#[test]
fn a_loop_of_one_cubic_a_trillion_long_and_half_a_unit_wide_keeps_its_area() {
    let thin = cubic([(0.0, 0.0), (1e12, 0.0), (1e12, 0.5), (0.0, 0.0)], true);
    let want = thin.area().abs();
    assert!(close(want, 7.5e10, 1e-9), "{want}");
    let got = try_area(&thin, NZ).expect("an area");
    assert!(close(got, want, 1e-6), "{got} against {want}");
    // Handles along one line still enclose nothing: along an axis, and rotated (each handle rounded).
    let flat = cubic([(0.0, 0.0), (1e12, 0.0), (5e11, 0.0), (0.0, 0.0)], true);
    assert_eq!(try_area(&flat, NZ), Ok(0.0));
    let (c, s) = (0.3f64.cos(), 0.3f64.sin());
    let rotated = cubic([(0.0, 0.0), (1e12 * c, 1e12 * s), (5e11 * c, 5e11 * s), (0.0, 0.0)], true);
    assert_eq!(try_area(&rotated, NZ), Ok(0.0));
    // The same thin loop turned by 45 degrees is a loop too.
    let h = std::f64::consts::FRAC_1_SQRT_2;
    let (u, v) = ((h, h), (-h, h));
    let at = |a: f64, b: f64| (a * u.0 + b * v.0, a * u.1 + b * v.1);
    let turned = cubic([(0.0, 0.0), at(1e12, 0.0), at(1e12, 0.5), (0.0, 0.0)], true);
    let got = try_area(&turned, NZ).expect("an area");
    assert!(close(got, turned.area().abs(), 1e-3) && got > 7e10, "{got}");
}

// ---- join

#[test]
fn join_closes_one_cubic_that_comes_back_within_the_tolerance_on_its_start() {
    let near = cubic([(0.0, 0.0), (100.0, 0.0), (0.0, 100.0), (0.5, 0.5)], false);
    let joined = join(&[near], 1.0);
    let o = Point::new(0.0, 0.0);
    assert_eq!(
        joined.elements(),
        &[PathEl::MoveTo(o), PathEl::CurveTo(Point::new(100.0, 0.0), Point::new(0.0, 100.0), o), PathEl::ClosePath],
        "the end is merged into the start, with no closing segment"
    );
    assert!(close(joined.area().abs(), 1_500.0, 1e-9));
    // A line has no loop to close: it is still closed with a line back.
    let line = open(&[(0.0, 0.0), (0.5, 0.0)]);
    assert_eq!(join(&[line], 1.0), closed(&[(0.0, 0.0), (0.5, 0.0)]));
    // Nor has a cubic whose handles lie along one line.
    let flat = cubic([(0.0, 0.0), (10.0, 0.0), (5.0, 0.0), (0.5, 0.0)], false);
    assert_eq!(join(&[flat], 1.0).elements().len(), 4, "move, the cubic, a line back and the close");
}

// ---- the planar map

/// `M0,0 C100,0 0,100 0,0`, with or without a `Z`: one cubic that loops round an area of 1,500.
fn loop_path(shut: bool) -> BezPath {
    cubic([(0.0, 0.0), (100.0, 0.0), (0.0, 100.0), (0.0, 0.0)], shut)
}

#[test]
fn divide_fills_a_loop_of_one_cubic_that_comes_back_to_its_start_without_a_close() {
    let (ring, shut) = (loop_path(false), loop_path(true));
    let cutter = || open(&[(-10.0, 10.0), (150.0, 10.0)]);
    let want = shut.area().abs();
    let pieces = pathfinder(PathfinderOp::Divide, &[Shape::new(ring.clone(), NZ, 7), Shape::new(cutter(), NZ, 9)]);
    assert_eq!(pieces.len(), 2, "the loop cut in two");
    assert!(pieces.iter().all(|p| p.key == 7));
    let total: f64 = pieces.iter().map(|p| p.path.area().abs()).sum();
    assert!(close(total, want, 1e-6), "{total} against {want}");
    // The closed form gives the same faces.
    let of_shut = pathfinder(PathfinderOp::Divide, &[Shape::new(shut, NZ, 7), Shape::new(cutter(), NZ, 9)]);
    assert_eq!(of_shut.len(), 2);
    // The planar map lists the loop as the fill of its faces.
    let (faces, _) = live_paint(&[Shape::new(ring.clone(), NZ, 7), Shape::new(cutter(), NZ, 9)]);
    assert_eq!(faces.iter().filter(|f| f.sources == [0]).count(), 2, "{:?}", faces.iter().map(|f| &f.sources).collect::<Vec<_>>());
    // Past the map's limit the fallback fills the loop too.
    let fallback = with_limits(limits(|l| l.shape_builder_segments = 1), || {
        pathfinder(PathfinderOp::Divide, &[Shape::new(ring.clone(), NZ, 7), Shape::new(cutter(), NZ, 9)])
    });
    let total: f64 = fallback.iter().map(|p| p.path.area().abs()).sum();
    assert!(fallback.iter().all(|p| p.key == 7) && close(total, want, 1e-6), "{total} in {} pieces", fallback.len());
    // Other open paths are cutting edges as they were: a ring of four anchors back to its start fills nothing.
    let rect_ring = open(&[(0.0, 0.0), (100.0, 0.0), (100.0, 100.0), (0.0, 100.0), (0.0, 0.0)]);
    assert!(pathfinder(PathfinderOp::Divide, &[Shape::new(rect_ring.clone(), NZ, 7), Shape::new(cutter(), NZ, 9)]).is_empty());
    let (faces, _) = live_paint(&[Shape::new(rect_ring, NZ, 7), Shape::new(cutter(), NZ, 9)]);
    assert!(faces.len() == 2 && faces.iter().all(|f| f.sources.is_empty()));
}

#[test]
fn a_shape_near_the_coordinate_limit_is_mapped_in_a_later_shear() {
    // Its top right corner is past the sweep's limit in the frame sheared by 1/32 (y + x / 32 is
    // 7.06e13), and within it in the one by 1/64.
    let big = rectangle(Rect::new(0.0, 6.0e13, 5e13, 6.9e13));
    let want = big.area().abs();
    assert!(try_area(&big, NZ).is_ok_and(|a| close(a, want, 1e-9)), "the shape is within the limit as it is");
    let (faces, edges) = live_paint(&[Shape::new(big, NZ, 0)]);
    assert_eq!(faces.len(), 1, "one face");
    assert_eq!(faces[0].sources, [0]);
    assert!(close(faces[0].path.area().abs(), want, 1e-6));
    assert!(!edges.is_empty());
    // A path that is not finite still has no map.
    let mut bad = rectangle(Rect::new(0.0, 0.0, 1.0, 1.0));
    bad.line_to((f64::NAN, 0.0));
    assert_eq!(live_paint(&[Shape::new(bad, NZ, 0)]), (Vec::new(), Vec::new()));
}

#[test]
fn a_short_open_edge_beside_art_far_away_is_kept_in_a_frame_where_it_is_not_level() {
    // Beside a square at 1e11 the sweep's tolerance is about 1.5e-3: edges 0.0075 to 0.0115 long are
    // five to eight of it, which the test of level edges passed over, and level in the usual frame
    // (they rise a thirty-second of their length there, less than the tolerance).
    for len in [0.0075, 0.009, 0.0115] {
        let shapes = [
            Shape::new(rectangle(Rect::new(0.0, 0.0, 100.0, 100.0)), NZ, 0),
            Shape::new(rectangle(Rect::new(1e11, 1e11, 1e11 + 1.0, 1e11 + 1.0)), NZ, 1),
            Shape::new(open(&[(300.0, 300.0), (300.0 + len, 300.0)]), NZ, 2),
        ];
        let (_, edges) = live_paint(&shapes);
        let kept: f64 = edges.iter().filter(|e| e.key == 2).map(|e| length(&e.path)).sum();
        assert!(close(kept, len, 1e-9), "an edge {len} long: {kept} of it kept");
    }
}

#[test]
fn ten_open_lines_each_level_in_one_of_the_ten_frames_are_mapped_and_outlined() {
    // Slopes -1/32, -1/64, ... -1/2, a thousand apart, so that none crosses another.
    let slopes = [32.0, 64.0, 16.0, 128.0, 8.0, 256.0, 512.0, 4.0, 1024.0, 2.0];
    let lines: Vec<Shape> = slopes
        .iter()
        .enumerate()
        .map(|(i, k)| Shape::new(open(&[(0.0, 1000.0 * i as f64), (1024.0, 1000.0 * i as f64 - 1024.0 / k)]), NZ, i as u64))
        .collect();
    let (_, edges) = live_paint(&lines);
    for (i, s) in lines.iter().enumerate() {
        let kept: f64 = edges.iter().filter(|e| e.key == i as u64).map(|e| length(&e.path)).sum();
        assert!(close(kept, length(&s.path), 1e-9), "line {i}: {kept} of {}", length(&s.path));
    }
    let outline = pathfinder(PathfinderOp::Outline, &lines);
    assert_eq!(outline.len(), 10, "each line, whole");
}

// ---- Outline

#[test]
fn outline_cuts_an_open_cubic_at_both_visits_of_the_point_where_it_crosses_itself() {
    // It crosses itself at t = (1 - sqrt(9 / 13)) / 2 and at 1 minus that.
    let curl = cubic([(0.0, 0.0), (100.0, 100.0), (-100.0, 100.0), (50.0, 0.0)], false);
    let t1 = (1.0 - (9.0f64 / 13.0).sqrt()) / 2.0;
    let seg = kurbo::CubicBez::new((0.0, 0.0), (100.0, 100.0), (-100.0, 100.0), (50.0, 0.0));
    let (p, q) = (seg.eval(t1), seg.eval(1.0 - t1));
    assert!(p.distance(q) < 1e-9, "{p:?} {q:?}");
    let pieces = pathfinder(PathfinderOp::Outline, &[Shape::new(curl.clone(), NZ, 1)]);
    assert_eq!(pieces.len(), 3, "before the crossing, the loop, and after it");
    let total: f64 = pieces.iter().map(|s| length(&s.path)).sum();
    assert!(close(total, length(&curl), 1e-6), "{total} against {}", length(&curl));
    // The loop starts and ends at the crossing.
    let ends = |s: &Shape| {
        let pts: Vec<Point> = s.path.elements().iter().filter_map(PathEl::end_point).collect();
        (pts.first().copied(), pts.last().copied())
    };
    assert!(pieces.iter().any(|s| matches!(ends(s), (Some(a), Some(b)) if a.distance(p) < 1e-6 && b.distance(p) < 1e-6)));
    // Closed, the outline is cut at the crossing twice too: the loop, and the rest.
    let mut shut = curl.clone();
    shut.line_to((50.0, -50.0));
    shut.line_to((0.0, -50.0));
    shut.close_path();
    let pieces = pathfinder(PathfinderOp::Outline, &[Shape::new(shut.clone(), NZ, 1)]);
    assert_eq!(pieces.len(), 2, "the loop, and the rest of the outline");
    // The whole outline, each piece from the crossing to the crossing, and not the loop twice: one
    // piece passes through (50, -50).
    let total: f64 = pieces.iter().map(|s| length(&s.path)).sum();
    assert!(close(total, length(&shut), 1e-6), "{total} against {}", length(&shut));
    for s in &pieces {
        assert!(matches!(ends(s), (Some(a), Some(b)) if a.distance(p) < 1e-6 && b.distance(p) < 1e-6), "{:?}", ends(s));
    }
    let corner = Point::new(50.0, -50.0);
    assert!(pieces.iter().any(|s| s.path.elements().iter().filter_map(PathEl::end_point).any(|q| q.distance(corner) < 1e-9)));
}

#[test]
fn outline_of_a_loop_too_long_for_an_accuracy_of_a_millionth_is_its_edge() {
    // About 2.6e13 long: the solver for the middle of it at an accuracy of 1e-6 asks for 2^65 steps.
    let big = cubic([(0.0, 0.0), (1e13, 0.0), (1e13, 1e13), (0.0, 0.0)], true);
    let pieces = pathfinder(PathfinderOp::Outline, &[Shape::new(big.clone(), NZ, 3)]);
    assert_eq!(pieces.len(), 1, "the loop's own edge");
    assert!(close(length(&pieces[0].path), length(&big), 1e-6));
}

// ---- interior points

#[test]
fn an_interior_point_is_found_in_unit_squares_hundreds_of_trillions_out_in_x() {
    for x in [1e14, 4e14] {
        let mut two = rectangle(Rect::new(x, 0.0, x + 1.0, 1.0));
        two.extend(rectangle(Rect::new(x, 100.0, x + 1.0, 101.0)).iter());
        let p = interior_point(&two).unwrap_or_else(|| panic!("no point in two squares at x = {x}"));
        assert!(two.winding(p) != 0, "{p:?}");
    }
}

#[test]
fn a_retrace_through_other_vertices_than_it_went_out_by_does_not_hide_the_square_beside_it() {
    let square = closed(&[(0.0, 400.3), (1.0, 400.3), (1.0, 401.3), (0.0, 401.3)]);
    // Out along x = 5 through every integer height, back through every half.
    let up: Vec<(f64, f64)> = (0..=1000).map(|y| (5.0, f64::from(y))).collect();
    let down: Vec<(f64, f64)> = (0..1000).rev().map(|y| (5.0, f64::from(y) + 0.5)).chain([(5.0, 0.0)]).collect();
    // And the same along a slanted line, whose points are rounded.
    let slant_up: Vec<(f64, f64)> = (0..=1000).map(|y| (20.0 + 0.3 * f64::from(y), f64::from(y))).collect();
    let slant_down: Vec<(f64, f64)> = (0..1000).rev().map(|y| (20.0 + 0.3 * (f64::from(y) + 0.5), f64::from(y) + 0.5)).chain([(20.0, 0.0)]).collect();
    for stub in [open(&[up, down].concat()), open(&[slant_up, slant_down].concat())] {
        let mut p = square.clone();
        p.extend(stub.iter());
        let at = interior_point(&p).expect("a point inside the square");
        assert!(p.winding(at) != 0 && at.x < 1.0 && at.y > 400.3 && at.y < 401.3, "{at:?}");
    }
}

// ---- offset

#[test]
fn an_inset_does_not_stroke_a_small_loop_of_its_subpath_inside_the_fill() {
    // A square 1e8 across whose subpath goes up from its bottom edge, round a small square wound the
    // same way, and back: 1.4 of edges not on the boundary, under the allowance of the lengths (1.6).
    let mut curl = BezPath::new();
    for (i, &(x, y)) in [
        (0.0, 0.0),
        (5e7, 0.0),
        (5e7, 0.5),
        (5e7 + 0.1, 0.5),
        (5e7 + 0.1, 0.6),
        (5e7, 0.6),
        (5e7, 0.5),
        (5e7, 0.0),
        (1e8, 0.0),
        (1e8, 1e8),
        (0.0, 1e8),
    ]
    .iter()
    .enumerate()
    {
        if i == 0 {
            curl.move_to((x, y));
        } else {
            curl.line_to((x, y));
        }
    }
    curl.close_path();
    let inset = try_offset_path(&curl, -2.0, Join::Miter, 4.0).expect("an inset");
    let plain = try_offset_path(&rectangle(Rect::new(0.0, 0.0, 1e8, 1e8)), -2.0, Join::Miter, 4.0).expect("an inset");
    // 2.3 above the bottom edge, over the loop: in the inset of the square, and 1.7 from the loop.
    let over = Point::new(5e7 + 0.05, 2.3);
    assert!(plain.winding(over) != 0, "the inset of the square holds it");
    assert!(inset.winding(over) != 0, "no channel is cut round the loop");
    assert_eq!(inset.elements().iter().filter(|e| matches!(e, PathEl::MoveTo(_))).count(), 1);
}

// ---- simplify

/// The largest distance from the curve `c`, probed at `n` parameters and at `extra` ones, to `path`.
fn curve_to_path(c: &kurbo::CubicBez, path: &BezPath, n: usize, extra: &[f64]) -> f64 {
    let probes = (0..=n).map(|k| k as f64 / n as f64).chain(extra.iter().copied());
    probes.map(|t| path.segments().map(|s| s.nearest(c.eval(t), 1e-12).distance_sq).fold(f64::INFINITY, f64::min).sqrt()).fold(0.0, f64::max)
}

#[test]
fn straight_simplification_keeps_the_curve_within_the_tolerance_between_its_samples() {
    // A curve whose highest point, 1.000002 above its chord, falls between two of its 257 samples,
    // all of which are within 1 of the chord: x = 300 t, y = a t (1 - t) (1 + 0.02 (2t - 1)).
    let e = 0.02;
    let f = |t: f64| t * (1.0 - t) * (1.0 + e * (2.0 * t - 1.0));
    let top = ((-2.0 + 6.0 * e) + ((-2.0 + 6.0 * e).powi(2) + 24.0 * e * (1.0 - e)).sqrt()) / (12.0 * e);
    let a = 1.000_002 / f(top);
    let c = kurbo::CubicBez::new((0.0, 0.0), (100.0, a * (1.0 - e) / 3.0), (200.0, a * (1.0 + e) / 3.0), (300.0, 0.0));
    assert!(close(c.eval(top).y, 1.000_002, 1e-12));
    let samples = (0..=256).map(|k| c.eval(f64::from(k) / 256.0).y).fold(0.0, f64::max);
    assert!(samples < 1.0, "every sample is within the tolerance of the chord: {samples}");
    let mut path = BezPath::new();
    path.move_to(c.p0);
    path.curve_to(c.p1, c.p2, c.p3);
    let opts = SimplifyOptions::default().tolerance(1.0).straight_lines(true);
    let simpler = simplify_with(&path, &opts);
    assert!(simpler.segments().count() >= 1 && simpler.segments().all(|s| matches!(s, kurbo::PathSeg::Line(_))));
    let off = curve_to_path(&c, &simpler, 4096, &[top]);
    assert!(off <= 1.0, "the curve is {off} from its simplification, past the tolerance of 1");
}

// ---- stroke regions

#[test]
fn a_single_outline_is_held_to_the_element_limit_as_several_are() {
    // A hatch of 20 strokes each way, one outline of 200 elements whose boundary is about 1,800 segments.
    let hatch = hatch(20);
    let few = limits(|l| l.elements = 1_000);
    assert!(hatch.elements().len() < few.elements);
    assert_eq!(with_limits(few, || try_stroke_region(std::slice::from_ref(&hatch))), Err(PathOpsError::TooComplex));
    assert!(with_limits(few, || stroke_region(std::slice::from_ref(&hatch))).elements().is_empty());
    // With room it is made.
    let roomy = limits(|l| l.elements = 10_000);
    assert!(with_limits(roomy, || try_stroke_region(std::slice::from_ref(&hatch))).is_ok_and(|p| !p.elements().is_empty()));
}

/// `m` thin strokes slanting each way across one another, as one path (the crate's own test hatch).
fn hatch(m: usize) -> BezPath {
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
