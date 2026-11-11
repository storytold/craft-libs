//! The ⇧ rules for a drag from a fixed point.

use craft_behavior::geometry::turn_to_45;
use kurbo::Point;

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
