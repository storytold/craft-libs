//! The one-press-per-pass rule: a pass carries at most one press, as its last pointer event.

/// Splits `events` after the first press. The first part is what this pass receives, up to and including the
/// press; the second holds every later event, in order, for the front of the next frame's events. So a release
/// comes at least one pass after its press.
pub fn split_after_first_press<E>(mut events: Vec<E>, is_press: impl Fn(&E) -> bool) -> (Vec<E>, Vec<E>) {
    let held = match events.iter().position(is_press) {
        Some(at) => events.split_off(at + 1),
        None => Vec::new(),
    };
    (events, held)
}
