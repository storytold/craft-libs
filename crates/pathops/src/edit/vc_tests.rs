// Carved in part from VectorCraft's crates/pathops (the tests of src/fit.rs and src/edit.rs, 199c4c3, 4a7d732 and bc19dd6) at storytold/vectorcraft 0945eaf2b3d4e93963aaa6d796b9efe53feaf624, on `BezPath`; the tests listed below as this crate's own are not VectorCraft's.
// Copyright (c) 2026 ArtCraft Team and the VectorCraft contributors. MIT OR Apache-2.0.

//! Tests of Simplify with an oracle that does not use the fit: each path is simplified, and each of the path and its result must be within the
//! tolerance of the other, measured on the curves themselves (no sample of the fit, no bound of its own: the nearest point on each
//! cubic of the other). Some are VectorCraft's, ported on `BezPath` with their names; the others are this crate's own.
//!
//! VectorCraft's (the same names in its `fit.rs` and `edit.rs` tests): `long_arcs_stay_within_the_tolerance`,
//! `long_arcs_and_short_anchors_stay_within_the_tolerance`, `long_arcs_converted_to_straight_lines_stay_within_the_tolerance`,
//! `long_and_short_segments_stay_within_the_tolerance`, `a_segment_that_needs_too_many_samples_stays_as_it_is`,
//! `a_segment_that_needs_too_many_samples_becomes_lines_when_asked_for_lines`,
//! `a_path_that_turns_back_past_its_end_converted_to_straight_lines_keeps_its_tip`, `no_fitted_piece_is_farther_than_the_tolerance`,
//! `a_zigzag_stays_within_the_tolerance`, `closed_paths_stay_within_the_tolerance`, `compound_paths_stay_within_the_tolerance`,
//! `coincident_anchors_stay_within_the_tolerance`, `random_paths_stay_within_the_tolerance`,
//! `smoothing_work_per_sample_stays_bounded_on_dense_paths`, `smoothing_points_whose_arc_length_overflows_returns_every_point` and
//! `grouped_smoothing_stays_within_a_thousandth_of_a_spread_of_weighing_one_by_one`; adapted from VectorCraft's
//! `lines_for_segments_that_need_too_many_samples_are_limited`: `lines_for_curves_too_fine_to_sample_are_held_to_the_result_limit` (the
//! limit here is the element limit over the whole call).
//!
//! This crate's own: `a_smoothed_loop_has_no_kink_at_its_start` (from the description of 199c4c3),
//! `a_smoothed_loop_is_fitted_along_the_sum_of_its_end_tangents`, `only_a_smoothed_loop_is_fitted_across_its_start`,
//! `a_corner_in_a_smoothed_loop_stays_a_corner`, `a_loop_of_long_arcs_with_a_small_turn_at_its_start_is_fitted_across_it`,
//! `a_long_straight_cubic_is_sampled_as_a_line_for_straight_lines`, `the_lines_of_a_fine_curve_are_charged_to_the_unit`,
//! `a_part_that_exactly_fills_the_samples_is_one_part`, `the_tree_of_sums_is_charged_by_its_samples_before_it_is_made`,
//! `what_is_made_is_held_to_the_element_limit_over_all_the_runs_of_a_path`,
//! `remove_redundant_points_is_held_to_the_oracle_on_the_inputs_simplify_is_tested_with`, and the tests of the fix round of the review
//! (`a_smoothed_loop_fitted_in_parts_leaves_each_joint_along_the_circle`, `big_smoothed_loops_fitted_in_parts_leave_each_joint_along_the_circle`,
//! `a_piece_that_stays_as_it_was_gives_back_what_its_flushed_parts_made` and `a_part_holds_no_more_samples_than_the_call_has_room_for`).

use kurbo::{BezPath, CubicBez, ParamCurve, ParamCurveExtrema, ParamCurveNearest, Point, Vec2};

use super::*;
use crate::fit::sample;
use crate::path::Seg;
use crate::testutil::anchor_count;

/// Numbers from 0 to 1, the same ones for the same `seed`.
fn random(mut seed: u64) -> impl FnMut() -> f64 {
    move || {
        seed = seed.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1_442_695_040_888_963_407);
        (seed >> 11) as f64 / (1u64 << 53) as f64
    }
}

/// An anchor: its point and the handles leaving it and arriving at it, as vectors from it.
#[derive(Clone, Copy)]
struct A {
    p: Point,
    out: Vec2,
    inn: Vec2,
}

impl A {
    fn smooth(p: Point, out: Vec2) -> A {
        A { p, out, inn: -out }
    }
    fn corner(p: Point) -> A {
        A { p, out: Vec2::ZERO, inn: Vec2::ZERO }
    }
}

/// The path through `anchors`, closed by a segment from the last back to the first when `closed`.
fn path_of(anchors: &[A], closed: bool) -> BezPath {
    let mut bp = BezPath::new();
    let Some(first) = anchors.first() else { return bp };
    bp.move_to(first.p);
    let seg = |bp: &mut BezPath, a: &A, b: &A| {
        if a.out == Vec2::ZERO && b.inn == Vec2::ZERO {
            bp.line_to(b.p);
        } else {
            bp.curve_to(a.p + a.out, b.p + b.inn, b.p);
        }
    };
    for w in anchors.windows(2) {
        seg(&mut bp, &w[0], &w[1]);
    }
    if closed {
        if let (Some(last), true) = (anchors.last(), anchors.len() > 1) {
            seg(&mut bp, last, first);
        }
        bp.close_path();
    }
    bp
}

fn polyline_path(pts: &[Point], closed: bool) -> BezPath {
    let anchors: Vec<A> = pts.iter().map(|&p| A::corner(p)).collect();
    path_of(&anchors, closed)
}

/// Largest distance from a point of `pts` to the nearest of the cubics `cs`, or infinity when a point is more than `reach` from the
/// bounding box of every cubic.
fn farthest(pts: &[Point], cs: &[CubicBez], reach: f64) -> f64 {
    let boxes: Vec<_> = cs.iter().map(|c| c.bounding_box().inflate(reach, reach)).collect();
    pts.iter()
        .map(|&p| {
            cs.iter().zip(&boxes).filter(|(_, b)| b.contains(p)).map(|(c, _)| c.nearest(p, 1e-9).distance_sq.sqrt()).fold(f64::INFINITY, f64::min)
        })
        .fold(0.0, f64::max)
}

/// The segments of `path` as cubics.
fn cubics(path: &BezPath) -> Vec<CubicBez> {
    path.segments().map(|s| Seg::straight(s).c).collect()
}

/// Samples of `cs` that follow each curve to within 1% of `tol`, at least 16 per curve.
fn close_samples(cs: &[CubicBez], tol: f64) -> Vec<Point> {
    let mut pts = Vec::new();
    for c in cs {
        let steps = (0.75 * second_difference(c) / (0.01 * tol)).sqrt().ceil().clamp(16.0, 20000.0);
        sample(c, steps as usize, &mut pts, true);
    }
    pts
}

/// Each of `before` and `after` is within `tol` of the other, measured on the curves themselves.
fn assert_within(before: &[CubicBez], after: &[CubicBez], tol: f64, name: &str) {
    let limit = tol * (1.0 + 1e-6);
    let to_result = farthest(&close_samples(before, tol), after, limit * 8.0);
    assert!(to_result <= limit, "{name}: a point of the path is {to_result} from the result at a tolerance of {tol}");
    let to_path = farthest(&close_samples(after, tol), before, limit * 8.0);
    assert!(to_path <= limit, "{name}: a point of the result is {to_path} from the path at a tolerance of {tol}");
}

