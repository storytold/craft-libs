//! Claims made in the second review of the pull request, each as the smallest test that fails when
//! the claim is true.

use craft_pathops::*;
use kurbo::{BezPath, ParamCurve, PathEl, Point, Rect, Shape as _};

use crate::common::{distance_to, poly, rectangle, rel_close as close};

const NZ: FillRule = FillRule::NonZero;
const KAPPA: f64 = 0.552_284_749_830_793_6;

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

fn subpaths(path: &BezPath) -> usize {
    path.elements().iter().filter(|e| matches!(e, PathEl::MoveTo(_))).count()
}

/// `M0,0 C100,0 0,100 0,0`, with or without a `Z`: one cubic that comes back to where it started
/// and loops round an area of 1,500.
fn loop_path(shut: bool) -> BezPath {
    let mut p = BezPath::new();
    p.move_to((0.0, 0.0));
    p.curve_to((100.0, 0.0), (0.0, 100.0), (0.0, 0.0));
    if shut {
        p.close_path();
    }
    p
}

/// The largest distance from the outline of `a` (probed at nine points of each segment, between its
/// ends too) to the outline of `b`.
fn deviation_of(a: &BezPath, b: &BezPath) -> f64 {
    let mut worst: f64 = 0.0;
    for seg in a.segments() {
        for k in 0..=8 {
            worst = worst.max(distance_to(b, seg.eval(f64::from(k) / 8.0)));
        }
    }
    worst
}

// ---- offset

/// Every subpath of `path` is closed.
fn all_closed(path: &BezPath) -> bool {
    subpaths(path) == path.elements().iter().filter(|e| matches!(e, PathEl::ClosePath)).count()
}

#[test]
fn a_cubic_that_returns_to_its_start_is_offset_as_a_closed_loop() {
    let inside = Point::new(20.0, 20.0);
    // The offsets of the loop (area 1,500) with round joins: grown by 0.5 and 2, shrunk by 0.5 and 2.
    for (delta, want) in [(0.5, 1572.81), (2.0, 1800.55), (-0.5, 1428.82), (-2.0, 1225.43)] {
        // The fallible form: `offset_path` gives the input back when the offset fails, which would pass
        // the checks below as the loop itself.
        let by_open = try_offset_path(&loop_path(false), delta, Join::Round, 4.0).expect("an offset of the open loop");
        let by_shut = try_offset_path(&loop_path(true), delta, Join::Round, 4.0).expect("an offset of the closed loop");
        for offset in [&by_open, &by_shut] {
            assert!(offset.winding(inside) != 0, "the loop's interior is in its offset by {delta}");
            assert!(all_closed(offset), "an outline, not the open loop: {offset:?}");
            assert!(close(area(offset, NZ), want, 1e-4), "offset {delta}: {} against {want}", area(offset, NZ));
        }
        assert!(close(area(&by_open, NZ), area(&by_shut, NZ), 1e-6), "offset {delta}: {} against {}", area(&by_open, NZ), area(&by_shut, NZ));
    }
}

#[test]
fn an_inset_does_not_stroke_a_small_shape_inside_a_huge_one_of_the_same_winding() {
    let mut nested = rectangle(Rect::new(0.0, 0.0, 1e6, 1e6));
    nested.extend(rectangle(Rect::new(500.0, 500.0, 501.0, 501.0)).iter());
    let inset = offset_path(&nested, -0.1, Join::Miter, 4.0);
    let want = (1e6 - 0.2) * (1e6 - 0.2);
    assert!((area(&inset, NZ) - want).abs() < 0.05, "area {} against {want}", area(&inset, NZ));
    assert_eq!(subpaths(&inset), 1, "no moat round the small square");
}

/// Open cubics that turn more tightly than the radius they are offset by, so that the stroker's inner
/// offset inverts and the winding of its loop cancels where the points are really within the radius.
fn tight_turns() -> Vec<(BezPath, f64)> {
    let cubic = |p: [(f64, f64); 4], d: f64| {
        let mut path = BezPath::new();
        path.move_to(p[0]);
        path.curve_to(p[1], p[2], p[3]);
        (path, d)
    };
    vec![
        cubic([(3.8, 3.6), (0.3, 0.9), (2.2, 5.4), (6.6, 5.1)], 3.5),
        cubic([(0.07, 6.46), (6.7, 9.65), (3.19, 4.12), (1.18, 6.78)], 1.8),
        cubic([(3.89, 4.28), (7.19, 1.45), (7.92, 3.2), (3.63, 2.01)], 3.4),
        cubic([(1.03, 4.61), (6.72, 2.82), (0.35, 6.71), (4.34, 1.01)], 3.9),
    ]
}

