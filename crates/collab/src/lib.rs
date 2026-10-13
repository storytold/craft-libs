//! Shared editing for the Craft apps.
//!
//! The relay assigns a strictly increasing sequence to every edit, and each client applies edits
//! in that order. Concurrent work in different regions proceeds freely. Concurrent work in the
//! same region is serialised by a scope lock. That is deliberately short of a CRDT, which would
//! require stable identity for positions and a rewrite of each app's document model.
//!
//! Three pieces:
//!
//! * [`proto`] is the wire protocol, two enums and nothing else.
//! * [`Session`] orders, deduplicates and tracks locks. It performs no IO and returns [`Effect`]
//!   values describing what the app should do.
//! * [`Hub`] is the transport seam a caller implements, intended for Socket.IO at both ends.
//!
//! A command is a name plus opaque JSON, matching the `CommandSpec` shape every Craft app already
//! has, and a lock scope is an opaque string each app reads its own way. So nothing here is tied
//! to one app. GridCraft is the easiest first mover, because its commands already carry their own
//! cell address while WordCraft's act on the local caret.
//!
//! ```
//! use craft_collab::{Effect, Session};
//! use serde_json::json;
//!
//! let mut s = Session::new("doc-42");
//! let _join = s.join("Alice");
//! // …the relay answers, the app feeds messages in, and acts on the effects it gets back.
//! let _ = s.submit("cell.set", json!({"cell": "B2", "input": "7"}), Some("B2".into()));
//! assert_eq!(s.inflight(), 1);
//! # let _ = Effect::Notice { detail: String::new() };
//! ```
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

mod error;
mod hub;
pub mod proto;
mod session;

/// A [`Hub`] over `rust_socketio`, for native apps.
///
/// Behind a feature, because `rust_socketio` depends on `native-tls` and cannot
/// build for `wasm32`. A browser build leaves it off.
#[cfg(feature = "native-hub")]
pub mod native;

#[cfg(test)]
mod tests;

pub use error::{Error, Result};
pub use hub::{Hub, decode, encode};
pub use proto::{ClientMsg, DocId, LockState, PeerId, Presence, Scope, ServerMsg};
pub use session::{Effect, Session};
