#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

//! Craft Link transport.
//!
//! Local sockets, discovery files, and the JSON-lines session. The server and
//! the client are separate modules. There is no tokio runtime in this crate.

pub mod activate;
pub mod asset;
pub mod auth;
pub mod bulk;
pub mod cache;
pub mod client;
pub mod discovery;
pub mod error;
pub mod fetch;
pub mod ids;
pub mod launch;
pub mod moniker;
pub mod payload;
pub mod record;
pub mod refresh;
pub mod server;
pub mod trust;
pub mod watch;
pub mod wire;

/// Unit tests that hold a discovery lock take `read`. Unit tests that start a
/// child process take `write`. A forked child owns a copy of every open file
/// until it calls exec, and a `flock` lock lives until the last copy closes.
/// Without this guard a lock released by `drop` can look held for a few
/// milliseconds, and `scan` or `publish` in another test then fails at random.
#[cfg(test)]
pub(crate) static SPAWN_GUARD: std::sync::RwLock<()> = std::sync::RwLock::new(());

pub use auth::{ensure_peer_user, generate_token, random_uuid, tokens_equal};
pub use discovery::{Discovery, DocInfo, Endpoint, InstanceRecord, Protocol, RuntimeDir};
pub use error::{ErrorObject, LinkError};
