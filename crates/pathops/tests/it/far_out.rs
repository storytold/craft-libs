//! Small shapes far from the origin: their areas, their normal forms and their offsets are the same as
//! near it, to the sweep's tolerance there. Areas taken about the origin cancel to nothing in the
//! products of coordinates that large (a square half a unit across at 1e8 read 0, and was dropped).

use craft_pathops::*;
use kurbo::{Arc, BezPath, Point, Rect, Shape as _, SvgArc, Vec2};

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
        // Each face where it is: the overlap, and the bounds of the rest of each square.
        let placed = |paths: Vec<&BezPath>, what: &str| {
            let boxes =
                [Rect::new(x + 0.5, x + 0.5, x + 1.0, x + 1.0), Rect::new(x, x, x + 1.0, x + 1.0), Rect::new(x + 0.5, x + 0.5, x + 1.5, x + 1.5)];
            for want in boxes {
                let same = |b: Rect| [b.x0 - want.x0, b.y0 - want.y0, b.x1 - want.x1, b.y1 - want.y1].iter().all(|d| d.abs() <= 2.0 * tolerance(x));
                assert!(paths.iter().any(|p| same(p.bounding_box())), "{what} at {x:e}: no face at {want:?}");
            }
        };
        placed(faces.iter().map(|f| &f.path).collect(), "live_paint");
        let divided = pathfinder(PathfinderOp::Divide, &shapes);
        let mut areas: Vec<f64> = divided.iter().map(|s| area(&s.path, NZ)).collect();
        areas.sort_by(f64::total_cmp);
        assert!(near(&areas, &[0.25, 0.75, 0.75]), "Divide at {x:e}: {areas:?}");
        placed(divided.iter().map(|s| &s.path).collect(), "Divide");
        let union = try_boolean(&a, NZ, &b, NZ, BoolOp::Union, DEFAULT_PRECISION).expect("a union");
        let bounds = union.bounding_box();
        let edges = [bounds.x0 - x, bounds.y0 - x, bounds.x1 - (x + 1.5), bounds.y1 - (x + 1.5)];
        assert!(
            edges.iter().all(|d| d.abs() <= 4.0 * tolerance(x)) && (area(&union, NZ) - 1.75).abs() <= 8.0 * tolerance(x),
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
            // The hole where it was: a subpath from (x + 3, x + 3) to (x + 7, x + 7).
            let hole = crate::common::split(&out[0].path).iter().map(BezPath::bounding_box).find(|b| b.width() < 5.0);
            let at = |b: Rect| [b.x0 - (x + 3.0), b.y0 - (x + 3.0), b.x1 - (x + 7.0), b.y1 - (x + 7.0)].iter().all(|d| d.abs() <= 2.0 * tolerance(x));
            assert!(hole.is_some_and(at), "{op:?} at {x:e}: the hole is at {hole:?}");
        }
    }
}

/// An arc from `from` to `to` that bows by `bow` (its sagitta, signed), as cubics.
fn arc_to(path: &mut BezPath, from: Point, to: Point, bow: f64) {
    let chord = (to - from).hypot();
    let radius = (chord * chord / 4.0 + bow * bow) / (2.0 * bow.abs());
    let arc = SvgArc { from, to, radii: Vec2::new(radius, radius), x_rotation: 0.0, large_arc: false, sweep: bow > 0.0 };
    Arc::from_svg_arc(&arc).expect("an arc").to_cubic_beziers(1e-6, |a, b, c| path.curve_to(a, b, c));
}

/// An open run of two arcs, each of chord 0.3 and bowing by 0.1 (one one way, one the other), starting at `(x, y)`, all scaled by 1,000.
fn s_run(x: f64, y: f64) -> BezPath {
    let at = |dx: f64| Point::new((x + dx) * 1_000.0, y * 1_000.0);
    let mut path = BezPath::new();
    path.move_to(at(0.0));
    arc_to(&mut path, at(0.0), at(0.3), 100.0);
    arc_to(&mut path, at(0.3), at(0.6), -100.0);
    path
}

/// The sweep checks, in a build with debug assertions, that the order it gives two curves is the one their positions have where it changes
/// (`intersect_cubics_rec` in `linesweeper`'s `curve/mod.rs`, lines 1301 to 1302 and 1305 to 1306 of 0.5.0), and the check fails for curves that run along one another when they are a few
/// hundred units across and at about 10 million from the path they outline (a stroke of 20 million across a run of two arcs, 600 across,
/// at (3,000, 4,000): run 163 of a grid of them; 1 of 700 at 1e7 of distance, 13 at 3e7, 259 at 1e8). The crate refuses what the check fails
/// in such a build, as `Degenerate`; in a release build the check is not made and the sweep goes on, and what it makes is what the same run
/// beside it, which the check passes, makes: in 700 runs at each distance from 1e6 to 1e10 areas that differ from one another by no more
/// than 1e-9 up to 1e8 of distance (4e-9 at 1e9 and 3e-6 at 1e10, the rounding of the areas of figures that large), in under 11 ms each.
#[test]
fn a_stroke_that_fails_the_sweeps_debug_check_is_refused_in_a_debug_build_and_made_in_a_release_one() {
    let width = 2.0e7;
    let outline = |run: &BezPath| try_outline_stroke(run, width, Cap::Butt, Join::Miter, 4.0);
    let beside = outline(&s_run(2.0, 4.0)).expect("the run beside it is outlined");
    let tripping = outline(&s_run(3.0, 4.0));
    if cfg!(debug_assertions) {
        assert_eq!(tripping.map(|_| ()), Err(PathOpsError::Degenerate));
    } else {
        let made = tripping.expect("a release build carries on");
        let (a, b) = (area(&made, NZ), area(&beside, NZ));
        assert!(a > 1.0e10 && (a - b).abs() <= 1e-8 * b, "{a} against {b}");
        let (m, n) = (made.bounding_box(), beside.bounding_box());
        assert!((m.width() - n.width()).abs() < 1.0 && (m.height() - n.height()).abs() < 1.0, "{m:?} against {n:?}");
    }
}
