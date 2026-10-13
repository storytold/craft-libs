//! The transport seam.
//!
//! A [`Hub`] is whatever carries the protocol. The crate never opens a connection, exactly as
//! `craft-nextcloud` never opens a socket, so one session implementation serves every target.
//!
//! The intended implementations are Socket.IO on both sides, which is where the Hub and
//! subscriber shape comes from. A native app drives `rust_socketio`. A browser build drives the
//! official `socket.io-client` through `wasm-bindgen`, because `rust_socketio` depends on
//! `native-tls` with no target-gated alternative and therefore cannot build for `wasm32`.
//!
//! Rooms do the subscribing. One room per document, which is how a relay fans an edit out to
//! exactly the peers holding that document open.

use crate::error::Result;
use crate::proto::{ClientMsg, ServerMsg};

/// A connection to the relay.
///
/// `send` is fire and forget. Ordering is the relay's job, and the session tolerates arrival out
/// of order, so an implementation needs no queue of its own.
///
/// `poll` is non-blocking and returns what has arrived since the last call. That suits an egui
/// app, which wants to drain messages once per frame rather than be called back on another
/// thread.
pub trait Hub {
    fn send(&mut self, msg: ClientMsg) -> Result<()>;

    /// Messages received since the previous call, oldest first.
    fn poll(&mut self) -> Vec<ServerMsg>;

    /// Whether the transport believes it is connected. A session stays usable across a brief
    /// disconnect, because the relay re-sends on rejoin.
    fn connected(&self) -> bool;
}

/// Encode a client message as the JSON a Socket.IO event carries.
pub fn encode(msg: &ClientMsg) -> Result<String> {
    serde_json::to_string(msg).map_err(|e| crate::error::Error::malformed("client", e.to_string()))
}

/// Decode one relay payload.
pub fn decode(payload: &str) -> Result<ServerMsg> {
    serde_json::from_str(payload).map_err(|e| crate::error::Error::malformed("server", e.to_string()))
}
