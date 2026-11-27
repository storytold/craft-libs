#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

//! The Craft apps' co-editing wire contract (add-in protocol 2).
//!
//! Apps that edit one document together exchange three kinds of payload through a transport
//! add-in: document updates, state vectors and awareness. This crate is what every app must do
//! the same way for two of them to meet in one room:
//!
//! - the limits and the base64 of a payload ([`MAX_UPDATE`], [`MAX_AWARENESS`], [`b64_encode`],
//!   [`b64_decode`]);
//! - [`crypto`]: end-to-end encryption, bound to the room and the method, so a message can't be
//!   replayed into another room or as another kind;
//! - [`awareness`]: who is in the room and what each client says about itself.
//!
//! The shared document itself (which shared types hold a workbook, a text or a deck) is each
//! app's schema and stays with the app, as do transports, processes and the UI.

pub mod awareness;
pub mod crypto;

/// The wire format apps announce (`host.collab.formats`).
pub const FORMAT: &str = "yjs-v1";
/// Largest update sent or applied, before base64 (protocol 2, section 7).
pub const MAX_UPDATE: usize = 16 << 20;
/// Largest awareness update.
pub const MAX_AWARENESS: usize = 64 << 10;

/// Why a collab operation failed. A bad payload is reported and dropped.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CollabError {
    /// The update (or state vector, or awareness message) can't be decoded or applied.
    Corrupt(String),
    /// The room holds another app's document, or another schema of this app's.
    Mismatch {
        app: String,
        schema: u64,
    },
    /// Decryption failed: wrong key, tampering, or the message was meant for another room.
    Decrypt,
    /// Larger than the protocol allows.
    TooLarge(usize),
    /// The room's state hasn't arrived yet.
    NotReady,
    Other(String),
}

impl std::fmt::Display for CollabError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CollabError::Corrupt(m) => write!(f, "a damaged update was dropped: {m}"),
            CollabError::Mismatch { app, schema } => write!(f, "This room is for {app} {schema}"),
            CollabError::Decrypt => write!(f, "a message that can't be decrypted was dropped"),
            CollabError::TooLarge(n) => write!(f, "a message of {n} bytes is larger than the 16 MiB allowed"),
            CollabError::NotReady => write!(f, "the room's document hasn't arrived yet"),
            CollabError::Other(m) => write!(f, "{m}"),
        }
    }
}

impl std::error::Error for CollabError {}

/// Standard base64 with padding (protocol 2's binary payloads).
pub fn b64_encode(bytes: &[u8]) -> String {
    use base64::Engine;
    base64::engine::general_purpose::STANDARD.encode(bytes)
}

/// Decodes standard base64 (padding optional).
pub fn b64_decode(text: &str) -> Result<Vec<u8>, CollabError> {
    use base64::Engine;
    let t = text.trim();
    base64::engine::general_purpose::STANDARD
        .decode(t)
        .or_else(|_| base64::engine::general_purpose::STANDARD_NO_PAD.decode(t.trim_end_matches('=')))
        .map_err(|e| CollabError::Corrupt(format!("base64: {e}")))
}

/// Fills `buf` with random bytes from the operating system (in the browser, its
/// `crypto.getRandomValues`).
pub fn random_bytes(buf: &mut [u8]) -> Result<(), CollabError> {
    getrandom::fill(buf).map_err(|e| CollabError::Other(format!("no random numbers: {e}")))
}

/// A fresh client id: random, below 2^53 (JavaScript-safe), never 0.
pub fn new_client_id() -> Result<u64, CollabError> {
    let mut b = [0u8; 8];
    random_bytes(&mut b)?;
    Ok((u64::from_le_bytes(b) & ((1u64 << 53) - 1)).max(1))
}

#[cfg(test)]
mod tests {
    use super::{b64_decode, b64_encode, new_client_id};

    #[test]
    fn base64_reads_what_it_writes_with_or_without_padding() {
        for bytes in [&b""[..], b"a", b"ab", b"abc", &[0, 255, 254, 3]] {
            let text = b64_encode(bytes);
            assert_eq!(b64_decode(&text).unwrap(), bytes);
            assert_eq!(b64_decode(text.trim_end_matches('=')).unwrap(), bytes);
            assert_eq!(b64_decode(&format!("  {text}\n")).unwrap(), bytes);
        }
        assert!(b64_decode("not base64!").is_err());
    }

    #[test]
    fn client_ids_fit_javascript_and_are_never_zero() {
        for _ in 0..1_000 {
            let id = new_client_id().unwrap();
            assert!((1..(1u64 << 53)).contains(&id));
        }
    }
}
