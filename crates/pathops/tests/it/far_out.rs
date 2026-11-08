//! Small shapes far from the origin: their areas, their normal forms and their offsets are the same as
//! near it, to the sweep's tolerance there. Areas taken about the origin cancel to nothing in the
//! products of coordinates that large (a square half a unit across at 1e8 read 0, and was dropped).

use craft_pathops::*;
use kurbo::{BezPath, Rect, Shape as _};

const NZ: FillRule = FillRule::NonZero;

/// Where the squares are put, along the diagonal.
const FAR: [f64; 7] = [1e6, 3e7, 1e8, 1e9, 1e10, 3e10, 1e11];

/// The square `side` across with its lower corner at `(x, x)`.
fn square_at(x: f64, side: f64) -> BezPath {
    let mut p = BezPath::new();
    p.move_to((x, x));
    p.line_to((x + side, x));
    p.line_to((x + side, x + side));
    p.line_to((x, x + side));
    p.close_path();
    p
}

/// The sweep's tolerance at `x`: 64 units in the last place, or a millionth.
fn tolerance(x: f64) -> f64 {
    (x * f64::EPSILON * 64.0).max(1e-6)
}

/// `got`, an operation's result, is the square from `lo` to `hi` (both at `(v, v)`), to `slack`: its
/// bounds, and its area, the crate's and that of its bounds.
fn is_square(what: &str, got: &BezPath, lo: f64, hi: f64, slack: f64) {
    let want = Rect::new(lo, lo, hi, hi);
    let b = got.bounding_box();
    let near = |a: f64, w: f64| (a - w).abs() <= slack;
    assert!(
        !got.elements().is_empty() && near(b.x0, want.x0) && near(b.y0, want.y0) && near(b.x1, want.x1) && near(b.y1, want.y1),
        "{what}: bounds {b:?}, not {want:?}"
    );
    let side = hi - lo;
    let a = area(got, NZ);
    assert!((a - side * side).abs() <= 4.0 * side * slack, "{what}: area {a}, not {}", side * side);
}

#[test]
fn the_area_of_a_small_square_far_out_is_its_area() {
    for x in FAR {
        let half = square_at(x, 0.5);
        assert_eq!(try_area(&half, NZ), Ok(0.25), "a square half a unit across at {x:e}");
        assert_eq!(area(&half, NZ), 0.25);
        is_square(&format!("normal form at {x:e}"), &try_normalize(&half, NZ).expect("normalized"), x, x + 0.5, tolerance(x));
    }
}

#[test]
fn a_small_square_far_out_is_inset_and_grown_as_near_the_origin() {
    for x in FAR {
        let (unit, big, slack) = (square_at(x, 1.0), square_at(x, 100.0), 2.0 * tolerance(x));
        let inset = try_offset_path(&unit, -0.25, Join::Miter, 4.0).expect("an inset");
        is_square(&format!("unit square at {x:e} inset by 0.25"), &inset, x + 0.25, x + 0.75, slack);
        let grown = try_offset_path(&unit, 0.25, Join::Miter, 4.0).expect("an outset");
        is_square(&format!("unit square at {x:e} grown by 0.25"), &grown, x - 0.25, x + 1.25, slack);
        let inset = try_offset_path(&big, -1.0, Join::Miter, 4.0).expect("an inset");
        is_square(&format!("square of 100 at {x:e} inset by 1"), &inset, x + 1.0, x + 99.0, slack);
    }
}

/// The planar map and Pathfinder find the faces of two small squares far out, overlapping by half,
/// as near the origin: three faces of a quarter, a half and a quarter of a square each.
#[test]
fn the_faces_of_small_squares_far_out_are_found() {
    for x in FAR {
        let (a, b) = (square_at(x, 1.0), square_at(x + 0.5, 1.0));
        let shapes = [Shape::new(a.clone(), NZ, 0), Shape::new(b.clone(), NZ, 1)];
        let (faces, _) = live_paint(&shapes);
        let mut areas: Vec<f64> = faces.iter().map(|f| area(&f.path, NZ)).collect();
        areas.sort_by(f64::total_cmp);
        let near = |got: &[f64], want: &[f64]| got.len() == want.len() && got.iter().zip(want).all(|(g, w)| (g - w).abs() <= 8.0 * tolerance(x));
        assert!(near(&areas, &[0.25, 0.75, 0.75]), "live_paint at {x:e}: {areas:?}");
        let divided = pathfinder(PathfinderOp::Divide, &shapes);
        let mut areas: Vec<f64> = divided.iter().map(|s| area(&s.path, NZ)).collect();
        areas.sort_by(f64::total_cmp);
        assert!(near(&areas, &[0.25, 0.75, 0.75]), "Divide at {x:e}: {areas:?}");
        let union = try_boolean(&a, NZ, &b, NZ, BoolOp::Union, DEFAULT_PRECISION).expect("a union");
        let bounds = union.bounding_box();
        assert!(
            (bounds.width() - 1.5).abs() <= 4.0 * tolerance(x) && (area(&union, NZ) - 1.75).abs() <= 8.0 * tolerance(x),
            "union at {x:e}: {bounds:?}"
        );
    }
}

/// A square with a hole far out is one face with its hole to Pathfinder, as near the origin: Divide and
/// Merge give it whole, its hole still a hole, where outer contours and holes are told apart by the
/// signs of their areas.
#[test]
fn a_square_with_a_hole_far_out_keeps_its_hole() {
    for x in FAR {
        let mut holed = square_at(x, 10.0);
        holed.move_to((x + 3.0, x + 3.0));
        for q in [(x + 3.0, x + 7.0), (x + 7.0, x + 7.0), (x + 7.0, x + 3.0)] {
            holed.line_to(q);
        }
        holed.close_path();
        let shapes = [Shape::new(holed, NZ, 0)];
        for op in [PathfinderOp::Divide, PathfinderOp::Merge] {
            let out = pathfinder(op, &shapes);
            let total: f64 = out.iter().map(|s| area(&s.path, NZ)).sum();
            let middle = kurbo::Point::new(x + 5.0, x + 5.0);
            assert!(
                out.len() == 1 && (total - 84.0).abs() <= 64.0 * tolerance(x) && out.iter().all(|s| s.path.winding(middle) == 0),
                "{op:?} at {x:e}: {} shapes, {total}",
                out.len()
            );
        }
    }
}
