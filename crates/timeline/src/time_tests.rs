//! Tick, TimeRange and TimeView.

use crate::time::{TICKS_PER_SECOND, Tick, TimeView};
use craft_ui::testing::trial;

fn secs(s: i64) -> Tick {
    Tick(s * TICKS_PER_SECOND)
}

#[trial]
fn zoom_about_keeps_the_anchor_under_the_pointer() {
    let mut view = TimeView::new(secs(10), 40.0);
    let anchor = secs(15);
    let x = view.x_of(anchor, 100.0);
    view.zoom_about(2.0, anchor, 1000.0);
    assert_eq!(view.points_per_second, 80.0);
    assert!((view.x_of(anchor, 100.0) - x).abs() < 0.01);
    assert_eq!(view.time_at(x, 100.0), anchor);

    // The zoom stops at its limits.
    view.zoom_about(1e9, anchor, 1000.0);
    assert_eq!(view.points_per_second, TimeView::MAX_ZOOM);
    view.zoom_about(1e-12, anchor, 1000.0);
    assert_eq!(view.points_per_second, TimeView::MIN_ZOOM);

    // An anchor outside the view zooms about the middle; the start never goes below zero.
    let mut view = TimeView::new(secs(10), 40.0);
    view.zoom_about(2.0, secs(500), 1000.0);
    assert!((view.x_of(secs(500), 0.0) - 500.0).abs() < 0.01);
    let mut view = TimeView::new(Tick::ZERO, 40.0);
    view.zoom_about(0.5, secs(1), 1000.0);
    assert!(view.start >= Tick::ZERO);
}
