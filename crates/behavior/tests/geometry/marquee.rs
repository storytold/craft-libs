//! The marquee gesture: square and centre count only after a release and a new press.

use craft_behavior::geometry::{Constraint, MarqueeGesture};
use craft_keymap::Mods;
use emath::pos2;
use kurbo::Point;

fn corners(gesture: &mut MarqueeGesture, to: Point) -> (Point, Point) {
    let rect = gesture.size_to(to, false, Constraint::Free);
    (rect.a, rect.b)
}

#[test]
fn shift_held_at_the_press_squares_only_after_a_release_and_press() {
    let origin = Point::new(0.0, 0.0);
    let mut gesture = MarqueeGesture::press(pos2(0.0, 0.0), origin, Mods::SHIFT);
    gesture.follow(Some(pos2(100.0, 50.0)), Mods::SHIFT);
    assert_eq!(corners(&mut gesture, Point::new(100.0, 50.0)), (origin, Point::new(100.0, 50.0)));
    gesture.follow(None, Mods::NONE);
    gesture.follow(None, Mods::SHIFT);
    assert_eq!(corners(&mut gesture, Point::new(100.0, 50.0)), (origin, Point::new(100.0, 100.0)));
}
