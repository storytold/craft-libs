//! The limits an application can choose ([`Limits`], [`with_limits`]): each one is read where it
//! applies, the defaults are the crate's constants, and a scope ends when the closure does.

use craft_pathops::*;
use kurbo::{BezPath, PathEl, Point, Rect};

use crate::common::{poly, polyline, rectangle};

fn square() -> BezPath {
    rectangle(Rect::new(0.0, 0.0, 10.0, 10.0))
}

/// A lens: two cubics from `(x, y)` to `(x + 3, y)`, one bulging 3 above and one 3 below.
fn lens(x: f64, y: f64) -> BezPath {
    let mut b = BezPath::new();
    b.move_to((x, y));
    b.curve_to((x, y + 4.0), (x + 3.0, y + 4.0), (x + 3.0, y));
    b.curve_to((x + 3.0, y - 4.0), (x, y - 4.0), (x, y));
    b.close_path();
    b
}

fn subpaths(path: &BezPath) -> usize {
    path.elements().iter().filter(|e| matches!(e, PathEl::MoveTo(_))).count()
}

fn with(f: impl FnOnce(&mut Limits)) -> Limits {
    let mut limits = Limits::default();
    f(&mut limits);
    limits
}

#[test]
fn the_defaults_are_the_constants() {
    let l = Limits::default();
    assert_eq!(l.elements, MAX_ELEMENTS);
    assert_eq!(l.segment_pairs, MAX_SEGMENT_PAIRS);
    assert_eq!(l.offset_segment_pairs, MAX_OFFSET_SEGMENT_PAIRS);
    assert_eq!(l.sweep_line_pairs, MAX_SWEEP_LINE_PAIRS);
    assert_eq!(l.shape_builder_segments, SHAPE_BUILDER_MAX_SEGMENTS);
    assert_eq!(l.fit_points, MAX_FIT_POINTS);
    assert_eq!(l.fit_work, FIT_WORK);
    assert_eq!(l.join_work, JOIN_WORK);
    assert_eq!(l.redundant_work, REDUNDANT_WORK);
    assert_eq!(l.mask_work, MASK_WORK);
    assert_eq!(l.sweep_event_work, MAX_SWEEP_EVENT_WORK);
    assert_eq!(l, Limits::DEFAULT);
}

#[test]
fn a_lower_element_limit_refuses_what_the_default_takes() {
    let square = square();
    let n = square.elements().len();
    assert_eq!(try_area(&square, FillRule::NonZero), Ok(100.0));
    let at = with(|l| l.elements = n);
    assert_eq!(with_limits(at, || try_area(&square, FillRule::NonZero)), Ok(100.0), "at the limit");
    let under = with(|l| l.elements = n - 1);
    assert_eq!(with_limits(under, || try_area(&square, FillRule::NonZero)), Err(PathOpsError::TooComplex));
    assert_eq!(with_limits(under, || try_unite_all(&[(&square, FillRule::NonZero)])), Err(PathOpsError::TooComplex));
    assert_eq!(with_limits(under, || try_offset_path(&square, 1.0, Join::Round, 4.0)), Err(PathOpsError::TooComplex));
    // A stroke is held to the same limit in its outline, which has more elements than the path.
    let line = polyline(&[Point::new(0.0, 0.0), Point::new(50.0, 0.0)], false);
    let outline = try_outline_stroke(&line, 4.0, Cap::Round, Join::Round, 4.0).unwrap();
    assert!(outline.elements().len() > 4);
    let few = with(|l| l.elements = 4);
    assert_eq!(with_limits(few, || try_outline_stroke(&line, 4.0, Cap::Round, Join::Round, 4.0)), Err(PathOpsError::TooComplex));
    // `simplify` leaves a path past the limit as it is.
    let circle = ellipse_path(400);
    let simpler = simplify(&circle, 0.5);
    assert!(simpler.elements().len() < circle.elements().len());
    let small = with(|l| l.elements = 100);
    assert_eq!(with_limits(small, || simplify(&circle, 0.5)), circle);
}

fn ellipse_path(points: usize) -> BezPath {
    let pts: Vec<Point> = (0..points)
        .map(|i| {
            let a = std::f64::consts::TAU * i as f64 / points as f64;
            Point::new(50.0 + 40.0 * a.cos(), 50.0 + 40.0 * a.sin())
        })
        .collect();
    polyline(&pts, true)
}

