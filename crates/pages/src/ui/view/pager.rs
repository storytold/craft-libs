//! The wheel's page turn in a single-page view.

use craft_behavior::{Wheel, WheelUnit};

/// The turn this pass's wheel asks for: `Some(true)` forward, `Some(false)` back. `at_edge` is the edge the view is
/// scrolled to (`Some(true)` the end, `Some(false)` the start, `None` neither) and `notch` the scroll a notch counts.
///
/// The share's `gathered` is the whole gesture's scroll in the view's points, so the rule needs nothing of its own
/// between passes. A negative scroll moves down the document, toward the next page. Line input turns each time the
/// gathered scroll reaches a multiple of `notch` in the share's direction, so two half notches turn once they add up
/// and a whole notch back turns back. Point input (a swipe and its momentum) turns once, when the gathered scroll
/// reaches `notch` either way, and turns back when it comes back a whole notch from there, to where the gesture
/// started. A turn needs the view at the edge it moves past, and a gesture that moved the view (the canvas marks it,
/// `Pointer::mark_wheel_moved`) does not turn: the pan that brought a long page to its edge does not run on into the
/// next one.
pub(crate) fn wheel_turn(wheel: &Wheel, at_edge: Option<bool>, notch: f64) -> Option<bool> {
    if !(notch.is_finite() && notch > 0.0) {
        return None;
    }
    let after = f64::from(wheel.gathered.y);
    let before = after - f64::from(wheel.scroll.y);
    if !(after.is_finite() && before.is_finite()) || after == before {
        return None;
    }
    let forward = after < before;
    // The levels as counts of notches along the share's direction: the share reaches levels `from + 1 ..= to`.
    let along = if forward { -1.0 } else { 1.0 };
    let (from, to) = ((along * before / notch).floor(), (along * after / notch).floor());
    let reached = match wheel.unit {
        WheelUnit::Line => to > from,
        // Level 1 is a whole notch past the start, the turn; level 0 is the start, the way back from a turn the other
        // way. A swipe reaches no other level, so it turns once however far it runs.
        _ => to > from && from <= 0.0 && to >= 0.0,
    };
    (reached && at_edge == Some(forward) && !wheel.moved).then_some(forward)
}
