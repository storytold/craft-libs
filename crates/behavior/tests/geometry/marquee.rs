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

#[test]
fn fixed_ratio_fits_the_canvas_from_the_clamped_anchor() {
    let mut gesture = MarqueeGesture::press(pos2(0.0, 0.0), Point::new(-40.0, 100.0), Mods::NONE);
    gesture.size_to(Point::new(50.0, 120.0), false, Constraint::Ratio(16.0, 9.0));
    assert_eq!(gesture.ratio_pixels(16.0, 9.0, [400, 300]), kurbo::Rect::new(0.0, 100.0, 50.0, 129.0));
    gesture.size_to(Point::new(80.0, 120.0), true, Constraint::Ratio(16.0, 9.0));
    assert_eq!(gesture.ratio_pixels(16.0, 9.0, [400, 300]), kurbo::Rect::new(0.0, 100.0, 80.0, 145.0));
}

#[test]
fn centred_square_fits_the_shorter_canvas_dimension_after_rounding() {
    let mut gesture = MarqueeGesture::press(pos2(0.0, 0.0), Point::new(50.4, 20.5), Mods::NONE);
    gesture.follow(None, Mods::ALT);
    gesture.size_to(Point::new(200.0, 100.0), false, Constraint::Ratio(1.0, 1.0));
    let r = gesture.ratio_pixels(1.0, 1.0, [100, 41]);
    assert_eq!(r.width(), r.height());
    assert_eq!((r.y0, r.y1), (0.0, 41.0));
    assert!(r.x0 >= 0.0 && r.x1 <= 100.0);
}
