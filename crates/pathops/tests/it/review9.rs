//! Claims made in the ninth review of the pull request, each as the smallest test that fails when the
//! claim is true.

use craft_pathops::*;
use kurbo::{BezPath, Rect};

use crate::common::{ellipse, rectangle};

const NZ: FillRule = FillRule::NonZero;

/// Every operation of the Pathfinder.
const OPS: [PathfinderOp; 10] = [
    PathfinderOp::Unite,
    PathfinderOp::MinusFront,
    PathfinderOp::Intersect,
    PathfinderOp::Exclude,
    PathfinderOp::Divide,
    PathfinderOp::Trim,
    PathfinderOp::Merge,
    PathfinderOp::Crop,
    PathfinderOp::Outline,
    PathfinderOp::MinusBack,
];

/// The default limits with `set` applied.
fn limits(set: impl FnOnce(&mut Limits)) -> Limits {
    let mut l = Limits::default();
    set(&mut l);
    l
}

/// A square, and beside it a closed loop of one cubic with the handle `handle` (the other along the same line,
/// so that it is no loop round an area, as one whose handle is no number is not): the loop is left out when the
/// path is filled, and what is left is the square.
fn square_and_loop(handle: f64) -> BezPath {
    let mut path = rectangle(Rect::new(0.0, 0.0, 10.0, 10.0));
    path.move_to((20.0, 0.0));
    path.curve_to((handle, 0.0), (30.0, 0.0), (20.0, 0.0));
    path.close_path();
    path
}

// ---- the shapes as given, before the fill leaves a loop out

/// A closed loop of one cubic with a handle that is no number is left out by the fill, and the Pathfinder
/// of the rest gave a result without it: the try forms check the shapes as given. Past what the sweep takes
/// it is the same (`Degenerate`); the forms that give no error give what they did, the square's.
#[test]
fn try_pathfinder_checks_the_shapes_as_given_before_the_fill_leaves_a_loop_out() {
    for (handle, error) in [(f64::NAN, PathOpsError::NonFinite), (f64::INFINITY, PathOpsError::NonFinite), (1e300, PathOpsError::Degenerate)] {
        let shapes = [Shape::new(square_and_loop(handle), NZ, 0), Shape::new(rectangle(Rect::new(5.0, 5.0, 15.0, 15.0)), NZ, 1)];
        for op in OPS {
            assert_eq!(try_pathfinder(op, &shapes), Err(error), "{op:?} with a handle of {handle}");
        }
        // The forms with no error are as they were: the loop is left out and the rest is made.
        assert!(!pathfinder(PathfinderOp::Divide, &shapes).is_empty(), "the square is divided where the loop is left out ({handle})");
        assert!(!pathfinder(PathfinderOp::Unite, &shapes).is_empty() || handle.is_nan());
    }
    // And a loop with a handle that is a number is no trouble.
    let shapes = [Shape::new(square_and_loop(30.0), NZ, 0), Shape::new(rectangle(Rect::new(5.0, 5.0, 15.0, 15.0)), NZ, 1)];
    for op in OPS {
        assert!(try_pathfinder(op, &shapes).is_ok(), "{op:?}");
    }
}

/// The same for the planar map: a path with such a loop was mapped without it, and `try_shape_builder` gave
/// the map, where `NonFinite` is what it documents. The infallible one gives the regions of the square.
#[test]
fn try_shape_builder_checks_the_shapes_as_given_before_the_map_leaves_a_loop_out() {
    for (handle, error) in [(f64::NAN, PathOpsError::NonFinite), (1e300, PathOpsError::Degenerate)] {
        let shapes = [Shape::new(square_and_loop(handle), NZ, 0)];
        for edges in [false, true] {
            assert_eq!(try_shape_builder(&shapes, edges), Err(error), "a handle of {handle}, edges {edges}");
        }
        assert_eq!(shape_builder(&shapes, false).regions.len(), 1, "the square is a region where the loop is left out");
        // With an open path among them, which the map takes the cuts from, the same.
        let open = Shape::new(crate::common::poly(&[(-5.0, 5.0), (25.0, 5.0)], false), NZ, 1);
        let with_line = [shapes[0].clone(), open];
        assert_eq!(try_shape_builder(&with_line, false), Err(error), "a handle of {handle}, with a cutting line");
        assert!(!shape_builder(&with_line, false).regions.is_empty());
    }
}

// ---- a limit reached after the map, which is an error to the try forms

/// Overlapping circles: faces covered by several shapes, so that the entries the masks are found by are
/// many.
fn circles() -> Vec<Shape> {
    (0..14u32).map(|i| Shape::new(ellipse(Rect::new(f64::from(i) * 7.0, 0.0, f64::from(i) * 7.0 + 40.0, 40.0)), NZ, u64::from(i))).collect()
}

