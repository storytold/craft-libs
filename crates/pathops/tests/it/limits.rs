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
    assert_eq!(l.ends_at_a_point, MAX_ENDS_AT_A_POINT);
    assert_eq!(l.crossing_work, MAX_CROSSING_WORK);
    assert_eq!(l.pass_work, MAX_PASS_WORK);
    assert_eq!(l.coverage_entries, MAX_COVERAGE_ENTRIES);
    assert_eq!(l.fill_work, MAX_FILL_WORK);
    assert_eq!(MAX_FILL_WORK, 1 << 26);
    assert_eq!(l, Limits::DEFAULT);
}

/// `n` rectangles nested in one shape, each 1,000 times as tall as it is wide (the bounds of their
/// sides never overlap but at the corners, so that every limit counted before the sweep takes thousands).
fn tall_nested(n: usize) -> BezPath {
    let mut one = BezPath::new();
    for j in 0..n {
        let (j, m) = (j as f64, (2 * n) as f64);
        one.extend(rectangle(Rect::new(10.0 * j, 10_000.0 * j, 10.0 * (m - j), 10_000.0 * (m - j))).iter());
    }
    one
}

#[test]
fn a_lower_fill_limit_gives_no_map_for_contours_nested_deep() {
    let shapes = [Shape::new(tall_nested(200), FillRule::NonZero, 0)];
    let (faces, _) = live_paint(&shapes);
    assert_eq!(faces.len(), 200, "a face between each two");
    // 200 nested in one shape: each face is wound against the subpaths round it, and each hole against
    // the faces round it, 6 elements each: about 6 x 200² of the 67 million.
    let low = with(|l| l.fill_work = 200_000);
    assert_eq!(with_limits(low, || live_paint(&shapes)), (Vec::new(), Vec::new()), "no map past the limit");
    // shape_builder then gives the filled areas, as for any map it cannot build: the outer rectangle.
    let filled = with_limits(low, || shape_builder(&shapes, false));
    assert_eq!(filled.regions.len(), 1);
    assert_eq!(with_limits(low, || shape_builder(&shapes, false)).regions, shape_builder(&shapes, false).regions);
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
    // Also when the closure unwinds (where panics abort, it cannot).
    #[cfg(panic = "unwind")]
    {
        let unwound = std::panic::catch_unwind(|| with_limits(none, || panic!("the closure fails")));
        assert!(unwound.is_err());
        assert_eq!(try_area(&square, FillRule::NonZero), Ok(100.0), "back to the defaults after a panic");
    }
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

#[test]
fn a_lower_mask_limit_gives_no_regions_and_no_division() {
    let shapes: Vec<Shape> = (0..6)
        .map(|i| Shape::new(rectangle(Rect::new(f64::from(i) * 20.0, 0.0, f64::from(i) * 20.0 + 10.0, 10.0)), FillRule::NonZero, u64::from(i as u32)))
        .collect();
    assert_eq!(pathfinder(PathfinderOp::Divide, &shapes).len(), 6);
    assert_eq!(regions(&shapes).len(), 6);
    let none = with(|l| l.mask_work = 0);
    assert!(with_limits(none, || pathfinder(PathfinderOp::Divide, &shapes)).is_empty());
    assert!(with_limits(none, || regions(&shapes)).is_empty());
    // The other operations of the same shapes do not list covering inputs.
    assert_eq!(with_limits(none, || pathfinder(PathfinderOp::Unite, &shapes)).len(), 1);
}

fn pie(k: usize) -> BezPath {
    let mut p = BezPath::new();
    for i in 0..k {
        let (a, c) = (std::f64::consts::TAU * i as f64 / k as f64, std::f64::consts::TAU * (i + 1) as f64 / k as f64);
        p.move_to((0.0, 0.0));
        p.line_to((100.0 * a.cos(), 100.0 * a.sin()));
        p.line_to((100.0 * c.cos(), 100.0 * c.sin()));
        p.close_path();
    }
    p
}

#[test]
fn a_lower_limit_on_ends_at_a_point_refuses_a_pie_the_default_takes() {
    let slices = pie(30);
    assert!(try_area(&slices, FillRule::NonZero).is_ok());
    // 30 slices have two ends each at the centre.
    let sixty = with(|l| l.ends_at_a_point = 60);
    assert!(with_limits(sixty, || try_area(&slices, FillRule::NonZero)).is_ok());
    let fewer = with(|l| l.ends_at_a_point = 59);
    assert_eq!(with_limits(fewer, || try_area(&slices, FillRule::NonZero)), Err(PathOpsError::TooComplex));
    // The default is 512 ends: a pie of 256 slices is within it, and of 257 is not.
    assert!(try_area(&pie(256), FillRule::NonZero).is_ok());
    assert_eq!(try_area(&pie(257), FillRule::NonZero), Err(PathOpsError::TooComplex));
}

#[test]
fn a_lower_coverage_limit_refuses_nested_shapes_the_default_takes() {
    let nested: Vec<BezPath> = (1..=10).map(|k| rectangle(Rect::new(-f64::from(k), -f64::from(k), f64::from(k), f64::from(k)))).collect();
    let inputs: Vec<(&BezPath, FillRule)> = nested.iter().map(|p| (p, FillRule::NonZero)).collect();
    let union = |m: &[bool]| m.iter().any(|&b| b);
    assert!(try_boolean_n(&inputs, union).is_ok());
    // Ten nested squares hold 4 x 10 x 11 entries.
    let at = with(|l| l.coverage_entries = 440);
    assert!(with_limits(at, || try_boolean_n(&inputs, union)).is_ok());
    let under = with(|l| l.coverage_entries = 439);
    assert_eq!(with_limits(under, || try_boolean_n(&inputs, union)), Err(PathOpsError::TooComplex));
    let shapes: Vec<Shape> = nested.iter().enumerate().map(|(i, p)| Shape::new(p.clone(), FillRule::NonZero, i as u64)).collect();
    assert_eq!(pathfinder(PathfinderOp::Exclude, &shapes).len(), 1);
    assert!(with_limits(under, || pathfinder(PathfinderOp::Exclude, &shapes)).is_empty());
    assert!(with_limits(under, || regions(&shapes)).is_empty());
}

#[test]
fn a_lower_pass_limit_gives_no_shapes_for_a_fan() {
    // Triangles that meet at one point, each its own key: a batch of passes each.
    let shapes: Vec<Shape> = (0..12)
        .map(|i| {
            let a = std::f64::consts::TAU * f64::from(i) / 12.0;
            let b = a + 0.2;
            Shape::new(poly(&[(0.0, 0.0), (100.0 * a.cos(), 100.0 * a.sin()), (100.0 * b.cos(), 100.0 * b.sin())], true), FillRule::NonZero, i as u64)
        })
        .collect();
    assert_eq!(pathfinder(PathfinderOp::Merge, &shapes).len(), 12);
    let none = with(|l| l.pass_work = 0);
    for op in [PathfinderOp::Merge, PathfinderOp::Divide, PathfinderOp::Trim] {
        assert!(with_limits(none, || pathfinder(op, &shapes)).is_empty(), "{op:?}");
    }
    // Operations that make no such passes are not held to it.
    assert_eq!(with_limits(none, || pathfinder(PathfinderOp::Unite, &shapes)).len(), 1);
}

/// `k` spikes (a triangle from an apex to two points on a circle of 100), the apexes on a ring of
/// radius `rho`, and with `far` one cubic whose control points reach that far.
fn spikes(k: usize, rho: f64, far: Option<f64>) -> BezPath {
    let mut p = BezPath::new();
    for i in 0..k {
        let a = std::f64::consts::TAU * i as f64 / k as f64;
        let b = a + std::f64::consts::TAU / (3.0 * k as f64);
        let m = (a + b) / 2.0;
        p.move_to((rho * m.cos(), rho * m.sin()));
        p.line_to((100.0 * a.cos(), 100.0 * a.sin()));
        p.line_to((100.0 * b.cos(), 100.0 * b.sin()));
        p.close_path();
    }
    if let Some(far) = far {
        p.move_to((200.0, 0.0));
        p.curve_to((far, 0.0), (far, 10.0), (200.0, 10.0));
        p.close_path();
    }
    p
}

#[test]
fn a_far_control_point_widens_the_tolerance_the_ends_are_counted_at() {
    // 300 spikes whose apexes lie within a hundredth of each other are 600 ends: to a sweep that
    // tolerates a millionth they are 300 points, and it takes them in a few milliseconds.
    assert!(try_area(&spikes(300, 0.004, None), FillRule::NonZero).is_ok());
    // One cubic with control points at 1e12 sets a tolerance of 0.014, and they are one point (5 s of sweep).
    assert_eq!(try_area(&spikes(300, 0.004, Some(1e12)), FillRule::NonZero), Err(PathOpsError::TooComplex));
    assert_eq!(try_area(&spikes(400, 0.004, Some(1e12)), FillRule::NonZero), Err(PathOpsError::TooComplex));
    // At 512 ends the sweep is made, as without the cubic.
    assert!(try_area(&spikes(256, 0.004, Some(1e12)), FillRule::NonZero).is_ok());
}

/// `k` thin triangles, each with a long edge through the origin from a point on a circle of 100 to
/// the opposite one: `k` lines that cross there, and no end of any.
fn lines_through_a_point(k: usize) -> BezPath {
    let mut p = BezPath::new();
    for i in 0..k {
        let a = std::f64::consts::PI * (i as f64 + 0.5) / k as f64;
        let (c, s) = (100.0 * a.cos(), 100.0 * a.sin());
        p.move_to((c, s));
        p.line_to((-c, -s));
        p.line_to((-c - 0.5 * s / 100.0, -s + 0.5 * c / 100.0));
        p.close_path();
    }
    p
}

#[test]
fn lines_that_cross_at_one_point_are_limited_by_their_work() {
    let star = lines_through_a_point(40);
    assert!(try_area(&star, FillRule::NonZero).is_ok());
    // The work of 40 lines through a point is 40 x 39², and of the rest of the shapes next to nothing.
    let roomy = with(|l| l.crossing_work = 100_000);
    assert!(with_limits(roomy, || try_area(&star, FillRule::NonZero)).is_ok());
    let tight = with(|l| l.crossing_work = 60_000);
    assert_eq!(with_limits(tight, || try_area(&star, FillRule::NonZero)), Err(PathOpsError::TooComplex));
    assert_eq!(with_limits(tight, || try_unite_all(&[(&star, FillRule::NonZero)])), Err(PathOpsError::TooComplex));
    // By default 257 lines through a point are refused, at once (a thousand took a minute and a half).
    assert_eq!(try_area(&lines_through_a_point(257), FillRule::NonZero), Err(PathOpsError::TooComplex));
    assert_eq!(try_area(&lines_through_a_point(1_020), FillRule::NonZero), Err(PathOpsError::TooComplex));
    // Lines that cross apart are no matter, however many.
    let mut hatch = BezPath::new();
    for i in 0..60 {
        let y = 3.0 * f64::from(i);
        hatch.move_to((0.0, y));
        hatch.line_to((300.0, y + 100.7));
        hatch.line_to((300.0, y + 103.7));
        hatch.close_path();
        hatch.move_to((0.0, 300.0 - y + 0.3));
        hatch.line_to((300.0, 200.0 - y));
        hatch.line_to((300.0, 203.0 - y));
        hatch.close_path();
    }
    assert!(with_limits(with(|l| l.crossing_work = 100), || try_area(&hatch, FillRule::NonZero)).is_ok());
}

#[test]
fn nested_squares_crossed_by_thin_bars_are_limited_by_the_pieces_they_are_cut_into() {
    // n nested squares and m bars across them: the sweep cuts each side at each bar, and each piece keeps its own list.
    let nested_and_barred = |n: i32, m: i32| -> Vec<Shape> {
        let mut shapes: Vec<Shape> = (1..=n)
            .map(|k| Shape::new(rectangle(Rect::new(-f64::from(k), -f64::from(k), f64::from(k), f64::from(k))), FillRule::NonZero, k as u64))
            .collect();
        let mut bars = BezPath::new();
        let w = 2.0 * f64::from(n) / f64::from(m);
        for i in 0..m {
            let x = -f64::from(n) + w * (f64::from(i) + 0.25);
            bars.extend(rectangle(Rect::new(x, -f64::from(n) - 1.0, x + 0.3 * w, f64::from(n) + 1.0)).iter());
        }
        shapes.push(Shape::new(bars, FillRule::NonZero, 0));
        shapes
    };
    let small = nested_and_barred(12, 12);
    assert_eq!(pathfinder(PathfinderOp::Exclude, &small).len(), 1);
    // Their entries are at most 12 x 12 x 12 x 8 and not under 12 x 12 x 12 x 2: a limit between refuses them.
    let tight = with(|l| l.coverage_entries = 1_000);
    assert!(with_limits(tight, || pathfinder(PathfinderOp::Exclude, &small)).is_empty());
    let loose = with(|l| l.coverage_entries = 14_000);
    assert_eq!(with_limits(loose, || pathfinder(PathfinderOp::Exclude, &small)).len(), 1);
    // By default 300 and 300 (1.3 GB) are refused before the sweep.
    let big = nested_and_barred(300, 300);
    assert!(pathfinder(PathfinderOp::Exclude, &big).is_empty());
    assert!(pathfinder(PathfinderOp::Divide, &big).is_empty());
    let paths: Vec<(&BezPath, FillRule)> = big.iter().map(|s| (&s.path, s.rule)).collect();
    assert_eq!(try_boolean_n(&paths, |m| m[0]), Err(PathOpsError::TooComplex));
}
