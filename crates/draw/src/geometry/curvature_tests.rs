//! Tests of the Curvature Pen's handles.

use craft_ui::testing::trial;
use kurbo::Point;

use super::curvature::curvature_handles;

fn p(x: f64, y: f64) -> Point {
    Point::new(x, y)
}

// An inner anchor's handles are collinear, parallel to the neighbours' chord, and a third of the
// distance to each neighbour long (uneven spacing).
#[trial]
fn curvature_handles_are_collinear_and_one_third_of_the_neighbour_distance() {
    let a = [p(0.0, 0.0), p(30.0, 40.0), p(120.0, 40.0), p(150.0, 0.0)];
    let h = curvature_handles(&a, &[], false);
    assert_eq!(h.len(), 4);
    let (i, o) = h[1];
    let chord = a[2] - a[0];
    let cross = |u: kurbo::Vec2, v: kurbo::Vec2| u.x * v.y - u.y * v.x;
    assert!(cross(o - a[1], chord).abs() < 1e-9);
    assert!(cross(i - a[1], chord).abs() < 1e-9);
    assert!((a[1].distance(i) - a[1].distance(a[0]) / 3.0).abs() < 1e-9);
    assert!((a[1].distance(o) - a[1].distance(a[2]) / 3.0).abs() < 1e-9);
    assert!((a[1] - i).dot(o - a[1]) > 0.0);
}

// `corners[i]` keeps anchor i's handles on the anchor; one or two anchors give straight segments,
// open or closed.
