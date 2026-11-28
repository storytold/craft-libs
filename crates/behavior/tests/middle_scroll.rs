//! The middle button's auto-scroll latch.

use craft_behavior::{MiddleScroll, PointerButton, PressCancel, PressPhase, PressStart};
use emath::{Vec2, pos2};

/// One frame's elapsed time.
const FRAME: f64 = 1.0 / 60.0;

/// A click keeps the latch, as does a hold of one frame even when that frame runs a second pass; a hold of two frames
/// past the dead zone ends it on release.
#[test]
fn a_click_inside_the_dead_zone_latches_and_a_hold_past_it_ends_on_release() {
    let anchor = pos2(100.0, 100.0);
    let past = pos2(100.0, 140.0);

    let mut latch = MiddleScroll::default();
    latch.press(anchor, true);
    assert_eq!(latch.step(pos2(100.0, 105.0), true, FRAME, false), Vec2::ZERO, "inside the dead zone nothing scrolls");
    latch.release();
    assert!(latch.active(), "a click latches");
    assert_eq!(latch.anchor(), Some(anchor));
    let scroll = latch.step(past, false, FRAME, false);
    assert!(scroll.y < 0.0 && scroll.x == 0.0, "past the dead zone below the anchor the view scrolls down: {scroll:?}");
    latch.step(past, false, FRAME, false);
    latch.release();
    assert!(latch.active(), "passes with the button up count no hold");
    latch.press(anchor, true);
    assert!(!latch.active(), "a second press ends the latch");
    assert_eq!(latch.anchor(), None);

    // One frame held past the dead zone, rerun as a second pass with no elapsed time.
    latch.press(anchor, true);
    latch.step(past, true, FRAME, false);
    latch.step(past, true, 0.0, false);
    latch.release();
    assert!(latch.active(), "one held frame is a click, however many passes it runs");

    // A fresh latch starts its count over: one more held frame is still a click.
    latch.press(anchor, true);
    latch.press(anchor, true);
    latch.step(past, true, FRAME, false);
    latch.release();
    assert!(latch.active(), "the count starts over at each latch");

    latch.press(anchor, true);
    latch.press(anchor, true);
    latch.step(past, true, FRAME, false);
    latch.step(past, true, FRAME, false);
    latch.release();
    assert!(!latch.active(), "a hold of two frames past the dead zone ends on release");
    assert_eq!(latch.anchor(), None);
}

/// A primary click that ends the latch is swallowed from its press through its release; the next click is not.
#[test]
fn a_primary_click_that_ends_auto_scroll_is_swallowed_through_its_release() {
    let at = pos2(100.0, 100.0);
    let start = PressStart::new(at, PointerButton::Primary, 0.0, 1);
    let down = PressPhase::Down { start };
    let held = PressPhase::Held { start, pos: at };
    let release = PressPhase::Release { start, pos: at, click: true };

    let mut latch = MiddleScroll::default();
    latch.press(at, true);
    assert!(latch.swallows(down), "the press that ends the latch is swallowed");
    assert!(!latch.active());
    assert!(latch.swallows(held), "its later phases are too");
    assert!(latch.swallows(release), "through its release");
    assert!(!latch.swallows(down), "the next click, with nothing latched, is not");
    assert!(!latch.swallows(held));
    assert!(!latch.swallows(release));

    latch.press(at, true);
    assert!(latch.swallows(down));
    assert!(latch.swallows(PressPhase::Cancel { start, reason: PressCancel::Escape }));
    assert!(!latch.swallows(held), "a cancel ends the swallowed press");
}
