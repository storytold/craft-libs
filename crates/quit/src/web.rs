//! Holding a browser tab open while work is unsaved.

/// Keeps one `beforeunload` listener that calls `prevent_default` while the last call said
/// `unsaved`. Call it each frame with whether work is at risk.
pub fn hold_leave(unsaved: bool) {
    let _ = unsaved;
    todo!("SH-C30")
}
