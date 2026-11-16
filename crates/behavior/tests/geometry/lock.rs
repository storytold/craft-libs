//! The ⇧ rules for a drag from a fixed point.

use craft_behavior::geometry::{axis_or_diagonal, turn_to_45};
use kurbo::{Point, Vec2};

#[test]
fn shift_turns_a_point_onto_the_nearest_45_degree_ray_keeping_its_distance() {
    let from = Point::new(10.0, 10.0);
    let point = from + kurbo::Vec2::from_angle(40f64.to_radians()) * 20.0;
    let turned = turn_to_45(from, point);
    let angle = (turned - from).atan2().to_degrees();
    assert!((angle - 45.0).abs() < 1e-9, "on the 45° ray, got {angle}");
    assert!(((turned - from).hypot() - 20.0).abs() < 1e-9, "the distance is kept");
    assert_eq!(turn_to_45(from, from), from);
}

/// A drag of `(dx, dy)` from `from`, reflected into each octant: the rule must not depend on the direction.
fn octants(dx: f64, dy: f64) -> [Vec2; 8] {
    [(dx, dy), (dy, dx), (-dx, dy), (-dy, dx), (dx, -dy), (dy, -dx), (-dx, -dy), (-dy, -dx)].map(|(x, y)| Vec2::new(x, y))
}

#[test]
fn shift_snaps_a_line_to_the_axis_within_30_degrees_and_to_the_diagonal_past_it() {
    let from = Point::new(10.0, 10.0);
    // 26.6° from the axis: onto the axis, keeping the offset along it.
    for d in octants(40.0, 20.0) {
        let snapped = axis_or_diagonal(from, from + d) - from;
        let along = if d.x.abs() > d.y.abs() { Vec2::new(d.x, 0.0) } else { Vec2::new(0.0, d.y) };
        assert_eq!(snapped, along, "{d:?} lies within 30° of an axis");
    }
    // 39.8° from the axis: onto the diagonal at the shorter offset, both signs kept.
    for d in octants(60.0, 50.0) {
        let snapped = axis_or_diagonal(from, from + d) - from;
        assert_eq!(snapped, Vec2::new(50f64.copysign(d.x), 50f64.copysign(d.y)), "{d:?} is past 30° from the axes");
    }
    // An anchor at a 5:12 slope, with the noise a stored path carries, stays on the axis.
    assert_eq!(axis_or_diagonal(from, from + Vec2::new(60.0, -25.0 + 5e-5)), Point::new(70.0, 10.0));
    assert_eq!(axis_or_diagonal(from, from), from, "no offset, no change");
}