#[test]
fn the_outline_of_an_open_curve_that_turns_inside_the_radius_has_no_holes_whichever_way_it_is_offset() {
    for (path, d) in tight_turns() {
        // The fallible form: `offset_path` gives the open curve back when the offset fails.
        let grown = try_offset_path(&path, d, Join::Round, 4.0).expect("the offset");
        let inset = try_offset_path(&path, -d, Join::Round, 4.0).expect("the inset");
        // Each is the outline of a stroke 2d wide (of a curve longer than d): closed, and more than 2d² in area.
        for offset in [&grown, &inset] {
            assert!(all_closed(offset) && area(offset, NZ) > 2.0 * d * d, "{d}: {offset:?}");
        }
        assert_eq!(subpaths(&grown), 1, "the offset by {d}");
        assert_eq!(subpaths(&inset), subpaths(&grown), "the offset by -{d} has the contours of the one by {d}");
        assert!(close(area(&grown, NZ), area(&inset, NZ), 1e-3), "{d}: {} against {}", area(&grown, NZ), area(&inset, NZ));
    }
}

// ---- Outline

/// `count` open hoods, each a different width: up from `(-x, 0)`, across, and down to `(x, 0)`. Their
/// own edges never meet, but the line that closes each one (along the axis) overlaps all the others'.
fn hoods(count: usize) -> Vec<Shape> {
    (0..count)
        .map(|i| {
            let x = 1000.0 + i as f64;
            let h = 10.0 * (i as f64 + 1.0);
            Shape::new(open(&[(-x, 0.0), (-x, h), (x, h), (x, 0.0)]), NZ, i as u64)
        })
        .collect()
}

#[test]
fn outline_of_open_paths_whose_closing_lines_overlap_a_lot_is_made_from_the_open_edges() {
    let shapes = hoods(100);
    let few = limits(|l| l.segment_pairs = 10_000);
    // The closing lines make the filled arrangement too complex for the limit; the map does not.
    assert!(with_limits(few, || regions(&shapes)).is_empty(), "the filled arrangement is over the limit");
    let divided = with_limits(few, || pathfinder(PathfinderOp::Outline, &shapes));
    assert_eq!(divided.len(), 100, "one piece for each hood");
    for s in &divided {
        let i = s.key as f64;
        let (x, h) = (1000.0 + i, 10.0 * (i + 1.0));
        assert!(close(length(&s.path), 2.0 * h + 2.0 * x, 1e-6), "hood {i}: {}", length(&s.path));
    }
    // And with the default limits it is the same.
    assert_eq!(pathfinder(PathfinderOp::Outline, &shapes).len(), 100);
}

#[test]
fn outline_gives_no_shapes_rather_than_unsplit_paths_when_the_map_of_an_open_path_is_over_its_limit() {
    let shapes = [Shape::new(rectangle(Rect::new(0.0, 0.0, 100.0, 100.0)), NZ, 1), Shape::new(open(&[(-10.0, 50.0), (110.0, 50.0)]), NZ, 2)];
    assert_eq!(pathfinder(PathfinderOp::Outline, &shapes).len(), 5, "with room: the rectangle in two pieces and the line in three");
    let small = limits(|l| l.shape_builder_segments = 2);
    let none = with_limits(small, || pathfinder(PathfinderOp::Outline, &shapes));
    assert!(none.is_empty(), "{} shapes, their crossings unknown", none.len());
    // Closed shapes are not affected: their outlines have the crossings in the filled areas.
    let two = [Shape::new(rectangle(Rect::new(0.0, 0.0, 100.0, 100.0)), NZ, 1), Shape::new(rectangle(Rect::new(50.0, 50.0, 150.0, 150.0)), NZ, 2)];
    let pieces = with_limits(small, || pathfinder(PathfinderOp::Outline, &two));
    assert_eq!(pieces.len(), 4, "{} pieces", pieces.len());
}

