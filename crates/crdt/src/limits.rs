//! Every enforced bound, with the v2 default values verbatim (spec §2.15).
//!
//! v3 only adds fields and never lowers a value without an ADR — every change to a limit is a
//! documented decision. The server enforces these; the decoders use them as allocation ceilings.

use serde::{Deserialize, Serialize};

/// The enforced limits of a collaboration session/file (v2 defaults kept literally).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Limits {
    /// Maximum size of one client message (the `Welcome` carrying a whole document is larger).
    pub max_message_bytes: usize,
    /// Maximum changes in one batch (one user action = one undo step).
    pub max_changes: usize,
    /// Maximum nodes in a document.
    pub max_nodes: usize,
    /// Maximum peers in one file.
    pub max_peers: usize,
    /// Maximum display-name length in characters, control characters stripped.
    pub max_name: usize,
    /// Batches kept in the catch-up log.
    pub max_log_batches: usize,
    /// `Changes` kept in the catch-up log.
    pub max_log_changes: usize,
}

impl Default for Limits {
    fn default() -> Self {
        // The v2 values, verbatim (spec §2.15 — `Limits` with its literal default values).
        Self {
            max_message_bytes: 64 * 1024 * 1024,
            max_changes: 500_000,
            max_nodes: 5_000_000,
            max_peers: 256,
            max_name: 64,
            max_log_batches: 10_000,
            max_log_changes: 1_000_000,
        }
    }
}

impl Limits {
    /// Sanitize a display name: strip control characters, cap at [`Limits::max_name`].
    ///
    /// Spec §2.12: "names are 64 characters without control characters" — a rule so small it is
    /// still a binding, tested requirement (spec §0.1 rule 10).
    #[must_use]
    pub fn clean_name(&self, name: &str) -> String {
        name.chars().filter(|c| !c.is_control()).take(self.max_name).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_match_v2() {
        let l = Limits::default();
        assert_eq!(l.max_message_bytes, 64 * 1024 * 1024);
        assert_eq!(l.max_changes, 500_000);
        assert_eq!(l.max_nodes, 5_000_000);
        assert_eq!(l.max_peers, 256);
        assert_eq!(l.max_name, 64);
        assert_eq!(l.max_log_batches, 10_000);
        assert_eq!(l.max_log_changes, 1_000_000);
    }

    #[test]
    fn names_lose_control_chars_and_length() {
        let l = Limits::default();
        assert_eq!(l.clean_name("sara\u{7}"), "sara");
        let long: String = "x".repeat(100);
        assert_eq!(l.clean_name(&long).len(), 64);
    }
}
