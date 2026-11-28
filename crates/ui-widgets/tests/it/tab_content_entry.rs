//! Regression coverage for tab content entry.

use craft_ui::tabs::{ContentEntry, ContentEntryFrame};
use craft_ui::testing::trial;
use egui::Id;

const SETTLED: ContentEntryFrame = ContentEntryFrame { progress: 1.0, direction: 0, moving: false };
#[trial]
fn selection_direction_interruption_and_initial_appearance() {
    let mut entry = ContentEntry::default();
    let a = Id::unique("a");
    let b = Id::unique("b");
    let c = Id::unique("c");
    assert_eq!(entry.update(a, 0, 0.0, 1.0, true), SETTLED);
    assert_eq!(entry.update(b, 1, 1.0, 1.0, true), ContentEntryFrame { progress: 0.0, direction: 1, moving: true });
    let mid = entry.update(b, 1, 1.5, 1.0, true);
    assert!(mid.progress > 0.0 && mid.progress < 1.0);
    assert_eq!(entry.update(c, 0, 1.5, 1.0, true).direction, -1);
    assert_eq!(entry.update(c, 0, 3.0, 1.0, true), SETTLED);
    assert_eq!(entry.update(c, 20, 4.0, 1.0, true), SETTLED);
}
#[trial]
fn motion_off_and_hostile_clocks_settle_without_replay() {
    let mut entry = ContentEntry::default();
    let a = Id::unique("a");
    let b = Id::unique("b");
    entry.update(a, 0, 0.0, 1.0, true);
    entry.update(b, 1, 1.0, 1.0, true);
    assert_eq!(entry.update(b, 1, 1.1, 1.0, false), SETTLED);
    assert_eq!(entry.update(b, 1, 1.2, 1.0, true), SETTLED);
    for duration in [f32::NAN, f32::INFINITY, -1.0, 0.0] {
        assert_eq!(entry.update(a, 0, 2.0, duration, true), SETTLED);
    }
    assert_eq!(entry.update(b, 1, f64::NAN, 1.0, true), SETTLED);
    entry.update(a, 0, 2.0, 1.0, true);
    assert_eq!(entry.update(b, 1, 1.0, 1.0, true), SETTLED);
}