#[test]
fn outline_keeps_a_closed_cubic_that_loops_round_an_area() {
    let one = [Shape::new(loop_path(true), NZ, 7)];
    let pieces = pathfinder(PathfinderOp::Outline, &one);
    assert_eq!(pieces.len(), 1, "{} pieces", pieces.len());
    assert!(close(length(&pieces[0].path), length(&loop_path(true)), 1e-9));
    assert_eq!(pieces[0].key, 7);
    // Crossed by a rectangle, it is cut where they meet.
    let both = [one[0].clone(), Shape::new(rectangle(Rect::new(-10.0, 20.0, 200.0, 30.0)), NZ, 8)];
    let pieces = pathfinder(PathfinderOp::Outline, &both);
    let of = |key: u64| pieces.iter().filter(|s| s.key == key).count();
    assert!(of(7) >= 2, "the loop is cut by the rectangle's edges: {} pieces", of(7));
    // A loop with no inside is no outline.
    let mut flat = BezPath::new();
    flat.move_to((0.0, 0.0));
    flat.curve_to((10.0, 0.0), (20.0, 0.0), (0.0, 0.0));
    flat.close_path();
    assert!(pathfinder(PathfinderOp::Outline, &[Shape::new(flat, NZ, 9)]).is_empty());
}

// ---- the planar map

#[test]
fn an_open_line_is_not_lost_when_distant_art_widens_the_sweeps_tolerance() {
    // 5e-4 off level in the sweep's usual frame, which a shape at 1e11 (tolerance 1.5e-3) snaps level.
    let line = open(&[(0.0, 0.0), (100.0, -3.1245)]);
    let far = rectangle(Rect::new(1e11, 1e11, 1e11 + 1.0, 1e11 + 1.0));
    let shapes = [Shape::new(line, NZ, 0), Shape::new(far, NZ, 1)];
    let want = length(&shapes[0].path);
    let (_, edges) = live_paint(&shapes);
    let got: f64 = edges.iter().filter(|e| e.key == 0).map(|e| length(&e.path)).sum();
    assert!(close(got, want, 1e-6), "live_paint: the line is {want} long, its edges {got}");
    let built = shape_builder(&shapes, false);
    let got: f64 = built.lines.iter().filter(|e| e.key == 0).map(|e| length(&e.path)).sum();
    assert!(close(got, want, 1e-6), "shape_builder: the line is {want} long, its pieces {got}");
}

/// A rectangle crossed by an open line, with a feature of the open paths a tolerance or less long:
/// a separate stub, a near-duplicate point in the line, or the line as a cubic with its first handle
/// 1e-9 from its start. Such a side is level in every shear, and must not empty the map.
fn crossed_with_a_micro_feature(kind: u32, len: f64) -> Vec<Shape> {
    let square = Shape::new(rectangle(Rect::new(0.0, 0.0, 100.0, 100.0)), NZ, 1);
    let mut line = BezPath::new();
    line.move_to((-10.0, 50.0));
    match kind {
        2 => {
            line.line_to((40.0, 50.0));
            line.line_to((40.0 + len, 50.0 + len));
            line.line_to((110.0, 50.0));
        }
        3 => line.curve_to((-10.0 + len, 50.0 + len), (60.0, 70.0), (110.0, 50.0)),
        _ => line.line_to((110.0, 50.0)),
    }
    let mut shapes = vec![square, Shape::new(line, NZ, 2)];
    if kind == 1 {
        shapes.push(Shape::new(open(&[(300.0, 300.0), (300.0 + len, 300.0 + len)]), NZ, 3));
    }
    shapes
}

