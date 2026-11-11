//! Who is in a shared document, and where their pointer is (N5.07).

use super::chat::CursorChat;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
/// Identifies a person within a session.
pub struct ParticipantId(pub u64);

/// The colours people get in join order (FigCraft `crates/engine/src/sharing/live/types.rs:27`).
pub const PEOPLE_COLORS: [[u8; 3]; 8] = [
    [0x0D, 0x99, 0xFF],
    [0x97, 0x47, 0xFF],
    [0x14, 0xAE, 0x5C],
    [0xF2, 0x48, 0x22],
    [0xD9, 0x73, 0x0D],
    [0xE8, 0x3E, 0x8C],
    [0x00, 0xA3, 0xA3],
    [0x66, 0x5C, 0xE0],
];

/// How fast a remote cursor glides to its reported position, per second (FigCraft `live.rs:71`).
pub const GLIDE: f64 = 20.0;

/// One person in the session.
#[derive(Clone, Debug, PartialEq)]
pub struct Participant {
    #[allow(missing_docs)]
    pub id: ParticipantId,
    #[allow(missing_docs)]
    pub name: String,
    /// The colour of their cursor and avatar, straight RGB.
    pub color: [u8; 3],
    /// Their pointer in document units, `None` off the canvas.
    pub cursor: Option<[f64; 2]>,
    /// The outlines of their selection, document units (at most 500 are drawn).
    pub selection: Vec<[[f64; 2]; 2]>,
    /// Their cursor chat bubble, if one is open.
    pub chat: Option<CursorChat>,
    /// They view only (no edits).
    pub viewer: bool,
    /// They are spotlighting (asking everyone to follow).
    pub spotlight: bool,
}

/// Up to two letters for an avatar: the first letters of the first two words, alphanumeric
/// ones preferred, upper case; "?" for none.
// figcraft: crates/engine/src/sharing/live.rs:354 `initials`.
pub fn initials(name: &str) -> String {
    let firsts = |alphanumeric: bool| -> String {
        name.split_whitespace().filter_map(|word| word.chars().next()).filter(|c| !alphanumeric || c.is_alphanumeric()).take(2).collect()
    };
    let mut letters = firsts(true);
    if letters.is_empty() {
        letters = firsts(false);
    }
    if letters.is_empty() { "?".into() } else { letters.to_uppercase() }
}

/// The next drawn position of a cursor shown at `shown` heading for `target`, `dt` seconds later
/// (clamped to 0…0.1): the gap closes by `1 − e^(−GLIDE·dt)`; with `reduce_motion` it jumps.
// figcraft: crates/ui-egui/src/live.rs:319 (in `draw`).
pub fn glide(shown: [f64; 2], target: [f64; 2], dt: f64, reduce_motion: bool) -> [f64; 2] {
    if reduce_motion {
        return target;
    }
    let closed = 1.0 - (-dt.clamp(0.0, 0.1) * GLIDE).exp();
    [shown[0] + (target[0] - shown[0]) * closed, shown[1] + (target[1] - shown[1]) * closed]
}
