//! The wheel's page turn in a single-page view.

/// The turn this pass's wheel asks for: `Some(true)` forward, `Some(false)` back. `at_edge` is the edge the view is
/// scrolled to (`Some(true)` the end, `Some(false)` the start, `None` neither) and `notch` the scroll a notch counts.
#[allow(dead_code)] // P3(pdf-canvas-w4): CV-C22
pub(crate) fn wheel_turn(wheel: &craft_behavior::Wheel, at_edge: Option<bool>, notch: f64) -> Option<bool> {
    let _ = (wheel, at_edge, notch);
    todo!("CV-C22")
}
