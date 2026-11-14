//! Conflict-free replicated types for Craft documents.
//!
//! // CRDT for concurrent document merge (catalog slug `crdt`)
//! Two people can edit the same list or the same paragraph without taking a lock, and when
//! their copies meet they converge. CollabCraft still owns rooms, tokens, presence, and
//! the socket relay. This crate is the merge math: no sockets, no files, no clock.
//!
//! | module | role |
//! |--------|------|
//! | [`change`] | the [`Change`] algebra: Create/Set/Delete/SetDoc/Put/Remove |
//! | [`ids`]    | client-scoped 64-bit [`NodeId`] allocation without coordination |
//! | [`frac`]   | fractional indexing sibling keys |
//! | [`value`]  | property values: JSON, asset refs, RGA text |
//! | [`replica`]  | the synchronized state: ordered children, doc entries, apply |
//! | [`diff`]     | Arc-diff between two document states to a change batch |
//! | [`server`]   | in-crate test oracle: validate, accept with seq, or refuse. Not the live relay |
//! | [`text`]     | the RGA text property type for concurrent writing |
//! | [`merge`]    | three-way merge and per-list-type merge strategies |
//! | [`limits`]   | every enforced bound |
//! | [`sim`]      | N-client convergence over memory links with random delays |
//!
//! Apps talk to `craft_collab::Session`. They apply [`text::TextOp`] values locally. The
//! in-crate [`server::Server`] exists so [`sim`] can prove replicas converge; CollabCraft
//! is what runs on the wire.

pub mod change;
pub mod diff;
pub mod frac;
pub mod ids;
pub mod limits;
pub mod merge;
pub mod replica;
pub mod server;
pub mod sim;
pub mod text;
pub mod value;

pub use change::{Batch, BatchId, Change, EntryId, Position};
pub use ids::{ClientId, NodeId};
pub use limits::Limits;
pub use replica::{ApplyMode, Replica};
pub use server::{Outcome, Server, SessionState};
pub use text::{TextDoc, TextOp};
pub use value::{AssetRef, Value};

/// The core error classification.
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
    /// Authenticated access denied.
    #[error("unauthorized: {0}")]
    Unauthorized(String),
    /// A quota or rate limit was hit. Always carries the retry-after semantics.
    #[error("quota: {0}")]
    Quota(String),
}
