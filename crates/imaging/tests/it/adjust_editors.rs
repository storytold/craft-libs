//! The slider-based adjustment editors, driven through the public API as a host would show them: one binding per
//! editor, one gesture per slider drag, the Set of a check box, a preset or a button, and the requests the colour wells
//! file.

use craft_imaging::ui::adjust::{AdjustSession, HueRangeValue};
use craft_ui::testing::trial;

// ---------------------------------------------------------------------------------------------
// Hue/Saturation

/// Dragging a range thumb across 0 degrees keeps the four bounds increasing.
#[trial]
fn range_thumbs_wrap_through_zero() {
    // The Magentas' end thumb dragged from 345 to 20 degrees.
    let moved = HueRangeValue::move_bound(HueRangeValue::neutral(5).bounds, 3, 20.0);
    assert_eq!(moved, [255.0, 285.0, 315.0, 380.0]);
    // The Reds' start thumb dragged from 315 (-45) to 340 degrees, past the second thumb's room.
    let moved = HueRangeValue::move_bound(HueRangeValue::neutral(0).bounds, 0, 340.0);
    assert_eq!(moved, [-20.0, -15.0, 15.0, 45.0]);
    assert!(moved.windows(2).all(|pair| pair[0] <= pair[1]));
}

// ---------------------------------------------------------------------------------------------
// The slider table and the session

/// Tracked, edited, Reset: the start settings; Reset again: the defaults; a pass without tracking or another target
/// takes the committed settings anew.
#[trial]
fn reset_returns_to_the_values_found_when_edited_else_defaults() {
    let mut session = AdjustSession::new();
    session.track(7, 1, &10);
    session.track(7, 2, &20);
    assert_eq!(session.start(), None, "nothing is offered before an edit");
    session.edited();
    session.track(7, 3, &30);
    assert_eq!(session.start(), Some(&10), "following passes keep the settings found first");
    assert_eq!(session.reset(|| 0), 10, "edited: the start settings");
    assert!(!session.is_edited());
    assert_eq!(session.reset(|| 0), 0, "a second Reset gives the defaults");

    session.edited();
    session.track(7, 9, &90);
    assert!(!session.is_edited());
    session.edited();
    assert_eq!(session.start(), Some(&90));

    session.track(8, 10, &5);
    assert!(!session.is_edited());
    session.edited();
    assert_eq!(session.start(), Some(&5));
}