#[test]
fn a_micro_feature_of_an_open_path_does_not_empty_the_map() {
    for (kind, len) in [(1, 1e-7), (2, 1e-7), (3, 1e-9)] {
        let shapes = crossed_with_a_micro_feature(kind, len);
        let (faces, edges) = live_paint(&shapes);
        assert_eq!((faces.len(), edges.len()), (2, 5), "live_paint, kind {kind}");
        let built = shape_builder(&shapes, true);
        assert_eq!((built.regions.len(), built.lines.len()), (2, 3), "shape_builder, kind {kind}");
        // The stub of the first is a path of its own, a piece of Outline (the map cannot see it).
        assert_eq!(pathfinder(PathfinderOp::Outline, &shapes).len(), if kind == 1 { 6 } else { 5 }, "Outline, kind {kind}");
        assert_eq!(pathfinder(PathfinderOp::Divide, &shapes).len(), 2, "Divide, kind {kind}");
    }
    // A feature of 1e-5 is long enough to be one: the stub is a piece of its own.
    let (faces, edges) = live_paint(&crossed_with_a_micro_feature(1, 1e-5));
    assert_eq!((faces.len(), edges.len()), (2, 6));
}

#[test]
fn open_lines_at_all_six_first_slopes_are_all_kept() {
    // Slopes -1/32, -1/64, -1/16, -1/128, -1/8, -1/256: each is level in one of the sweep's first six frames.
    let rises = [-8.0, -4.0, -16.0, -2.0, -32.0, -1.0];
    let shapes: Vec<Shape> =
        rises.iter().enumerate().map(|(i, &dy)| Shape::new(open(&[(0.0, 100.0 * i as f64), (256.0, 100.0 * i as f64 + dy)]), NZ, i as u64)).collect();
    let (_, edges) = live_paint(&shapes);
    for (i, s) in shapes.iter().enumerate() {
        let got: f64 = edges.iter().filter(|e| e.key == i as u64).map(|e| length(&e.path)).sum();
        assert!(close(got, length(&s.path), 1e-6), "line {i}: {} long, its edges {got}", length(&s.path));
    }
}

#[test]
fn an_interior_point_is_found_in_components_ten_billion_apart() {
    let mut two = closed(&[(0.0, 0.0), (1.0, 0.0), (1.0, 1.0), (0.0, 1.0)]);
    two.extend(closed(&[(0.0, 1e10), (1.0, 1e10), (1.0, 1e10 + 1.0), (0.0, 1e10 + 1.0)]).iter());
    let p = interior_point(&two).expect("a point inside one of the squares");
    assert!(two.winding(p) != 0, "{p:?}");
}

#[test]
fn an_interior_point_is_found_in_components_a_trillion_out_in_x() {
    for x in [1e11, 1e12, 1e13] {
        let mut two = rectangle(Rect::new(x, 0.0, x + 1.0, 1.0));
        two.extend(rectangle(Rect::new(x, 100.0, x + 1.0, 101.0)).iter());
        let p = interior_point(&two).unwrap_or_else(|| panic!("no point in two squares at x = {x}"));
        assert!(two.winding(p) != 0, "{p:?}");
    }
}

#[test]
fn a_path_that_retraces_itself_does_not_hide_the_square_it_is_beside() {
    let mut p = closed(&[(0.0, 400.3), (1.0, 400.3), (1.0, 401.3), (0.0, 401.3)]);
    // Out along x = 5 through a vertex at every integer height and back through the same ones.
    let out: Vec<(f64, f64)> = (0..=1000).map(|y| (5.0, f64::from(y))).collect();
    let back: Vec<(f64, f64)> = (0..1000).rev().map(|y| (5.0, f64::from(y))).collect();
    p.extend(open(&[out, back].concat()).iter());
    let at = interior_point(&p).expect("a point inside the square");
    assert!(p.winding(at) != 0 && at.x < 1.0 && at.y > 400.3 && at.y < 401.3, "{at:?}");
}

// ---- booleans

#[test]
// The predicate's panic is caught only where panics unwind; where they abort it ends the process.
#[cfg(panic = "unwind")]
fn a_panic_of_the_predicate_in_the_recovery_is_an_error_not_a_retry() {
    let (a, b) = (closed(&[(3.0, 1.0), (3.0, 3.0), (0.0, 1.0)]), closed(&[(3.0, 0.0), (0.0, 2.0), (3.0, 2.0), (0.0, 0.0), (0.0, 1.0)]));
    let asked = std::cell::Cell::new(false);
    // The first time it is asked it panics, the second time it answers.
    let union = |c: &[usize]| {
        assert!(asked.replace(true), "first call");
        c.contains(&0) || c.contains(&1)
    };
    let got = try_boolean_n(&[(&a, NZ), (&b, NZ)], union).map(|p| area(&p, NZ));
    assert_eq!(got, Err(PathOpsError::Degenerate), "a panic is an error");
}

