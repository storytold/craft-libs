//! Regression coverage for sliding tab underlines.

use craft_ui::tabs::{SlidingUnderline, UnderlineFrame};
use craft_ui::testing::trial;
use egui::{Id, Rect};
fn rect(x: f32) -> Rect {
    Rect::from_min_size(egui::pos2(x, 30.0), egui::vec2(40.0, 2.0))
}
fn frame(state: &mut SlidingUnderline, id: &str, x: f32, now: f64, motion: bool) -> UnderlineFrame {
    let result = state.update(Id::unique(id), rect(x), now, 1.0, motion);
    assert!(result.is_some(), "valid target rejected");
    result.unwrap_or(UnderlineFrame { rect: Rect::NOTHING, moving: false })
}
#[trial]
fn interrupted_selection_starts_at_the_displayed_marker_and_eventually_settles() {
    let mut state = SlidingUnderline::default();
    assert_eq!(frame(&mut state, "a", 0.0, 0.0, true).rect, rect(0.0));
    assert_eq!(frame(&mut state, "b", 100.0, 1.0, true).rect, rect(0.0));
    let midway = frame(&mut state, "b", 100.0, 1.5, true);
    assert!(midway.moving && midway.rect.left() > 0.0 && midway.rect.left() < 100.0);
    assert_eq!(frame(&mut state, "c", 200.0, 1.5, true).rect, midway.rect);
    assert_eq!(frame(&mut state, "c", 200.0, 3.0, true), UnderlineFrame { rect: rect(200.0), moving: false });
}
#[trial]
fn resize_motion_off_and_clock_reset_snap_without_leaving_stale_motion() {
    let mut state = SlidingUnderline::default();
    frame(&mut state, "a", 0.0, 0.0, true);
    frame(&mut state, "b", 100.0, 1.0, true);
    assert!(!frame(&mut state, "b", 120.0, 1.1, true).moving);
    assert_eq!(frame(&mut state, "c", 200.0, 1.2, false).rect, rect(200.0));
    assert!(!frame(&mut state, "d", 300.0, 0.0, true).moving);
    assert!(!frame(&mut state, "e", 400.0, f64::NAN, true).moving);
    assert!(state.update(Id::unique("bad"), Rect::NOTHING, 1.0, 1.0, true).is_none());
    assert!(!frame(&mut state, "f", 500.0, 2.0, true).moving);
}