#[test]
fn a_lower_segment_pair_limit_refuses_what_the_default_takes() {
    let square = square();
    let none = with(|l| l.segment_pairs = 0);
    assert_eq!(with_limits(none, || try_area(&square, FillRule::NonZero)), Err(PathOpsError::TooComplex));
    // Four sides, each meeting the next at its end: four pairs.
    let four = with(|l| l.segment_pairs = 4);
    assert_eq!(with_limits(four, || try_area(&square, FillRule::NonZero)), Ok(100.0));
    let three = with(|l| l.segment_pairs = 3);
    assert_eq!(with_limits(three, || try_area(&square, FillRule::NonZero)), Err(PathOpsError::TooComplex));
    let other = rectangle(Rect::new(5.0, 5.0, 15.0, 15.0));
    assert!(
        with_limits(none, || pathfinder(
            PathfinderOp::Unite,
            &[Shape::new(square.clone(), FillRule::NonZero, 0), Shape::new(other, FillRule::NonZero, 1)]
        ))
        .is_empty()
    );
}

#[test]
fn a_lower_sweep_line_limit_refuses_a_row_of_shapes_at_one_height() {
    let mut row = BezPath::new();
    for i in 0..30 {
        row.extend(lens(f64::from(i) * 5.0, 20.0).iter());
    }
    // 30 lenses of 4 pieces each at the same height: thousands of pairs on the sweep line.
    assert!(try_area(&row, FillRule::NonZero).is_ok());
    let few = with(|l| l.sweep_line_pairs = 200);
    assert_eq!(with_limits(few, || try_area(&row, FillRule::NonZero)), Err(PathOpsError::TooComplex));
    // The same lenses in a column share no height, but the pieces of each (6 pairs a lens, 180 in all).
    let mut column = BezPath::new();
    for i in 0..30 {
        column.extend(lens(0.0, 20.0 + f64::from(i) * 10.0).iter());
    }
    assert!(with_limits(few, || try_area(&column, FillRule::NonZero)).is_ok());
}

#[test]
fn a_lower_offset_pair_limit_refuses_an_offset_and_the_default_takes_it() {
    let square = square();
    assert!(try_offset_path(&square, 1.0, Join::Round, 4.0).is_ok());
    let none = with(|l| l.offset_segment_pairs = 0);
    assert_eq!(with_limits(none, || try_offset_path(&square, 1.0, Join::Round, 4.0)), Err(PathOpsError::TooComplex));
    // The offset's limit is its own: a boolean is not held to it.
    assert_eq!(with_limits(none, || try_area(&square, FillRule::NonZero)), Ok(100.0));
}

#[test]
fn a_lower_shape_builder_limit_leaves_only_the_filled_areas() {
    let shapes = [Shape::new(square(), FillRule::NonZero, 0), Shape::new(poly(&[(-5.0, 5.0), (15.0, 5.0)], false), FillRule::NonZero, 1)];
    let full = shape_builder(&shapes, false);
    assert!(!full.lines.is_empty() && full.regions.len() >= 2, "{} lines, {} regions", full.lines.len(), full.regions.len());
    let few = with(|l| l.shape_builder_segments = 2);
    let filled_only = with_limits(few, || shape_builder(&shapes, false));
    assert!(filled_only.lines.is_empty() && filled_only.edges.is_empty());
    assert_eq!(filled_only.regions.len(), 1, "the square, whole");
}

#[test]
fn lower_fit_limits_leave_the_path_as_it_is() {
    let circle = ellipse_path(400);
    let simpler = simplify(&circle, 0.5);
    assert!(simpler.elements().len() < circle.elements().len());
    let no_work = with(|l| l.fit_work = 0);
    assert_eq!(with_limits(no_work, || simplify(&circle, 0.5)), circle);
    let no_points = with(|l| l.fit_points = 2);
    assert_eq!(with_limits(no_points, || simplify(&circle, 0.5)), circle);
    // The fit's own limits are not the others'.
    let no_join = with(|l| l.join_work = 0);
    assert_eq!(with_limits(no_join, || simplify(&circle, 0.5)), simpler);
}

#[test]
fn a_lower_join_limit_leaves_the_subpaths_apart() {
    let paths = [poly(&[(0.0, 0.0), (10.0, 0.0)], false), poly(&[(10.0, 0.0), (10.0, 10.0)], false), poly(&[(10.0, 10.0), (0.0, 10.0)], false)];
    assert_eq!(subpaths(&join(&paths, 0.01)), 1);
    let none = with(|l| l.join_work = 0);
    assert_eq!(subpaths(&with_limits(none, || join(&paths, 0.01))), 3);
}

