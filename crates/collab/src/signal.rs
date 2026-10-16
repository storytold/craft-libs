//! Transient signals (spec §2.7, §6.1): cursor chat and spotlight.
//!
//! Signals are presence, not document: they never enter the document, the journal or any saved
//! state. The server forwards them only to clients that announced the matching feature word,
//! cleans them (unknown kinds ignored, over 1 KiB dropped, chat trimmed to 100 characters with
//! control characters turned to spaces), and rate-limits at 20 signals/second on average
//! (batches of 40) — excess is dropped without disconnecting.

use crate::ids::ClientId;
use serde::{Deserialize, Serialize};

/// The signal kinds the protocol defines (extensible at the end only — the v2 discipline).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum SignalKind {
    /// Cursor chat: what someone is typing in their bubble (≤100 chars, control chars → spaces).
    Chat,
    /// Spotlight: "on" (Some) / off (None); a newer spotlight takes over; leaving ends it.
    Spotlight,
}

/// One signal on the wire.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Signal {
    /// Who signalled.
    pub from: ClientId,
    /// The kind.
    pub kind: SignalKind,
    /// The payload (chat text or "on").
    pub text: String,
}

/// The hard bounds (v2 values, spec §2.7/§2.12).
pub const MAX_SIGNAL_BYTES: usize = 1024;
/// Chat text is trimmed to this many characters.
pub const MAX_CHAT_CHARS: usize = 100;
/// The average rate: 20 signals/second (burst 40).
pub const RATE_PER_SECOND: u32 = 20;
/// The burst allowance.
pub const BURST: u32 = 40;

/// Clean a chat signal (control characters become spaces, then trimmed).
#[must_use]
pub fn clean_chat(text: &str) -> String {
    text.chars().map(|c| if c.is_control() { ' ' } else { c }).take(MAX_CHAT_CHARS).collect()
}

/// Whether a serialized signal fits the wire budget.
#[must_use]
pub fn fits(signal: &Signal) -> bool {
    postcard::to_allocvec(signal).is_ok_and(|v| v.len() <= MAX_SIGNAL_BYTES)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chat_is_cleaned_and_trimmed() {
        let long: String = "a".repeat(300);
        assert_eq!(clean_chat(&long).len(), MAX_CHAT_CHARS);
        assert_eq!(clean_chat("hi\u{1b}[0m"), "hi [0m");
    }

    #[test]
    fn oversize_signals_do_not_fit() {
        let s = Signal { from: ClientId(1), kind: SignalKind::Chat, text: "x".repeat(2_000) };
        assert!(!fits(&s));
    }
}
