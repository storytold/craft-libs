//! The submenu pointer triangle, checked without a frame.

use craft_ui::testing::internals::pointer_heads_toward;
use craft_ui::testing::trial;
use egui::{Rect, pos2};

#[trial]
fn the_pointer_heads_for_the_submenu_inside_the_triangle_to_its_near_edge() {
    let submenu = Rect::from_min_max(pos2(100.0, 0.0), pos2(200.0, 100.0));
    let from = pos2(0.0, 50.0);
    assert!(pointer_heads_toward(from, pos2(50.0, 50.0), submenu));
    assert!(pointer_heads_toward(from, pos2(50.0, 30.0), submenu));
    assert!(!pointer_heads_toward(from, pos2(50.0, 200.0), submenu));
    assert!(!pointer_heads_toward(from, pos2(-20.0, 50.0), submenu));
}