/// Simplify of `path` has no more anchors, and each of the path and its result is within `tol` of the other. Returns the result.
fn assert_simplifies_within(path: &BezPath, tol: f64, name: &str) -> BezPath {
    let out = simplify_with(path, &SimplifyOptions::default().tolerance(tol));
    assert!(anchor_count(&out) <= anchor_count(path), "{name}: {} anchors from {}", anchor_count(&out), anchor_count(path));
    assert_within(&cubics(path), &cubics(&out), tol, name);
    out
}

/// The segments of `path` as a run for `simplify_run`.
fn run_of(path: &BezPath) -> Vec<Seg> {
    crate::path::subs(path).first().map(|s| s.chain()).unwrap_or_default()
}

fn fit_unlimited() -> Fit {
    Fit::new(usize::MAX, MAX_FIT_POINTS)
}

/// Anchors along a circle of radius `r` about the origin, from `first` degrees in steps of `step` degrees, smooth, with the handles of a
/// circular arc.
fn arc_anchors(r: f64, first: f64, step: f64, count: usize) -> Vec<A> {
    let handle = r * 4.0 / 3.0 * (step.to_radians() / 4.0).tan();
    (0..count)
        .map(|i| {
            let (s, c) = (first + i as f64 * step).to_radians().sin_cos();
            A::smooth(Point::new(r * c, r * s), Vec2::new(-s, c) * handle)
        })
        .collect()
}

/// `count` corner anchors 0.5 pt apart, going on from `from` along `dir`.
fn short_anchors(from: Point, dir: Vec2, count: usize) -> Vec<A> {
    (1..=count).map(|i| A::corner(from + dir * (i as f64 * 0.5))).collect()
}

#[test]
fn long_arcs_stay_within_the_tolerance() {
    // Simplify keeps a subpath as it was when the refit has more anchors, so this measures the refit itself.
    for (r, tol) in [(1_000_000.0, 2.0), (1_000_000.0, 1.0), (100_000.0, 0.1), (100_000.0, 1.0 / 32.0), (20_000.0, 0.01), (5_000.0, 0.01)] {
        let run = run_of(&path_of(&arc_anchors(r, 0.0, 90.0, 4), false));
        let opts = SimplifyOptions::default().tolerance(tol);
        let out = simplify_run(&run, &opts, &mut fit_unlimited(), None);
        let (before, after): (Vec<_>, Vec<_>) = (run.iter().map(|s| s.c).collect(), out.iter().map(|s| s.c).collect());
        assert_within(&before, &after, tol, &format!("arcs of {r} pt"));
        assert!(after.len() < 200, "arcs of {r} pt at {tol} pt give {} curves", after.len());
    }
}

#[test]
fn long_arcs_and_short_anchors_stay_within_the_tolerance() {
    for (r, tol) in [(100_000.0, 1.0), (100_000.0, 0.1), (100_000.0, 1.0 / 32.0), (20_000.0, 0.01), (5_000.0, 0.01), (1_000_000.0, 1.0)] {
        let mut anchors = arc_anchors(r, 0.0, 90.0, 4);
        let end = anchors[3].p;
        anchors.extend(short_anchors(end, Vec2::new(1.0, 0.0), 3000));
        let out = assert_simplifies_within(&path_of(&anchors, false), tol, &format!("arcs of {r} pt and short anchors"));
        assert!(anchor_count(&out) < 200, "arcs of {r} pt at {tol} pt give {} anchors", anchor_count(&out));
    }
}

#[test]
fn long_arcs_converted_to_straight_lines_stay_within_the_tolerance() {
    // (VectorCraft's second case, a radius of 20,000 at 0.01, takes the check 90 s in a debug build: the first is the same shape of input.)
    {
        let (r, tol) = (100_000.0, 0.1);
        let mut anchors = arc_anchors(r, 0.0, 90.0, 4);
        let end = anchors[3].p;
        anchors.extend(short_anchors(end, Vec2::new(1.0, 0.0), 3000));
        let path = path_of(&anchors, false);
        let out = simplify_with(&path, &SimplifyOptions::default().tolerance(tol).straight_lines(true));
        assert_within(&cubics(&path), &cubics(&out), tol, &format!("lines along arcs of {r} pt"));
    }
}

#[test]
fn long_and_short_segments_stay_within_the_tolerance() {
    for (r, tol) in [(50_000.0, 2.0), (50_000.0, 1.0 / 32.0), (8_000.0, 0.01), (500.0, 0.5)] {
        let mut anchors = Vec::new();
        for block in 0..6 {
            let first = f64::from(block) * 60.0;
            let arc = arc_anchors(r, first, 20.0, 3);
            let (s, c) = (first + 40.0).to_radians().sin_cos();
            let end = arc[2].p;
            anchors.extend(arc);
            anchors.extend(short_anchors(end, Vec2::new(-s, c), 300));
        }
        assert_simplifies_within(&path_of(&anchors, false), tol, &format!("radius {r}"));
    }
}

#[test]
fn a_segment_that_needs_too_many_samples_stays_as_it_is() {
    // Radius 10,000,000 pt at 0.001 pt needs about 770,000 steps per quarter circle.
    let mut anchors = arc_anchors(10_000_000.0, 0.0, 90.0, 4);
    let end = anchors[3].p;
    anchors.extend(short_anchors(end, Vec2::new(1.0, 0.0), 500));
    let path = path_of(&anchors, false);
    let out = simplify_with(&path, &SimplifyOptions::default().tolerance(0.001));
    assert_eq!(cubics(&out)[..3], cubics(&path)[..3]);
    assert!(anchor_count(&out) < 20, "{} anchors", anchor_count(&out));
}

#[test]
fn a_segment_that_needs_too_many_samples_becomes_lines_when_asked_for_lines() {
    // About 55,000 lines keep this arc within 0.001 pt, and about 770,000 samples would be needed to fit it.
    let (r, tol) = (10_000_000.0, 0.001);
    let path = path_of(&arc_anchors(r, 0.0, 90.0, 2), false);
    let arc = cubics(&path)[0];
    let out = cubics(&simplify_with(&path, &SimplifyOptions::default().tolerance(tol).straight_lines(true)));
    assert!(out.iter().all(|c| crate::fit::is_straight(c, 1e-6)));
    for (k, line) in out.iter().enumerate() {
        let middle = arc.eval((k as f64 + 0.5) / out.len() as f64);
        assert!(middle.distance(line.p0.midpoint(line.p3)) <= tol, "line {k} of {}", out.len());
    }
}

#[test]
fn a_path_that_turns_back_past_its_end_converted_to_straight_lines_keeps_its_tip() {
    // A line to (100, 0), a smooth 0.2 pt U-turn and a line back to (50, 0.2): every point is within 0.4 pt of the line through the
    // ends, but the tip is 50 pt past the end (VectorCraft bc19dd6).
    let k = 0.2 * 2.0 / 3.0;
    let (a, b) = (Point::new(100.0, 0.0), Point::new(100.0, 0.2));
    let mut path = BezPath::new();
    path.move_to((0.0, 0.0));
    path.line_to(a);
    path.curve_to(Point::new(100.0 + k, 0.0), Point::new(100.0 + k, 0.2), b);
    path.line_to((50.0, 0.2));
    let out = simplify_with(&path, &SimplifyOptions::default().tolerance(1.0).straight_lines(true));
    assert_within(&cubics(&path), &cubics(&out), 1.0, "a U-turn past the end");
}

