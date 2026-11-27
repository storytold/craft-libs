//! Cursor chat: a short message next to your pointer that others see (N5.09).

/// Seconds a bubble stays after its last change (FigCraft `live.rs:76`).
pub const CHAT_HOLD_S: f64 = 5.0;
/// Seconds it takes to fade out after that (`live.rs:77`).
pub const CHAT_FADE_S: f64 = 1.0;
/// A cursor chat message and when it last changed (monotonic seconds).
#[derive(Clone, Debug, PartialEq)]
pub struct CursorChat {
    #[allow(missing_docs)]
    pub text: String,
    /// Monotonic time, in seconds, of the last change.
    pub changed: f64,
}

/// The bubble's opacity `age` seconds after its last change: 1 for [`CHAT_HOLD_S`], then down to
/// 0 over [`CHAT_FADE_S`]; with `reduce_motion` it stays 1 until the fade would end, then 0.
// figcraft: crates/ui-egui/src/live.rs:214 `chat_fade`.
pub fn chat_alpha(age: f64, reduce_motion: bool) -> f32 {
    if age < CHAT_HOLD_S {
        1.0
    } else if age >= CHAT_HOLD_S + CHAT_FADE_S {
        0.0
    } else if reduce_motion {
        1.0
    } else {
        (1.0 - (age - CHAT_HOLD_S) / CHAT_FADE_S) as f32
    }
}