#[test]
fn a_lower_redundant_limit_leaves_the_point_a_closed_path_starts_at() {
    // A square that starts in the middle of its first side: the first pass along the path leaves
    // that point, and the search after it (the one the limit bounds) removes it.
    let path = poly(&[(5.0, 0.0), (10.0, 0.0), (10.0, 10.0), (0.0, 10.0), (0.0, 0.0)], true);
    let cleaned = remove_redundant_points(&path, 0.01);
    let none = with(|l| l.redundant_work = 0);
    let left = with_limits(none, || remove_redundant_points(&path, 0.01));
    assert!(cleaned.elements().len() < left.elements().len(), "{} against {}", cleaned.elements().len(), left.elements().len());
    // Collinear points along the path are merged in the first pass, whatever the limit.
    let line: Vec<Point> = (0..=100).map(|i| Point::new(f64::from(i), 0.0)).chain([Point::new(100.0, 10.0), Point::new(0.0, 10.0)]).collect();
    assert!(with_limits(none, || remove_redundant_points(&polyline(&line, true), 0.01)).elements().len() < 10);
}

#[test]
fn a_scope_ends_with_its_closure_nests_and_stays_on_its_thread() {
    let square = square();
    let none = with(|l| l.elements = 0);
    let refused = Err(PathOpsError::TooComplex);
    let answers = with_limits(none, || {
        let outer = try_area(&square, FillRule::NonZero);
        // The innermost scope wins, and the outer one is back after it.
        let inner = with_limits(Limits::default(), || try_area(&square, FillRule::NonZero));
        let after = try_area(&square, FillRule::NonZero);
        // Another thread has the defaults.
        let other = std::thread::scope(|s| s.spawn(|| try_area(&square, FillRule::NonZero)).join().unwrap());
        (outer, inner, after, other)
    });
    assert_eq!(answers, (refused, Ok(100.0), refused, Ok(100.0)));
    assert_eq!(try_area(&square, FillRule::NonZero), Ok(100.0), "back to the defaults");
    // Also when the closure unwinds.
    let unwound = std::panic::catch_unwind(|| with_limits(none, || panic!("the closure fails")));
    assert!(unwound.is_err());
    assert_eq!(try_area(&square, FillRule::NonZero), Ok(100.0), "back to the defaults after a panic");
    assert_eq!(with_limits(none, || 7), 7, "the closure's value comes back");
}

#[test]
fn a_lower_mask_limit_refuses_an_operation_with_many_inputs() {
    let empty = BezPath::new();
    let squares: Vec<BezPath> = (0..10).map(|i| rectangle(Rect::new(f64::from(i) * 20.0, 0.0, f64::from(i) * 20.0 + 10.0, 10.0))).collect();
    let mut inputs: Vec<(&BezPath, FillRule)> = squares.iter().map(|s| (s, FillRule::NonZero)).collect();
    inputs.extend(std::iter::repeat_n((&empty, FillRule::NonZero), 300));
    let union = |m: &[bool]| m.iter().any(|&b| b);
    assert!(close_area(&try_boolean_n(&inputs, union).unwrap(), 1000.0));
    let none = with(|l| l.mask_work = 0);
    assert_eq!(with_limits(none, || try_boolean_n(&inputs, union)), Err(PathOpsError::TooComplex));
    assert!(with_limits(none, || boolean_n(&inputs, union)).elements().is_empty());
    // Two inputs need only a few flags a segment, but some.
    assert_eq!(
        with_limits(none, || try_boolean(&squares[0], FillRule::NonZero, &squares[1], FillRule::NonZero, BoolOp::Union, DEFAULT_PRECISION)),
        Err(PathOpsError::TooComplex)
    );
}

fn close_area(path: &BezPath, want: f64) -> bool {
    (area(path, FillRule::NonZero) - want).abs() < 1e-6
}

#[test]
fn a_lower_sweep_event_limit_refuses_a_hatch_the_default_takes() {
    // 30 strokes slanting each way: swept in a blink by default.
    let hatch = {
        let mut b = BezPath::new();
        for i in 0..30 {
            let x = f64::from(i) * 4.0;
            b.extend(poly(&[(x, 0.0), (x + 1.0, 0.0), (x + 121.0, 120.0), (x + 120.0, 120.0)], true).iter());
            b.extend(poly(&[(x + 120.0, 0.0), (x + 121.0, 0.0), (x + 1.0, 120.0), (x, 120.0)], true).iter());
        }
        b
    };
    assert!(try_area(&hatch, FillRule::NonZero).is_ok());
    let none = with(|l| l.sweep_event_work = 0);
    assert_eq!(with_limits(none, || try_area(&hatch, FillRule::NonZero)), Err(PathOpsError::TooComplex));
    assert_eq!(with_limits(none, || try_unite_all(&[(&hatch, FillRule::NonZero)])), Err(PathOpsError::TooComplex));
}