/// More entries of the masks than `mask_work` allows, in the faces of closed shapes, are no faces: Divide
/// gives no shapes, and `try_pathfinder` says why instead of giving an empty drawing; and `try_shape_builder`
/// the same, where the faces of the filled areas were left out of a map that was made.
#[test]
fn the_faces_of_closed_shapes_past_the_mask_limit_are_an_error_to_the_try_forms() {
    let shapes = circles();
    let free = pathfinder(PathfinderOp::Divide, &shapes);
    let regions = shape_builder(&shapes, false).regions;
    assert!(free.len() > 14 && regions.len() == free.len());
    assert_eq!(try_pathfinder(PathfinderOp::Divide, &shapes).map(|s| s.len()), Ok(free.len()));
    assert_eq!(try_shape_builder(&shapes, false).map(|b| b.regions.len()), Ok(regions.len()));
    let tight = limits(|l| l.mask_work = 10);
    // The infallible forms are as they were: no shapes, and no regions of the filled areas.
    assert!(with_limits(tight, || pathfinder(PathfinderOp::Divide, &shapes)).is_empty());
    assert!(with_limits(tight, || shape_builder(&shapes, false)).regions.is_empty());
    assert_eq!(with_limits(tight, || try_pathfinder(PathfinderOp::Divide, &shapes)), Err(PathOpsError::TooComplex));
    assert_eq!(with_limits(tight, || try_shape_builder(&shapes, false)), Err(PathOpsError::TooComplex));
    assert_eq!(with_limits(tight, || try_shape_builder(&shapes, true)), Err(PathOpsError::TooComplex));
}

/// The same for the work of the contour passes: a fan of shapes at one point is a pass each, and past
/// `pass_work` Divide has no faces.
#[test]
fn the_faces_of_a_fan_past_the_pass_limit_are_an_error_to_the_try_forms() {
    // Thirty squares that share the origin as a corner, each in a quadrant turned a little more.
    let fan: Vec<Shape> = (0..30u32)
        .map(|i| {
            let a = f64::from(i) * 0.2;
            let (c, s) = (a.cos() * 50.0, a.sin() * 50.0);
            Shape::new(crate::common::poly(&[(0.0, 0.0), (c, s), (c - s, s + c), (-s, c)], true), NZ, u64::from(i))
        })
        .collect();
    assert!(!pathfinder(PathfinderOp::Divide, &fan).is_empty());
    let tight = limits(|l| l.pass_work = 10);
    assert!(with_limits(tight, || pathfinder(PathfinderOp::Divide, &fan)).is_empty());
    assert_eq!(with_limits(tight, || try_pathfinder(PathfinderOp::Divide, &fan)), Err(PathOpsError::TooComplex));
    assert_eq!(with_limits(tight, || try_shape_builder(&fan, false)), Err(PathOpsError::TooComplex));
}

// ---- offsets of faces whose contours touch

/// The unit cells of the `n` by `n` grid that `mask` has set, as closed squares of one path (each touches
/// its neighbours along an edge, and the ones across a corner at a point).
fn cells(mask: u32, n: usize) -> BezPath {
    let mut path = BezPath::new();
    for i in (0..n * n).filter(|i| mask >> i & 1 == 1) {
        let (x, y) = ((i % n) as f64, (i / n) as f64);
        path.extend(rectangle(Rect::new(x, y, x + 1.0, y + 1.0)).iter());
    }
    path
}

/// The area of the cells grown by `d` (the points within `d` of one in the largest of the distances along the
/// axes), which is what a miter offset of axis-aligned squares is, on a raster of tenths (`d` a multiple of
/// one). `d` negative takes the cells in by it: the points whose square of that half-width is all in them.
fn grid_offset_area(mask: u32, n: usize, d: f64) -> f64 {
    let (step, reach) = (0.1, d.abs());
    let set: Vec<(f64, f64)> = (0..n * n).filter(|i| mask >> i & 1 == 1).map(|i| ((i % n) as f64, (i / n) as f64)).collect();
    let in_cell = |x: f64, y: f64| set.iter().any(|&(cx, cy)| x >= cx && x < cx + 1.0 && y >= cy && y < cy + 1.0);
    let w = (reach / step).round() as i64;
    let (low, high) = if d > 0.0 { (-w, n as i64 * 10 + w) } else { (0, n as i64 * 10) };
    let mut count = 0usize;
    for ix in low..high {
        for iy in low..high {
            let (x, y) = (ix as f64 * step + step / 2.0, iy as f64 * step + step / 2.0);
            let covered = if d > 0.0 {
                set.iter().any(|&(cx, cy)| x >= cx - reach && x <= cx + 1.0 + reach && y >= cy - reach && y <= cy + 1.0 + reach)
            } else {
                // Every pixel the square covers, the half pixels at its edges too.
                (-w..=w).all(|dx| (-w..=w).all(|dy| in_cell(x + dx as f64 * step, y + dy as f64 * step)))
            };
            count += usize::from(covered);
        }
    }
    count as f64 * step * step
}

/// Offsets of faces whose contours touch at a point (cells of a grid, which meet across a corner, with
/// holes that touch the same way and are closed up or opened out when `d` is half the width between them)
/// are exact: the arrangement's tree of contours, which can put a contour that touches another inside
/// it, does not decide which of them are spurious. Every 211th of the patterns of cells in a 4 by 4 grid,
/// grown and taken in by distances to the tenth, against the area of the same by raster.
#[test]
fn an_offset_of_cells_that_touch_at_corners_has_the_area_it_should() {
    let n = 4;
    let mut checked = 0;
    for mask in (1..1u32 << (n * n)).step_by(211) {
        for d in [0.2, 0.5, 0.6, -0.2, -0.5] {
            let out = offset_path(&cells(mask, n), d, Join::Miter, 10.0);
            let (got, want) = (area(&out, NZ), grid_offset_area(mask, n, d));
            assert!((got - want).abs() < 1e-6 * want.max(1.0), "pattern {mask:016b} by {d}: an area of {got} where it is {want}");
            checked += 1;
        }
    }
    assert!(checked > 1_000, "{checked} offsets");
}
