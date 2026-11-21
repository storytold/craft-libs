//! The faces of an offset are judged spurious by a point inside the face, holes and islands left out:
//! a point inside the outer contour alone can lie in one of its holes, near the path, and the whole
//! face was dropped.

use craft_pathops::*;
use kurbo::{BezPath, Point, Shape as _};

use crate::common::rel_close as close;

const NZ: FillRule = FillRule::NonZero;

/// The rectangle from `(x0, y0)` to `(x1, y1)` as a subpath of `p`, one way round (`turn`) or the other.
fn rect(p: &mut BezPath, (x0, y0): (f64, f64), (x1, y1): (f64, f64), turn: bool) {
    p.move_to((x0, y0));
    if turn {
        p.line_to((x0, y1));
        p.line_to((x1, y1));
        p.line_to((x1, y0));
    } else {
        p.line_to((x1, y0));
        p.line_to((x1, y1));
        p.line_to((x0, y1));
    }
    p.close_path();
}

/// A 100 x 100 square with a rectangular hole from `lo` to `hi`.
fn square_with_hole(lo: (f64, f64), hi: (f64, f64)) -> BezPath {
    let mut p = BezPath::new();
    rect(&mut p, (0.0, 0.0), (100.0, 100.0), false);
    rect(&mut p, lo, hi, true);
    p
}

/// The area of the inset of `path` by `d` (mitred joins), through both forms.
fn inset_area(path: &BezPath, d: f64) -> (f64, f64) {
    let tried = try_offset_path(path, -d, Join::Miter, 4.0).expect("an inset");
    (area(&tried, NZ), area(&offset_path(path, -d, Join::Miter, 4.0), NZ))
}

/// A square hole 20 across, at x from `left`: the outer contour of the inset alone has its widest span
/// at mid-height through the middle of the square, x = 50, which is on or near the hole's edge for some
/// of these (50 to 70, 50.2 to 70.2, 30.1 to 50.1), and the whole inset was empty.
#[test]
fn an_inset_keeps_a_face_whose_outer_contour_alone_is_widest_through_a_hole() {
    for (left, d) in [(40.0, 0.25), (51.0, 0.25), (50.0, 0.25), (50.2, 0.25), (30.1, 0.25), (50.0, 5.0), (54.0, 5.0)] {
        let path = square_with_hole((left, 40.0), (left + 20.0, 60.0));
        let want = (100.0 - 2.0 * d) * (100.0 - 2.0 * d) - (20.0 + 2.0 * d) * (20.0 + 2.0 * d);
        let (tried, plain) = inset_area(&path, d);
        assert!(close(tried, want, 1e-6) && close(plain, want, 1e-6), "hole from x {left}, inset by {d}: {tried} and {plain}, not {want}");
    }
}

/// Slits through the middle of the square, from narrow to wide, centred on it or with an edge through
/// it: the middle of the outer contour's widest span is in the slit or on its edge at every width.
#[test]
fn an_inset_keeps_a_face_with_a_slit_through_its_middle() {
    let d = 0.25;
    let slits = [0.01, 0.1, 1.0, 10.0].into_iter().flat_map(|w| [(50.0 - w / 2.0, w), (50.0, w)]);
    for (left, w) in slits {
        let path = square_with_hole((left, 20.0), (left + w, 80.0));
        let want = (100.0 - 2.0 * d) * (100.0 - 2.0 * d) - (w + 2.0 * d) * (60.0 + 2.0 * d);
        let (tried, plain) = inset_area(&path, d);
        assert!(close(tried, want, 1e-6) && close(plain, want, 1e-6), "slit {w} wide from x {left}: {tried} and {plain}, not {want}");
    }
}

/// A grown offset keeps a hole whose own widest span runs through an island in it: a 100 x 100 square
/// with a 60 x 60 hole, and in the hole a bar 30 x 2 across its middle, grown by 3. (A 10 x 10 island
/// there was kept: the middle of the span is 5 inside it, further than the offset.)
#[test]
fn an_outset_keeps_a_hole_whose_widest_span_runs_through_an_island() {
    for (island, d) in [
        (((45.0, 45.0), (55.0, 55.0)), 1.0),
        (((45.0, 45.0), (55.0, 55.0)), 3.0),
        (((35.0, 49.0), (65.0, 51.0)), 3.0),
        (((35.0, 49.0), (65.0, 51.0)), 1.0),
    ] {
        let mut path = square_with_hole((20.0, 20.0), (80.0, 80.0));
        rect(&mut path, island.0, island.1, false);
        let (w, h) = (island.1.0 - island.0.0, island.1.1 - island.0.1);
        let hole = (60.0 - 2.0 * d) * (60.0 - 2.0 * d) - (w + 2.0 * d) * (h + 2.0 * d);
        let want = (100.0 + 2.0 * d) * (100.0 + 2.0 * d) - hole;
        let grown = area(&try_offset_path(&path, d, Join::Miter, 4.0).expect("an outset"), NZ);
        assert!(close(grown, want, 1e-6), "island {w} x {h}, grown by {d}: {grown}, not {want}");
    }
}

/// The outline of the open subpaths that an inset adds keeps a hole of the outline whose widest span
/// runs through an island of it: an open path round a square 100 across that overshoots its start,
/// and an open bar across the middle, inset by 1. A point between them, in the hole, is outside.
#[test]
fn the_outline_of_open_subpaths_keeps_a_hole_whose_widest_span_runs_through_an_island() {
    let mut path = BezPath::new();
    path.move_to((0.0, 0.0));
    for q in [(100.0, 0.0), (100.0, 100.0), (0.0, 100.0), (0.0, -10.0)] {
        path.line_to(q);
    }
    path.move_to((35.0, 50.0));
    path.line_to((65.0, 50.0));
    let outline = try_offset_path(&path, -1.0, Join::Miter, 4.0).expect("an outline");
    assert!(outline.winding(Point::new(25.0, 25.0)) == 0, "the hole between the ring and the bar is filled");
    assert!(outline.winding(Point::new(50.0, 50.0)) != 0 && outline.winding(Point::new(50.0, 0.5)) != 0);
}