/// `rows` rows of `cols` smooth anchors 1.6 pt apart, alternately 2 pt up.
fn serpentine(rows: usize, cols: usize, closed: bool) -> BezPath {
    let mut anchors = Vec::with_capacity(rows * cols);
    for row in 0..rows {
        let dir = if row % 2 == 0 { 1.0 } else { -1.0 };
        for col in 0..cols {
            let x = if row % 2 == 0 { col } else { cols - 1 - col };
            let p = Point::new(x as f64 * 1.6, row as f64 * 6.0 + if col % 2 == 0 { 0.0 } else { 2.0 });
            anchors.push(A::smooth(p, Vec2::new(0.5 * dir, 0.0)));
        }
    }
    path_of(&anchors, closed)
}

/// `n` straight segments of `step` pt that turn by `degrees` left and right in turn.
fn zigzag(n: usize, degrees: f64, step: f64, closed: bool) -> BezPath {
    let half = (degrees / 2.0).to_radians();
    let mut p = Point::ZERO;
    let mut pts = Vec::with_capacity(n);
    for i in 0..n {
        pts.push(p);
        let a = if i % 2 == 0 { half } else { -half };
        p += Vec2::new(a.cos(), a.sin()) * step;
    }
    polyline_path(&pts, closed)
}

/// Smooth anchors along arcs that bend one way and then the other, 40 anchors per arc.
fn alternating_arcs(arcs: usize) -> BezPath {
    let mut anchors = Vec::with_capacity(arcs * 40);
    let (mut p, mut heading) = (Point::ZERO, 0.0_f64);
    for arc in 0..arcs {
        for _ in 0..40 {
            heading += if arc % 2 == 0 { 0.08 } else { -0.08 };
            let d = Vec2::new(heading.cos(), heading.sin()) * 2.0;
            anchors.push(A::smooth(p, d / 3.0));
            p += d;
        }
    }
    path_of(&anchors, false)
}

/// A polyline of random steps, with some points repeated when `repeats` is set.
fn random_walk(rnd: &mut impl FnMut() -> f64, n: usize, repeats: bool, closed: bool) -> BezPath {
    let mut p = Point::new(rnd() * 100.0, rnd() * 100.0);
    let mut pts = Vec::with_capacity(n);
    for _ in 0..n {
        pts.push(p);
        if repeats && rnd() < 0.3 {
            pts.push(p);
        }
        p += Vec2::new(rnd() * 6.0 - 3.0, rnd() * 6.0 - 3.0);
    }
    polyline_path(&pts, closed)
}

/// A chain of smooth anchors with random steps and handles.
fn random_curves(rnd: &mut impl FnMut() -> f64, n: usize, closed: bool) -> BezPath {
    let mut p = Point::ZERO;
    let mut anchors = Vec::with_capacity(n);
    for _ in 0..n {
        anchors.push(A::smooth(p, Vec2::new(1.0 + rnd() * 6.0, rnd() * 8.0 - 4.0)));
        p += Vec2::new(5.0 + rnd() * 20.0, rnd() * 30.0 - 15.0);
    }
    path_of(&anchors, closed)
}

/// `n` smooth anchors 1.6 pt apart, each at a random height of up to 3 pt.
fn random_ripple(rnd: &mut impl FnMut() -> f64, n: usize, closed: bool) -> BezPath {
    let anchors: Vec<A> = (0..n).map(|i| A::smooth(Point::new(i as f64 * 1.6, rnd() * 3.0), Vec2::new(0.5, 0.0))).collect();
    path_of(&anchors, closed)
}

#[test]
fn no_fitted_piece_is_farther_than_the_tolerance() {
    for (rows, cols) in [(1, 300), (1, 1000), (5, 400)] {
        assert_simplifies_within(&serpentine(rows, cols, false), 1.0, &format!("serpentine {rows} x {cols}"));
    }
    assert_simplifies_within(&serpentine(1, 1000, false), 0.25, "serpentine at 0.25 pt");
    assert_simplifies_within(&alternating_arcs(25), 0.5, "alternating arcs");
}

#[test]
fn a_zigzag_stays_within_the_tolerance() {
    // The turns are below the 30 degree corner threshold, so the fit runs across them.
    assert_simplifies_within(&zigzag(1500, 20.0, 3.0, false), 0.5, "zigzag");
    assert_simplifies_within(&zigzag(1500, 29.0, 1.0, false), 0.1, "fine zigzag");
    // These turns are corners, so every segment is a run of its own.
    assert_simplifies_within(&zigzag(500, 60.0, 3.0, false), 2.0, "zigzag of corners");
    // Simplify samples a segment this long more than `tol` apart, so a curve between two samples can leave the path without a sample
    // showing it.
    assert_simplifies_within(&zigzag(60, 25.0, 900.0, false), 1.0, "zigzag of long segments");
    assert_simplifies_within(&zigzag(60, 8.0, 2500.0, true), 0.5, "closed zigzag of long segments");
    // The short anchors make the result smaller than the path, so Simplify does not return the path as it was.
    for (step, tol) in [(1000.0, 0.1), (5000.0, 1.0), (20000.0, 1.0)] {
        let mut path = zigzag(40, 25.0, step, false);
        let end = crate::path::subs(&path).first().map_or(Point::ZERO, |s| s.end());
        for i in 1..=2000 {
            path.line_to(end + Vec2::new(f64::from(i) * 0.5, 0.0));
        }
        assert_simplifies_within(&path, tol, &format!("zigzag of {step} pt segments and short anchors"));
    }
}

#[test]
fn closed_paths_stay_within_the_tolerance() {
    assert_simplifies_within(&serpentine(3, 1000, true), 1.0, "closed serpentine");
    assert_simplifies_within(&zigzag(800, 20.0, 3.0, true), 0.5, "closed zigzag");
    let circle: Vec<A> = (0..2000)
        .map(|i| {
            let a = f64::from(i) / 2000.0 * std::f64::consts::TAU;
            A::smooth(Point::new(300.0 * a.cos(), 300.0 * a.sin()), Vec2::new(-a.sin(), a.cos()) * 0.3)
        })
        .collect();
    assert_simplifies_within(&path_of(&circle, true), 0.05, "closed circle of 2,000 anchors");
}

#[test]
fn compound_paths_stay_within_the_tolerance() {
    let mut rnd = random(11);
    let mut compound = serpentine(1, 1000, false);
    compound.extend(zigzag(400, 20.0, 3.0, true).iter());
    compound.extend(random_curves(&mut rnd, 100, false).iter());
    compound.extend(polyline_path(&[Point::new(0.0, -50.0), Point::new(100.0, -50.0)], false).iter());
    assert_simplifies_within(&compound, 0.75, "compound");
}

#[test]
fn coincident_anchors_stay_within_the_tolerance() {
    let mut rnd = random(23);
    assert_simplifies_within(&random_walk(&mut rnd, 600, true, false), 1.0, "repeated points");
    assert_simplifies_within(&random_walk(&mut rnd, 600, true, true), 1.0, "repeated points, closed");
    let p = Point::new(5.0, 5.0);
    assert_simplifies_within(&path_of(&[A::smooth(p, Vec2::new(1.0, 0.0)); 50], false), 0.5, "one place");
    let mut anchors = Vec::new();
    for row in 0..1500usize {
        let p = Point::new(row as f64 * 1.6, if row % 2 == 0 { 0.0 } else { 2.0 });
        anchors.extend([A::smooth(p, Vec2::new(0.5, 0.0)); 2]);
    }
    assert_simplifies_within(&path_of(&anchors, false), 1.0, "every anchor twice");
}

