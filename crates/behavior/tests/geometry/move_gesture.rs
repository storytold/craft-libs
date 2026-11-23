//! The direction lock of the move gesture.

use craft_behavior::geometry::{Directions, lock_direction};
use kurbo::Vec2;

fn polar(deg: f64, len: f64) -> Vec2 {
    let a = deg.to_radians();
    Vec2::new(len * a.cos(), len * a.sin())
}

#[test]
fn lock_direction_keeps_the_axis_until_eight_degrees_past_half() {
    let axis = lock_direction(Vec2::new(10.0, 0.0), None, Directions::Eight);
    assert_eq!(axis, Vec2::new(10.0, 0.0));
    // 22.5 + 8 = 30.5 degrees is the switch.
    let kept = lock_direction(polar(30.0, 10.0), Some(axis), Directions::Eight);
    assert_eq!(kept.y, 0.0);
    assert!(kept.x > 0.0);
    let switched = lock_direction(polar(31.0, 10.0), Some(axis), Directions::Eight);
    assert!((switched.y - switched.x).abs() < 1e-9 && switched.y > 0.0);
}
