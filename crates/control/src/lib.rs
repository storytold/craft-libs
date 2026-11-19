//! The control channel any app may share (docs/control-protocol.md): the per-launch token and its
//! files ([`auth`]), the loopback server that admits a connection by the token and hands each
//! request to the window ([`server`]), the client that talks to it ([`client`]), the request the
//! window answers ([`ControlRequest`]), the document watch subscribed connections wait on
//! ([`DocumentWatch`]), what a request still running does when it finishes ([`Effect`]), and AI
//! agents in the open window behind the ArtCraft Launcher's switch ([`agents`]: its settings on
//! every target, the switch, the connection file and the CLI's side with feature `agents`). It
//! needs no UI toolkit; the window's own requests over egui (`ui.screenshot`, `ui.resize`) are
//! `craft-control-egui`.
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]
#![forbid(unsafe_code)]

pub mod agents;
pub mod auth;
mod cancel;
#[cfg(not(target_arch = "wasm32"))]
pub mod client;
mod effect;
mod request;
#[cfg(not(target_arch = "wasm32"))]
pub mod server;

pub use cancel::{Cancel, Ticket};
pub use effect::Effect;
pub use request::{ControlRequest, ControlResponse, DOCUMENT_WATCH, DocumentWatch, Subscriber, cancelled_chat_response, chat_turn_cancelled};

/// How an error from the client (`client`, native only) starts when a request is still running
/// in the app after the wait. Outside the client so callers on every target can test for it.
pub const STILL_RUNNING: &str = "still running:";