#[test]
fn random_paths_stay_within_the_tolerance() {
    let mut rnd = random(0x9e37_79b9_7f4a_7c15);
    for case in 0..60 {
        let tol = [0.05, 0.25, 1.0, 4.0][case % 4];
        let n = 2 + (rnd() * 300.0) as usize;
        let closed = case % 3 == 0;
        let path = if case % 2 == 0 { random_walk(&mut rnd, n, false, closed) } else { random_curves(&mut rnd, n, closed) };
        assert_simplifies_within(&path, tol, &format!("case {case}"));
    }
    for (case, tol) in [0.5, 1.0, 2.0].into_iter().enumerate() {
        assert_simplifies_within(&random_ripple(&mut rnd, 2000, case == 1), tol, &format!("ripple at {tol} pt"));
    }
}

/// With Convert to Straight Lines, a curve too fine to sample becomes as many equal lines as keep it within the tolerance, up to a
/// segment's limit, and the call holds [`Limits::elements`](crate::Limits) segments of result in all (charged before they are made): past that the piece stays as
/// it is, never lines that pass the tolerance (VectorCraft makes them, as many as it allows).
#[test]
fn lines_for_curves_too_fine_to_sample_are_held_to_the_result_limit() {
    let (r, tol) = (10_000_000.0, 0.001);
    let opts = SimplifyOptions::default().tolerance(tol).straight_lines(true);
    let one = path_of(&arc_anchors(r, 0.0, 90.0, 2), false);
    let three = path_of(&arc_anchors(r, 0.0, 90.0, 4), false);
    let with_elements = |elements: usize| crate::Limits { elements, ..crate::Limits::default() };
    // Room for the lines of one arc (about 55,000) and not of three: the three stay as they are, the one is lines.
    let tight = with_elements(100_000);
    let made = crate::with_limits(tight, || simplify_with(&three, &opts));
    assert_eq!(made, three, "three arcs of 55,000 lines are over 100,000 elements: the piece stays");
    let made = crate::with_limits(tight, || simplify_with(&one, &opts));
    assert!(made.elements().len() > 50_000 && made.elements().len() <= 100_000 && made.segments().all(|s| matches!(s, kurbo::PathSeg::Line(_))));
    // With room for them all they are lines, within the tolerance of the arcs.
    let roomy = with_elements(300_000);
    let made = crate::with_limits(roomy, || simplify_with(&three, &opts));
    assert!(made.elements().len() > 150_000 && made.elements().len() <= 300_000, "{}", made.elements().len());
    // The lines are charged to the work before they are made: with none left the piece stays.
    let mut starved = Fit::new(1_000, MAX_FIT_POINTS);
    let run = run_of(&one);
    assert!(!simplify_piece(&run, (tol, None), &opts, &mut starved, &mut Vec::new()), "no work for 55,000 lines");
}

/// A smoothed closed path with no corner is fitted along one direction across its start, not along the tangents of its first and
/// last curves, so the start has no kink (VectorCraft 199c4c3).
#[test]
fn a_smoothed_loop_has_no_kink_at_its_start() {
    let pts: Vec<Point> = (0..600)
        .map(|i| {
            let a = f64::from(i) / 600.0 * std::f64::consts::TAU;
            let r = 100.0 + 3.0 * (7.0 * a).sin();
            Point::new(r * a.cos(), r * a.sin())
        })
        .collect();
    let loop_path = polyline_path(&pts, true);
    let opts = SimplifyOptions::default().tolerance(1.0).smooth(20.0);
    let out = simplify_with(&loop_path, &opts);
    let cs = cubics(&out);
    assert!(cs.len() >= 4 && cs.len() < 100, "{} curves", cs.len());
    let (first, last) = (cs.first().unwrap(), cs.last().unwrap());
    assert!(last.p3.distance(first.p0) < 1e-9, "closed");
    let kink = crate::fit::end_tangent(last).dot(crate::fit::start_tangent(first));
    assert!(kink > 0.999_999, "the tangents arriving at and leaving the start are {kink} apart");
    // A path that is not smoothed keeps the tangents of its own curves, and a loop with a corner is not turned across it.
    let plain = simplify_with(&loop_path, &SimplifyOptions::default().tolerance(1.0));
    assert!(!cubics(&plain).is_empty());
}

/// `smooth_points` weighing every sample on its own (VectorCraft's `smooth_points_one_by_one`).
fn smooth_points_one_by_one(pts: &[Point], sigma: f64) -> Vec<Point> {
    let mut s = vec![0.0];
    for w in pts.windows(2) {
        s.push(s.last().copied().unwrap_or(0.0) + w[0].distance(w[1]));
    }
    let total = s.last().copied().unwrap_or(0.0);
    pts.iter()
        .zip(&s)
        .map(|(&p, &si)| {
            let reach = (3.0 * sigma).min(si).min(total - si);
            if reach <= 1e-9 {
                return p;
            }
            let spread = reach / 3.0;
            let (mut sum, mut weight) = (Vec2::ZERO, 0.0);
            for (q, &sj) in pts.iter().zip(&s).filter(|(_, sj)| (*sj - si).abs() <= reach) {
                let w = (-(sj - si) * (sj - si) / (2.0 * spread * spread)).exp();
                sum += (*q - p) * w;
                weight += w;
            }
            p + sum / weight
        })
        .collect()
}

/// A traced outline: `n` straight segments around a circle of radius 200, each anchor moved in or out by up to 0.5 pt.
fn traced_circle(n: usize) -> BezPath {
    let pts: Vec<Point> = (0..n)
        .map(|i| {
            let a = i as f64 / n as f64 * std::f64::consts::TAU;
            let r = 200.0 + 0.5 * ((i * 7919) % 13) as f64 / 13.0;
            Point::new(300.0 + r * a.cos(), 300.0 + r * a.sin())
        })
        .collect();
    polyline_path(&pts, true)
}

/// The samples Simplify takes of `path` at a tolerance of 0.5 (the segments sampled in the intervals [`intervals`] gives).
fn samples_of(path: &BezPath, opts: &SimplifyOptions) -> Vec<Point> {
    let mut pts = Vec::new();
    for (k, s) in run_of(path).iter().enumerate() {
        let Sampling::In(m) = intervals(s, opts.tolerance, opts) else { panic!("a segment that cannot be sampled") };
        sample(&s.c, m, &mut pts, k == 0);
    }
    pts
}

/// What a smoothing of `pts` is charged: the work spent, which is what is counted.
fn smoothing_work(pts: &[Point], sigma: f64) -> usize {
    let mut fit = Fit::new(usize::MAX, MAX_FIT_POINTS);
    assert_eq!(smooth_points(pts, sigma, &mut fit).map(|v| v.len()), Some(pts.len()));
    usize::MAX - fit.work
}

