//! The direction lock of the move gesture.

use craft_behavior::geometry::{ConstrainStyle, Directions, MoveGesture, lock_direction};
use craft_keymap::Mods;
use kurbo::{Point, Vec2};

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

#[test]
fn a_shift_move_with_four_directions_locks_an_axis_and_frees_it_when_shift_is_let_go() {
    let mut drag = MoveGesture::press(&ConstrainStyle::new(4.0, 4.0), Point::ZERO, Mods::SHIFT, Directions::Four);
    let at = Point::new(10.0, 3.0);
    let locked = drag.follow(at, Mods::SHIFT, 1.0).expect("the move changed the value");
    assert_eq!(locked.delta.y, 0.0, "Shift holds the offset to the horizontal axis: {:?}", locked.delta);
    assert!(locked.delta.x > 0.0, "{:?}", locked.delta);
    assert!(drag.locked().is_some(), "the lock is remembered while Shift is down");
    let free = drag.follow(at, Mods::NONE, 1.0).expect("letting Shift go changed the value");
    assert_eq!(free.delta, Vec2::new(10.0, 3.0), "without Shift the offset is free");
    assert_eq!(drag.locked(), None, "the lock is forgotten when Shift is let go");
}
