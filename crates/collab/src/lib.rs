//! # craft-collab — the generalized collaboration core
//!
//! The pure, network-free heart of `CraftHub`, extracted and generalized from the proven
//! `figcraft-collab` core (spec §2.15): every tested semantic is kept, the `FigCraft`-specific
//! document knowledge is lifted behind [`adapter::DocSchema`] so any Craft app can adapt its
//! document to the same object/property model (GOPM — spec §3.4/§4.6).
//!
//! The module map mirrors the figcraft crate one-to-one (spec §2.15 — "the same module has its
//! test file"):
//!
//! | module | role |
//! |--------|------|
//! | [`change`] | the [`Change`] algebra: Create/Set/Delete/SetDoc/Put/Remove |
//! | [`ids`]    | client-scoped 64-bit [`NodeId`] allocation without coordination (§5.1) |
//! | [`frac`]   | fractional indexing sibling keys (§5.2) |
//! | [`value`]  | property values: JSON, asset refs (SHA-256), RGA text (§6.6) |
//! | [`replica`]  | the synchronized state: ordered children, doc entries, apply (strict/lenient) |
//! | [`diff`]     | Arc-diff between two document states → a change batch (§5.3) |
//! | [`server`]   | the pure server state machine: validate, accept with seq, or refuse with fixes (§5.4) |
//! | [`client`]   | optimistic client state: unconfirmed batches, reconciliation, per-user undo (§5.5/§5.6) |
//! | [`journal`]  | append-only persistence of accepted batches + client counters (§6.3) |
//! | [`code`]     | human-readable join codes: Crockford base-32, stretching, constant-time secrets (§6.4) |
//! | [`presence`] | ephemeral presence: cursors, selections, viewports, app projections (§6.1) |
//! | [`signal`]   | transient signals: cursor chat and spotlight, never stored (§2.7) |
//! | [`text`]     | the RGA text property type for fine-grained concurrent writing (§6.6) |
//! | [`merge`]    | three-way merge and per-list-type merge strategies (§6.8, docs/merge-matrix.md) |
//! | [`limits`]   | every enforced bound, v2 values verbatim (§2.15) |
//! | [`adapter`]  | [`adapter::DocSchema`] and [`adapter::HubAdapter`] — the app integration contract |
//! | [`sim`]      | the N-client convergence simulator over memory links with random delays (§5.5) |
//!
//! The crate never touches sockets, files, clocks or randomness sources: all I/O lives behind
//! traits so the whole core is testable in memory (the family's layering rule, spec §14.5).
//!
//! Ported semantics are marked with the spec section they implement; every property test
//! references the P1..Pn guarantees of its section.

pub mod adapter;
pub mod change;
pub mod client;
pub mod code;
pub mod diff;
pub mod frac;
pub mod ids;
pub mod journal;
pub mod limits;
pub mod merge;
pub mod presence;
pub mod replica;
pub mod server;
pub mod signal;
pub mod sim;
pub mod text;
pub mod value;

pub use adapter::{DocSchema, HubAdapter};
pub use change::{Batch, BatchId, Change, EntryId, Position};
pub use ids::{ClientId, NodeId};
pub use limits::Limits;
pub use replica::{ApplyMode, Replica};
pub use server::{Outcome, Server, SessionState};
pub use value::{AssetRef, Value};

/// The core error classification (spec §2.15: keep the v2 categories, add `Unauthorized`/`Quota`).
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum Error {
    /// A node id that does not exist in the document.
    #[error("unknown object {0:?}")]
    Unknown(NodeId),
    /// A semantically invalid change (human-readable reason, translated client-side by key).
    #[error("invalid: {0}")]
    Invalid(String),
    /// Wire encoding or decoding failure.
    #[error("codec: {0}")]
    EncodeDecode(String),
    /// A message out of protocol order.
    #[error("protocol: {0}")]
    Protocol(String),
    /// Authenticated access denied (v3 extension — explicit, documented, non-breaking).
    #[error("unauthorized: {0}")]
    Unauthorized(String),
    /// A quota or rate limit was hit (v3 extension). Always carries the retry-after semantics.
    #[error("quota: {0}")]
    Quota(String),
}

/// Compare secrets (tokens, keys, their hashes) without stopping at the first difference.
///
/// The family's constant-time comparison, ported verbatim from figcraft-collab (spec §2.15);
/// every token, join-code and webhook-signature comparison in `CraftHub` goes through it.
#[must_use]
pub fn same_secret(a: &str, b: &str) -> bool {
    let (a, b) = (a.as_bytes(), b.as_bytes());
    a.len() == b.len() && a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn same_secret_is_constant_shape() {
        assert!(same_secret("k7m2qx9td", "k7m2qx9td"));
        assert!(!same_secret("k7m2qx9td", "k7m2qx9te"));
        assert!(!same_secret("short", "longer-string"));
        // A prefix of the secret never matches.
        assert!(!same_secret("abcdef", "abc"));
    }
}