#[test]
fn smoothing_work_per_sample_stays_bounded_on_dense_paths() {
    // Weighing every sample within three spreads took 360 steps per sample for 1,000 anchors and 2,949 for 16,000 in VectorCraft: the work
    // grew with the square of the anchor count. Here the same traced outline at 1,000 to 64,000 anchors is held to a few hundred.
    let opts = SimplifyOptions::default().tolerance(0.5);
    let mut last = None;
    for n in [1_000usize, 4_000, 16_000, 64_000] {
        let pts = samples_of(&traced_circle(n), &opts);
        let work = smoothing_work(&pts, 100.0);
        assert!(work <= 400 * pts.len(), "{n} anchors: {work} steps for {} samples", pts.len());
        if let Some((last_n, per_sample)) = last {
            // Four times the anchors, the same work for each sample (or less: the samples of a segment are 4 at least).
            assert!(
                work as f64 / pts.len() as f64 <= 1.3 * per_sample,
                "{last_n} anchors {per_sample} a sample, {n} anchors {}",
                work as f64 / pts.len() as f64
            );
        }
        last = Some((n, work as f64 / pts.len() as f64));
    }
    // A curve sampled at a tiny tolerance and smoothed far beyond its length.
    let c = CubicBez::new((0.0, 0.0), (0.0, 100.0), (100.0, 100.0), (100.0, 0.0));
    let mut pts = Vec::new();
    sample(&c, 100_000, &mut pts, true);
    let work = smoothing_work(&pts, 1000.0);
    assert!(work <= 400 * pts.len(), "{work} steps for {} samples", pts.len());
}

#[test]
fn smoothing_points_whose_arc_length_overflows_returns_every_point() {
    let pts: Vec<Point> = (0..40).map(|i| Point::new(if i % 2 == 0 { -1e308 } else { 1e308 }, f64::from(i))).collect();
    let mut fit = Fit::new(usize::MAX, MAX_FIT_POINTS);
    assert_eq!(smooth_points(&pts, 10.0, &mut fit).map(|v| v.len()), Some(pts.len()));
    // However the arc lengths came out, the work is charged as it is made: what it is spent on is what the budget holds.
    let mut little = Fit::new(100, MAX_FIT_POINTS);
    assert_eq!(smooth_points(&pts, 10.0, &mut little), None);
    assert_eq!(little.work, 0);
}

#[test]
fn grouped_smoothing_stays_within_a_thousandth_of_a_spread_of_weighing_one_by_one() {
    let mut seed = 7u64;
    let mut next = || {
        seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        (seed >> 11) as f64 / (1u64 << 53) as f64
    };
    for case in 0..300 {
        let n = 3 + (next() * 2000.0) as usize;
        let mut p = Point::ZERO;
        let mut dir: f64 = 0.0;
        let dense = next() < 0.5;
        let pts: Vec<Point> = (0..n)
            .map(|_| {
                dir += (next() - 0.5) * 1.5;
                let step = if dense { next() * 0.05 } else { next() * 5.0 };
                p += Vec2::new(dir.cos(), dir.sin()) * step;
                p
            })
            .collect();
        let sigma = 0.01 + next() * 50.0;
        let exact = smooth_points_one_by_one(&pts, sigma);
        let grouped = smooth_points(&pts, sigma, &mut Fit::new(usize::MAX, MAX_FIT_POINTS)).unwrap();
        assert_eq!(grouped.len(), exact.len());
        for (i, (a, b)) in exact.iter().zip(&grouped).enumerate() {
            assert!(a.distance(*b) <= 1e-3 * sigma, "case {case}, point {i}: {a:?} and {b:?}, spread {sigma}");
        }
        assert_eq!((grouped.first(), grouped.last()), (pts.first(), pts.last()), "case {case}");
    }
}

/// The tree of sums is made as the samples are charged and does not outgrow them: with no work for it, no smoothing.
#[test]
fn the_tree_of_sums_is_charged_by_its_samples_before_it_is_made() {
    let pts: Vec<Point> = (0..500).map(|i| Point::new(f64::from(i), (f64::from(i) * 0.1).sin())).collect();
    // The charge fails and takes what is left (as every charge to a fit does): no tree, no points.
    let mut fit = Fit::new(499, MAX_FIT_POINTS);
    assert_eq!(smooth_points(&pts, 5.0, &mut fit), None);
    assert_eq!(fit.work, 0);
    // The tree is paid for, and no work is left for the points.
    let mut fit = Fit::new(500, MAX_FIT_POINTS);
    assert_eq!(smooth_points(&pts, 5.0, &mut fit), None);
    assert_eq!(fit.work, 0);
    // With enough for the nodes it looks at, every point is made.
    let mut fit = Fit::new(500 + 500 * 200, MAX_FIT_POINTS);
    assert_eq!(smooth_points(&pts, 5.0, &mut fit).map(|v| v.len()), Some(500));
}

/// The refit of a run from its own points, with the lens allowance (its lengths added with the rounding compensated) and the witness
/// points, that this crate's `remove_redundant_points` makes (VectorCraft's does not, and neither commit of the port touches it) is held to the
/// same oracle as Simplify, on the inputs VectorCraft tests Simplify with: each of the path and its result within the tolerance of the
/// other, measured on the curves themselves. Both refits are held on the one set of inputs.
#[test]
fn remove_redundant_points_is_held_to_the_oracle_on_the_inputs_simplify_is_tested_with() {
    let mut anchors = arc_anchors(100_000.0, 0.0, 90.0, 4);
    let end = anchors[3].p;
    anchors.extend(short_anchors(end, Vec2::new(1.0, 0.0), 3000));
    let circle: Vec<A> = (0..2000)
        .map(|i| {
            let a = f64::from(i) / 2000.0 * std::f64::consts::TAU;
            A::smooth(Point::new(300.0 * a.cos(), 300.0 * a.sin()), Vec2::new(-a.sin(), a.cos()) * 0.3)
        })
        .collect();
    let mut rnd = random(31);
    let cases: Vec<(&str, BezPath, f64)> = vec![
        ("serpentine", serpentine(1, 300, false), 1.0),
        ("serpentine of five rows", serpentine(5, 200, false), 1.0),
        ("alternating arcs", alternating_arcs(25), 0.5),
        ("zigzag", zigzag(500, 20.0, 3.0, false), 0.5),
        ("zigzag of long segments", zigzag(60, 25.0, 900.0, false), 1.0),
        ("long arcs and short anchors", path_of(&anchors, false), 1.0),
        ("closed circle of 2,000 anchors", path_of(&circle, true), 0.05),
        ("random curves", random_curves(&mut rnd, 200, false), 0.25),
        ("ripple", random_ripple(&mut rnd, 1000, false), 1.0),
    ];
    for (name, path, tol) in cases {
        let out = remove_redundant_points(&path, tol);
        assert!(anchor_count(&out) <= anchor_count(&path), "{name}");
        assert_within(&cubics(&path), &cubics(&out), tol, &format!("remove_redundant_points of {name}"));
        // And Simplify, the same path, the same oracle: neither is held to less.
        assert_simplifies_within(&path, tol, name);
    }
}

