//! Claims made in the seventh review of the pull request, each as the smallest test that fails when
//! the claim is true.

use craft_pathops::*;
use kurbo::{BezPath, Point, Rect, Shape as _};

use crate::common::{rectangle, rel_close as close};

const NZ: FillRule = FillRule::NonZero;

// ---- the inset's boundary

/// A line out and back that the sweep cancels, whose middle lies on another edge of the boundary, is
/// no part of the boundary: a square 1e8 across with a hole 0.3 above its bottom edge, and a spur from
/// the bottom edge up 0.6 and back, through the strip under the hole and into it. Its middle is on the
/// hole's edge, and its length (1.2) is within what the lengths of a square that size can tell apart;
/// stroked, it cut a notch through the strip's inset.
#[test]
fn an_inset_does_not_stroke_a_cancelled_line_whose_middle_lies_on_another_edge() {
    let mut path = BezPath::new();
    path.move_to((5e7, 0.0));
    for q in [(5e7, 0.6), (5e7, 0.0), (1e8, 0.0), (1e8, 1e8), (0.0, 1e8), (0.0, 0.0)] {
        path.line_to(q);
    }
    path.close_path();
    // The hole, wound the other way.
    path.move_to((5e7 - 10.0, 0.3));
    for q in [(5e7 - 10.0, 20.0), (5e7 + 10.0, 20.0), (5e7 + 10.0, 0.3)] {
        path.line_to(q);
    }
    path.close_path();
    let inset = try_offset_path(&path, -0.1, Join::Round, 4.0).expect("an inset");
    // In the strip under the hole, 0.15 from both its edges: in the inset, and on the spur's line.
    assert!(inset.winding(Point::new(5e7 + 5.0, 0.15)) != 0, "the strip's inset");
    assert!(inset.winding(Point::new(5e7, 0.15)) != 0, "a notch is cut round the spur");
}

// ---- interior points

/// A rectangle whose sides are adjacent numbers has no point strictly inside it: the middle of the gap
/// between its two crossings rounds onto one of them, an edge, which the winding can count as inside.
#[test]
fn an_interior_point_is_never_a_crossing() {
    for x0 in [1.0f64, 1e6, 12345.678] {
        let x1 = x0.next_up();
        let thin = rectangle(Rect::new(x0, 0.0, x1, 1.0));
        let got = interior_point(&thin);
        assert!(got.is_none_or(|p| p.x > x0 && p.x < x1), "at {x0}: {got:?}, on a side");
    }
    // A rectangle with room between its sides still has its point.
    let wide = rectangle(Rect::new(1.0, 0.0, 1.0 + 1e-12, 1.0));
    let p = interior_point(&wide).expect("a point");
    assert!(p.x > 1.0 && p.x < 1.0 + 1e-12);
}

// ---- areas

/// `area` refuses what `try_area` refuses: a path with a loop of one cubic whose handle is no number,
/// or so large that its products overflow, is no area (0), not the area of the rest of the path.
#[test]
fn the_area_of_a_path_try_area_refuses_is_nothing() {
    let square = rectangle(Rect::new(0.0, 0.0, 10.0, 10.0));
    for handle in [f64::NAN, 1e200] {
        let mut path = square.clone();
        path.move_to((20.0, 0.0));
        path.curve_to((handle, 0.0), (20.0, 10.0), (20.0, 0.0));
        path.close_path();
        assert!(try_area(&path, NZ).is_err());
        assert_eq!(area(&path, NZ), 0.0, "with a handle of {handle}");
    }
    assert!(close(area(&square, NZ), 100.0, 1e-12));
}
