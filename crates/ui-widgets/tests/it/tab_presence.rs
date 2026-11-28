//! Regression coverage for contextual tab presence.

use craft_ui::tabs::{TabPresence, TabPresenceFrame};
use craft_ui::testing::trial;
use egui::Id;
#[trial]
fn outgoing_tabs_reverse_without_duplicates_and_keep_their_last_position() {
    let mut presence = TabPresence::default();
    let a = Id::unique("table");
    let b = Id::unique("chart");
    assert_eq!(presence.update(&[(a, 2)], 0.0, 1.0, true)[0].visibility, 1.0);
    let frames = presence.update(&[(b, 3)], 1.0, 1.0, true);
    assert_eq!(frames.len(), 2);
    assert_eq!(frames[0].visibility, 0.0);
    assert_eq!(frames[1].position, 2);
    assert!(!frames[1].present);
    let frames = presence.update(&[(b, 3)], 1.5, 1.0, true);
    let outgoing = frames[1].visibility;
    assert!(outgoing > 0.0 && outgoing < 1.0);
    let frames = presence.update(&[(a, 4), (b, 3), (a, 9)], 1.5, 1.0, true);
    assert_eq!(frames.len(), 2);
    assert_eq!(frames[0].visibility, outgoing);
    assert!(frames[0].present);
    assert_eq!(frames[0].position, 4);
    assert!(presence.update(&[(a, 4)], 4.0, 1.0, true).iter().all(|f| f.visibility.is_finite()));
    presence.update(&[], 5.0, 1.0, true);
    let frames = presence.update(&[], 6.0, 1.0, true);
    assert!(frames.is_empty());
}
#[trial]
fn motion_off_invalid_parameters_and_rollback_drop_ghosts_and_settle() {
    let a = Id::unique("a");
    let b = Id::unique("b");
    for (now, duration, motion) in [(1.2, 1.0, false), (f64::NAN, 1.0, true), (1.2, f32::NAN, true), (1.2, -1.0, true), (0.5, 1.0, true)] {
        let mut presence = TabPresence::default();
        presence.update(&[(a, 0)], 0.0, 1.0, true);
        presence.update(&[(b, 1)], 1.0, 1.0, true);
        assert_eq!(
            presence.update(&[(b, 1)], now, duration, motion),
            vec![TabPresenceFrame { id: b, position: 1, visibility: 1.0, present: true, moving: false }]
        );
    }
}