/// What a simplify makes is held to [`crate::Limits::elements`] over the whole call, not for each run of segments: many subpaths each
/// under the limit made as many segments as they were many. Each is charged, lines as they are made and a refit by the samples it is made of,
/// before it is made, to what the call has left; a piece the charge refuses stays as it was (the one choice that keeps the result within the
/// tolerance of the path, and what was made of the pieces before it is kept). Counted at 2, 4, 8 and 16 subpaths, each under the limit and
/// two of them over it.
#[test]
fn what_is_made_is_held_to_the_element_limit_over_all_the_runs_of_a_path() {
    // Lines of a curve too fine to sample: each of these arcs is about 55,000 lines, and the limit has room for one.
    let (r, tol) = (10_000_000.0, 0.001);
    let opts = SimplifyOptions::default().tolerance(tol).straight_lines(true);
    let arc = path_of(&arc_anchors(r, 0.0, 90.0, 2), false);
    let limits = crate::Limits { elements: 100_000, ..crate::Limits::default() };
    for n in [2usize, 4, 8, 16] {
        let mut paths = BezPath::new();
        for _ in 0..n {
            paths.extend(arc.iter());
        }
        let mut fit = Fit::new(usize::MAX, MAX_FIT_POINTS);
        let out = crate::with_limits(limits, || simplify_within(&paths, &opts, &mut fit));
        let lines = out.segments().filter(|s| matches!(s, kurbo::PathSeg::Line(_))).count();
        let curves = out.segments().filter(|s| matches!(s, kurbo::PathSeg::Cubic(_))).count();
        assert!(fit.made <= limits.elements, "{n} subpaths: {} segments made", fit.made);
        assert_eq!(lines, fit.made, "{n} subpaths: what was made is what is lines");
        assert!(lines > 50_000 && lines <= limits.elements, "{n} subpaths: the first is lines, {lines} of them");
        assert_eq!(curves, n - 1, "{n} subpaths: the others stay as the arc they were");
        let (before, after) = (cubics(&paths), cubics(&out));
        // The lines are within the tolerance of the first arc, and the others are the arc itself.
        for (k, c) in after.iter().filter(|c| !crate::fit::is_straight(c, 1e-6)).enumerate() {
            assert_eq!(*c, before[0], "{n} subpaths: arc {k}");
        }
    }
    // Refits: subpaths of a zigzag whose samples a straight-line conversion keeps (each about as many segments as it has),
    // the limit at one and a half of one: the call makes no more than the limit in all, whatever the number of subpaths.
    let opts = SimplifyOptions::default().tolerance(0.5).straight_lines(true);
    let one = zigzag(600, 25.0, 3.0, false);
    let single = {
        let mut fit = Fit::new(usize::MAX, MAX_FIT_POINTS);
        simplify_within(&one, &opts, &mut fit);
        fit.made
    };
    assert!(single > 300, "{single} segments made of one");
    let limits = crate::Limits { elements: single + single / 2, ..crate::Limits::default() };
    for n in [2usize, 4, 8, 16] {
        let mut paths = BezPath::new();
        for _ in 0..n {
            paths.extend(one.iter());
        }
        let mut fit = Fit::new(usize::MAX, MAX_FIT_POINTS);
        let out = crate::with_limits(limits, || simplify_within(&paths, &opts, &mut fit));
        assert!(fit.made <= limits.elements, "{n} subpaths: {} segments made for a limit of {}", fit.made, limits.elements);
        assert!(fit.made >= single, "{n} subpaths: the first is made ({} of {single})", fit.made);
        // Made or as it was, the result is within the tolerance of the path both ways.
        if n <= 4 {
            assert_within(&cubics(&paths), &cubics(&out), 0.5, &format!("{n} subpaths"));
        }
    }
}

/// A closed polygon of `n` vertices round a circle of radius 100 whose radius wobbles by 3 seven times round, from angle 0: smooth, so
/// with no corner in it.
fn wobbly_loop(n: usize) -> BezPath {
    let pts: Vec<Point> = (0..n)
        .map(|i| {
            let a = i as f64 / n as f64 * std::f64::consts::TAU;
            let r = 100.0 + 3.0 * (7.0 * a).sin();
            Point::new(r * a.cos(), r * a.sin())
        })
        .collect();
    polyline_path(&pts, true)
}

/// The direction the first curve of `path` leaves its start along, and the last arrives at it along (unit vectors).
fn ends_of(path: &BezPath) -> (Vec2, Vec2) {
    let cs = cubics(path);
    let (first, last) = (cs.first().expect("a curve"), cs.last().expect("a curve"));
    (crate::fit::start_tangent(first), crate::fit::end_tangent(last))
}

/// The tangents of the first and last segments of a subpath of `path`, as Simplify reads them: the direction the first leaves along and the
/// last arrives along.
fn own_ends(path: &BezPath) -> (Vec2, Vec2) {
    let segs = crate::path::subs(path)[0].chain();
    (crate::fit::start_tangent(&segs[0].c), crate::fit::end_tangent(&segs[segs.len() - 1].c))
}

/// A smoothed loop with no corner is fitted along one direction across its start: the sum of the tangents of its first and last segments,
/// normalised, which it leaves along and arrives along (the first and last segments of a polygon are two directions, and the sum is neither).
#[test]
fn a_smoothed_loop_is_fitted_along_the_sum_of_its_end_tangents() {
    let loop_path = wobbly_loop(600);
    let (first, last) = own_ends(&loop_path);
    assert!(first.dot(last) < 0.999_99, "the polygon's own tangents differ at its start: {}", first.dot(last));
    let across = crate::fit::unit(first + last).expect("a direction");
    let out = simplify_with(&loop_path, &SimplifyOptions::default().tolerance(1.0).smooth(20.0));
    let (leaves, arrives) = ends_of(&out);
    assert!(leaves.dot(across) > 0.999_999_9 && arrives.dot(across) > 0.999_999_9, "across {across:?}; leaves {leaves:?}, arrives {arrives:?}");
    assert!(cubics(&out).len() < 100, "fitted");
}

/// Without smoothing, a loop keeps the tangents of its own first and last segments, and so does an open path that is smoothed: the direction
/// across the start is for a smoothed loop only.
#[test]
fn only_a_smoothed_loop_is_fitted_across_its_start() {
    let loop_path = wobbly_loop(600);
    let (first, last) = own_ends(&loop_path);
    let plain = simplify_with(&loop_path, &SimplifyOptions::default().tolerance(0.5));
    let (leaves, arrives) = ends_of(&plain);
    assert!(cubics(&plain).len() < 200, "fitted");
    assert!(leaves.dot(first) > 0.999_999_9 && arrives.dot(last) > 0.999_999_9, "unsmoothed: its own tangents, {leaves:?} and {arrives:?}");
    // An open path with no corner in it, smoothed: its own tangents at its two ends.
    let mut open = BezPath::new();
    open.move_to((0.0, 0.0));
    for i in 1..400 {
        let x = f64::from(i) * 2.0;
        open.line_to((x, 20.0 * (x / 30.0).sin()));
    }
    let (first, last) = own_ends(&open);
    let out = simplify_with(&open, &SimplifyOptions::default().tolerance(0.5).smooth(10.0));
    let (leaves, arrives) = ends_of(&out);
    assert!(cubics(&out).len() < 200, "fitted");
    assert!(leaves.dot(first) > 0.999_999_9 && arrives.dot(last) > 0.999_999_9, "an open path keeps its own tangents: {leaves:?} and {arrives:?}");
}

/// A loop with one corner is not turned across it: the corner stays (the tangents arriving and leaving there are far apart).
#[test]
fn a_corner_in_a_smoothed_loop_stays_a_corner() {
    // A teardrop: x = cos t, y = sin t * sin(t / 2)^2, with its one sharp point at t = 0.
    let pts: Vec<Point> = (0..400)
        .map(|i| {
            let t = f64::from(i) / 400.0 * std::f64::consts::TAU;
            Point::new(100.0 * t.cos(), 100.0 * t.sin() * (t / 2.0).sin().powi(2))
        })
        .collect();
    let out = simplify_with(&polyline_path(&pts, true), &SimplifyOptions::default().tolerance(0.5).smooth(5.0));
    let (leaves, arrives) = ends_of(&out);
    assert!(arrives.dot(leaves) < 0.8, "the point of the teardrop is still a corner: {arrives:?} then {leaves:?}");
}

