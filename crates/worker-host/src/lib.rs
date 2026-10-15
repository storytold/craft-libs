//! Worker message protocol (Rust side). JS bootstrap: `assets/craft-worker-host.js`.

#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

mod protocol;

#[cfg(test)]
mod tests;

pub use protocol::{HostMessage, WorkerMessage};