#[test]
fn empty_inputs_do_not_switch_off_the_check_of_the_others() {
    let (a, b) = (closed(&[(3.0, 1.0), (3.0, 3.0), (0.0, 1.0)]), closed(&[(3.0, 0.0), (0.0, 2.0), (3.0, 2.0), (0.0, 0.0), (0.0, 1.0)]));
    let empty = BezPath::new();
    let mut inputs: Vec<(&BezPath, FillRule)> = vec![(&a, NZ), (&b, NZ)];
    inputs.extend(std::iter::repeat_n((&empty, NZ), 7));
    let got = try_boolean_n(&inputs, |c| c.contains(&0) || c.contains(&1)).expect("the union");
    assert!(close(area(&got, NZ), 4.375, 1e-6), "the union is {}, right is 4.375", area(&got, NZ));
    assert!(close(area(&boolean_n(&inputs, |c| c.contains(&0) || c.contains(&1)), NZ), 4.375, 1e-6));
}

#[test]
fn a_thin_loop_of_one_cubic_is_kept() {
    let mut p = BezPath::new();
    p.move_to((0.0, 0.0));
    p.curve_to((1e9, 0.0), (1e9, 1.0), (0.0, 0.0));
    p.close_path();
    let want = p.area().abs();
    assert!(close(want, 1.5e8, 1e-9), "{want}");
    assert!(close(try_area(&p, NZ).unwrap(), want, 1e-6), "{:?}", try_area(&p, NZ));
    let united = try_unite_all(&[(&p, NZ)]).unwrap();
    assert!(close(area(&united, NZ), want, 1e-6), "the union has area {}", area(&united, NZ));
    // Still no inside for a loop whose handles lie along one line.
    let mut flat = BezPath::new();
    flat.move_to((0.0, 0.0));
    flat.curve_to((1e9, 0.0), (5e8, 0.0), (0.0, 0.0));
    flat.close_path();
    assert_eq!(try_area(&flat, NZ), Ok(0.0));
    let mut slanted = BezPath::new();
    slanted.move_to((10.0, 20.0));
    slanted.curve_to((110.0, 120.0), (210.0, 220.0), (10.0, 20.0));
    slanted.close_path();
    assert_eq!(try_area(&slanted, NZ), Ok(0.0));
}

// ---- simplify

#[test]
fn straight_simplification_of_a_huge_curve_stays_within_the_tolerance_between_its_samples() {
    let r = 5e5;
    let mut quarter = BezPath::new();
    quarter.move_to((r, 0.0));
    quarter.curve_to((r, KAPPA * r), (KAPPA * r, r), (0.0, r));
    let opts = SimplifyOptions::default().tolerance(1.0).straight_lines(true);
    let simpler = simplify_with(&quarter, &opts);
    assert!(simpler.segments().count() > 1 && simpler.segments().all(|s| matches!(s, kurbo::PathSeg::Line(_))), "{simpler:?}");
    // Within the tolerance, but for the rounding of distances measured at coordinates of 500,000
    // (1.2e-10 each).
    let within = opts.tolerance + 1e-9;
    // The chords of the result, probed between their ends, against the curve.
    let off = deviation_of(&simpler, &quarter);
    assert!(off <= within, "the result is {off} from the curve");
    // And the curve against the result.
    let back = deviation_of(&quarter, &simpler);
    assert!(back <= within, "the curve is {back} from the result");
}

#[test]
fn a_run_needing_more_samples_than_the_limit_stays_with_its_first_sample_counted() {
    // Nearly a straight line of one cubic: four intervals and so five samples at this tolerance.
    let mut p = BezPath::new();
    p.move_to((0.0, 0.0));
    p.curve_to((3.0, 0.1), (6.0, -0.1), (9.0, 0.0));
    let opts = SimplifyOptions::default().tolerance(5.0);
    let at = |points: usize| with_limits(limits(|l| l.fit_points = points), || simplify_with(&p, &opts));
    assert_eq!(at(4), p, "four points are too few for five samples: the run stays");
    assert_ne!(at(5), p, "five are enough");
}