/// A long cubic that is straight to within a twentieth of the tolerance is sampled as a line is, in 256 intervals at most, and not in as many
/// as its handles' second difference would ask of a curve (thousands): its chords are on it.
#[test]
fn a_long_straight_cubic_is_sampled_as_a_line_for_straight_lines() {
    let c = CubicBez::new((0.0, 0.0), (3e5, 0.001), (6e5, -0.001), (1e6, 0.0));
    let mut path = BezPath::new();
    path.move_to(c.p0);
    path.curve_to(c.p1, c.p2, c.p3);
    crate::work::reset();
    let out = simplify_with(&path, &SimplifyOptions::default().tolerance(1.0).straight_lines(true));
    let samples = crate::work::take(crate::work::Work::FitSamples);
    assert!(samples <= MAX_INTERVALS + 1, "{samples} samples");
    assert!(out.segments().all(|s| matches!(s, kurbo::PathSeg::Line(_))));
}

/// The lines of a curve too fine to sample are charged to the elements the call may make, the work, and the lines a segment holds, to the
/// unit: exactly the room is enough, one under it is not; and the same for a segment's most (65,536) lines.
#[test]
fn the_lines_of_a_fine_curve_are_charged_to_the_unit() {
    let (r, tol) = (10_000_000.0, 0.001);
    let opts = SimplifyOptions::default().tolerance(tol).straight_lines(true);
    let arc = path_of(&arc_anchors(r, 0.0, 90.0, 2), false);
    let c = cubics(&arc)[0];
    let lines = line_steps(&c, tol);
    assert!(lines > 50_000 && lines < MAX_CHORD_INTERVALS, "{lines} lines");
    let made = |elements: usize| {
        let limits = crate::Limits { elements, ..crate::Limits::default() };
        let out = crate::with_limits(limits, || simplify_with(&arc, &opts));
        out.segments().filter(|s| matches!(s, kurbo::PathSeg::Line(_))).count()
    };
    // The elements: room for exactly the lines is enough, one fewer is not (the arc stays).
    assert_eq!(made(lines), lines);
    assert_eq!(made(lines - 1), 0);
    // The work: the lines and one more (the samples are one more than the intervals).
    let run = run_of(&arc);
    let mut enough = Fit::new(lines + 1, MAX_FIT_POINTS);
    let mut result = Vec::new();
    crate::work::reset();
    assert!(simplify_piece(&run, (tol, None), &opts, &mut enough, &mut result));
    assert_eq!((result.len(), enough.work, enough.made), (lines, 0, lines));
    assert_eq!(crate::work::take(Work::FitSamples), lines + 1, "the lines and the point they start from");
    let mut short = Fit::new(lines, MAX_FIT_POINTS);
    assert!(!simplify_piece(&run, (tol, None), &opts, &mut short, &mut Vec::new()));
    // A segment holds 65,536 lines and no more: a tolerance that asks for exactly that many is made, one more is not.
    let tolerance_for = |need: usize| 0.75 * crate::fit::second_difference(&c) / ((need as f64 - 0.5) * (need as f64 - 0.5));
    for (need, holds) in [(MAX_CHORD_INTERVALS, true), (MAX_CHORD_INTERVALS + 1, false)] {
        let tol = tolerance_for(need);
        assert_eq!(line_steps(&c, tol), need);
        let opts = SimplifyOptions::default().tolerance(tol).straight_lines(true);
        let out = simplify_with(&arc, &opts);
        let lines = out.segments().filter(|s| matches!(s, kurbo::PathSeg::Line(_))).count();
        assert_eq!(lines, if holds { need } else { 0 }, "a tolerance for {need} lines");
    }
}

/// A part holds up to `fit_points` samples: a run that exactly fills one is one part (a nearly straight run is one line), and one sample more
/// makes two, with an anchor between them; and a segment that is the part alone is fitted with exactly its samples and not with one fewer.
#[test]
fn a_part_that_exactly_fills_the_samples_is_one_part() {
    let opts = SimplifyOptions::default().tolerance(1.0);
    // Two nearly straight cubics end to end, 10 long each: 20 intervals each.
    let straight = |from: f64| CubicBez::new((from, 0.0), (from + 3.0, 0.0001), (from + 7.0, -0.0001), (from + 10.0, 0.0));
    let mut path = BezPath::new();
    path.move_to((0.0, 0.0));
    for c in [straight(0.0), straight(10.0)] {
        path.curve_to(c.p1, c.p2, c.p3);
    }
    let run = run_of(&path);
    let m = match intervals(&run[0], 1.0, &opts) {
        Sampling::In(m) => m,
        _ => panic!("sampled"),
    };
    let parts = |max_points: usize| {
        let mut result = Vec::new();
        assert!(simplify_piece(&run, (1.0, None), &opts, &mut Fit::new(usize::MAX, max_points), &mut result));
        result.len()
    };
    // The first segment takes m + 1 samples, the second m more.
    assert_eq!(parts(2 * m + 1), 1, "exactly the samples: one part, one line");
    assert_eq!(parts(2 * m), 2, "one short: two parts");
    // One segment alone: its m + 1 samples fit a part of m + 1, and not of m (the piece stays: false).
    let one = &run[..1];
    let mut result = Vec::new();
    let mut fit = Fit::new(usize::MAX, m + 1);
    assert!(simplify_piece(one, (1.0, None), &opts, &mut fit, &mut result));
    assert!(result[0].line, "fitted to a line");
    assert_eq!(fit.made, result.len(), "what the call has made is what it made: charged for the samples, and given back what was not made");
    // The room the call has to make segments in is a part's samples too: exactly the samples is enough, one under is not.
    let limits = |elements: usize| crate::Limits { elements, ..crate::Limits::default() };
    let mut single = BezPath::new();
    single.move_to(run[0].c.p0);
    single.curve_to(run[0].c.p1, run[0].c.p2, run[0].c.p3);
    let lines = |p: &BezPath| p.segments().filter(|s| matches!(s, kurbo::PathSeg::Line(_))).count();
    assert_eq!(lines(&crate::with_limits(limits(m + 1), || simplify_with(&single, &opts))), 1, "room for the {} samples", m + 1);
    assert_eq!(lines(&crate::with_limits(limits(m), || simplify_with(&single, &opts))), 0, "one under: the piece stays");
    assert!(!simplify_piece(one, (1.0, None), &opts, &mut Fit::new(usize::MAX, m), &mut Vec::new()));
}

