//! Tests of artboard rectangles.

use craft_ui::testing::trial;
use kurbo::{Rect, Vec2};

use super::artboard::resized_rect;

// `resized_rect` of (0, 0, 100, 100) with the left handle dragged by 150 gives (100, 0, 150, 100);
// dragged by exactly 100 it is 1 px wide; ⌥ moves the right edge the other way.
#[trial]
fn artboard_resize_past_opposite_edge_flips_with_min_one_pixel() {
    let r = Rect::new(0.0, 0.0, 100.0, 100.0);
    assert_eq!(resized_rect(r, [-1, 0], Vec2::new(150.0, 0.0), false), Rect::new(100.0, 0.0, 150.0, 100.0));
    assert_eq!(resized_rect(r, [-1, 0], Vec2::new(100.0, 0.0), false), Rect::new(100.0, 0.0, 101.0, 100.0));
    assert_eq!(resized_rect(r, [-1, 0], Vec2::new(20.0, 0.0), true), Rect::new(20.0, 0.0, 80.0, 100.0));
    assert_eq!(resized_rect(r, [1, 1], Vec2::new(10.0, -10.0), true), Rect::new(-10.0, 10.0, 110.0, 90.0));
}

// `drawn_rect` and `moved_rect` round to whole pixels and order the corners.