/// A smooth loop of long arcs with a turn of 20 degrees at its start (under the corner threshold) is fitted across it, along the bisector, and
/// not by its own tangents: the check that no turn is a corner compares each segment with the one before it, and every pair of arcs the
/// other way round (the end of the next, the start of this one) is 60 degrees apart, which would read as a corner.
#[test]
fn a_loop_of_long_arcs_with_a_small_turn_at_its_start_is_fitted_across_it() {
    let (r, step) = (100.0_f64, 30.0_f64);
    let h = r * 4.0 / 3.0 * (step.to_radians() / 4.0).tan();
    let arrive = Vec2::new(20.0_f64.to_radians().sin(), 20.0_f64.to_radians().cos());
    let anchors: Vec<A> = (0..12)
        .map(|i| {
            let (s, c) = (f64::from(i) * step).to_radians().sin_cos();
            let mut a = A::smooth(Point::new(r * c, r * s), Vec2::new(-s, c) * h);
            if i == 0 {
                a.inn = -arrive * h;
            }
            a
        })
        .collect();
    let path = path_of(&anchors, true);
    let (first, last) = own_ends(&path);
    assert!((first.dot(last).acos().to_degrees() - 20.0).abs() < 1e-6, "a turn of 20 degrees at the start");
    let across = crate::fit::unit(first + last).expect("a direction");
    let out = simplify_with(&path, &SimplifyOptions::default().tolerance(0.5).smooth(5.0));
    assert!(cubics(&out).len() < 12, "{} curves: fitted", cubics(&out).len());
    let (leaves, arrives) = ends_of(&out);
    assert!(leaves.dot(across) > 0.999_999_9 && arrives.dot(across) > 0.999_999_9, "across {across:?}; leaves {leaves:?}, arrives {arrives:?}");
}

/// A closed polygon of `n` vertices round the circle of radius `r` about the origin: smooth, with no corner in it.
fn circle_loop(n: usize, r: f64) -> BezPath {
    let pts: Vec<Point> = (0..n)
        .map(|i| {
            let a = i as f64 / n as f64 * std::f64::consts::TAU;
            Point::new(r * a.cos(), r * a.sin())
        })
        .collect();
    polyline_path(&pts, true)
}

/// The least cosine, over the anchors of `path` (a loop round the origin), between the direction the curve arrives at the anchor along or leaves
/// it along and the direction of the circle through the anchor there.
fn least_cosine_to_circle(path: &BezPath) -> f64 {
    let cs = cubics(path);
    let mut least: f64 = 1.0;
    for (i, c) in cs.iter().enumerate() {
        let Some(circle) = crate::fit::unit(Vec2::new(-c.p0.y, c.p0.x)) else { continue };
        let arriving = crate::fit::end_tangent(&cs[(i + cs.len() - 1) % cs.len()]);
        least = least.min(crate::fit::start_tangent(c).dot(circle)).min(arriving.dot(circle));
    }
    least
}

/// A smoothed loop with no corner that is fitted in parts leaves each joint of two parts along the circle, as it does anywhere else. Only a
/// part that is the whole of the piece is fitted across the loop's start: that direction is the circle's at the start and at no other
/// place, and a part given it at its joint turns the curve back on itself there (a loop of 3,000 vertices round a circle of radius 1,000 had a
/// joint 114 degrees off the circle, and one of 4,000 round a radius of 2,000 had one 176 degrees off). Parts of 3,000 samples, five of them
/// for this loop; the result is within the tolerance of the loop both ways, and fitted.
#[test]
fn a_smoothed_loop_fitted_in_parts_leaves_each_joint_along_the_circle() {
    let (tol, r) = (0.05, 50.0);
    let path = circle_loop(400, r);
    let opts = SimplifyOptions::default().tolerance(tol).smooth(0.5);
    let mut fit = Fit::new(usize::MAX, 3000);
    let out = simplify_within(&path, &opts, &mut fit);
    let k = cubics(&out).len();
    assert!((5..40).contains(&k), "{k} curves: fitted in several parts");
    let cos = least_cosine_to_circle(&out);
    assert!(cos > 0.999, "a joint leaves or arrives {:.1} degrees off the circle (cosine {cos})", cos.clamp(-1.0, 1.0).acos().to_degrees());
    assert_within(&cubics(&path), &cubics(&out), tol, "a loop fitted in parts");
    // As one part, the same loop is fitted across its start (and its other anchors are along the circle too).
    let whole = simplify_within(&path, &opts, &mut fit_unlimited());
    assert!(cubics(&whole).len() < k, "fitted in one part: fewer curves");
    assert!(least_cosine_to_circle(&whole) > 0.999);
}

/// The loops of the review at their sizes, with the default limits (parts of 524,288 samples: the first loop has 768,000 samples and the
/// second 1,024,000, so each is fitted in two parts): the direction at every joint is the circle's to within a small angle, and the
/// result is within the tolerance. Seconds in a release build, minutes in a debug one.
#[test]
#[ignore = "seconds: run with cargo test --release"]
fn big_smoothed_loops_fitted_in_parts_leave_each_joint_along_the_circle() {
    for (n, r) in [(3000, 1000.0), (4000, 2000.0)] {
        let path = circle_loop(n, r);
        let opts = SimplifyOptions::default().tolerance(0.01).smooth(0.5);
        let out = simplify_with(&path, &opts);
        let k = cubics(&out).len();
        assert!((4..400).contains(&k), "{n} vertices: {k} curves");
        let cos = least_cosine_to_circle(&out);
        assert!(cos > 0.999, "{n} vertices at radius {r}: a joint is {:.1} degrees off the circle", cos.clamp(-1.0, 1.0).acos().to_degrees());
        assert_within(&cubics(&path), &cubics(&out), 0.01, &format!("{n} vertices"));
    }
}

/// A piece that stays as it was gives back all that its parts made, those that were flushed before the one that gave out too: a straight run
/// of short lines in parts of 60 samples, then a line too long to be sampled, which stays as it is and takes the whole run with it.
#[test]
fn a_piece_that_stays_as_it_was_gives_back_what_its_flushed_parts_made() {
    let mut path = BezPath::new();
    path.move_to((0.0, 0.0));
    for i in 1..=80 {
        path.line_to((f64::from(i) * 0.5, 0.0));
    }
    path.line_to((3e9, 0.0));
    let opts = SimplifyOptions::default().tolerance(0.1);
    // Made without the long line: the parts are fitted and flushed.
    let mut short = BezPath::new();
    short.extend(path.iter().take(81));
    let mut fit = Fit::new(usize::MAX, 60);
    let fitted = simplify_within(&short, &opts, &mut fit);
    assert!(fit.made >= 2 && fitted.elements().len() < short.elements().len(), "{} made: the parts of the short run are fitted", fit.made);
    // With it: the piece stays as it was, and nothing is counted as made.
    let mut fit = Fit::new(usize::MAX, 60);
    let out = simplify_within(&path, &opts, &mut fit);
    assert_eq!(out.elements().len(), path.elements().len(), "the piece stays as it was");
    assert_eq!(fit.made, 0, "what the flushed parts made is given back");
    assert_eq!(fit.room(), crate::budget::limits().elements);
}

/// A part holds no more samples than the call has room to make segments for ([`Fit::part_size`]): a run of 12,600 samples under a limit of
/// 4,096 elements is fitted in parts of 4,096, and does not stay as it was for being longer than the room (as it would if the part were as long
/// as `max_points`, 2^20).
#[test]
fn a_part_holds_no_more_samples_than_the_call_has_room_for() {
    let tol = 0.05;
    let path = path_of(&arc_anchors(100.0, 0.0, 2.0, 91), false);
    let opts = SimplifyOptions::default().tolerance(tol);
    let limits = crate::Limits { elements: 4096, ..crate::Limits::default() };
    let mut fit = fit_unlimited();
    let out = crate::with_limits(limits, || simplify_within(&path, &opts, &mut fit));
    let k = cubics(&out).len();
    assert!((4..20).contains(&k), "{k} curves from 90: fitted in parts of the room the call has");
    assert!(fit.made <= limits.elements);
    assert_within(&cubics(&path), &cubics(&out), tol, "parts held to the room");
    assert_eq!(fit_unlimited().part_size(), MAX_FIT_POINTS.min(crate::budget::limits().elements));
}
