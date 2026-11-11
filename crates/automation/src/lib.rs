#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::unreachable)]

//! Drive a running Craft app from tests, agents and the CLI, and record and play actions.
//!
//! [`actions`] and [`ui::actions`] build on every target: the recorded-action list, its player
//! and the Actions panel. The rest is desktop only.
//!
//! A transport thread ([`server`], or a channel in tests) turns each JSON line into a
//! [`control::ControlRequest`]; the UI thread drains them between frames through
//! [`control::ControlChannel`], runs the app's method table on each, and settles the
//! [`control::Reply`]: a reply now, a reply once injected input ([`synthetic`]) has been
//! processed, a reply once a background job ends, or a screenshot through the bounded queue
//! ([`screenshot`]). [`profiler`] and [`trace`] time frames for the channel's `perf` output.
//!
//! The crate never names a document: what `engine.execute`, `ui.inspect` or `ui.set` mean is the
//! app's, through its method table. Everything a method table does not want to own is here.

pub mod actions;
#[cfg(feature = "ui")]
pub mod ui;

#[cfg(not(target_arch = "wasm32"))]
pub mod args;
#[cfg(all(feature = "ui", not(target_arch = "wasm32")))]
pub mod control;
#[cfg(not(target_arch = "wasm32"))]
pub mod profiler;
#[cfg(not(target_arch = "wasm32"))]
pub mod protocol;
#[cfg(all(feature = "ui", not(target_arch = "wasm32")))]
pub mod screenshot;
#[cfg(not(target_arch = "wasm32"))]
pub mod security;
#[cfg(all(feature = "ui", not(target_arch = "wasm32")))]
pub mod synthetic;
#[cfg(not(target_arch = "wasm32"))]
pub mod trace;

#[cfg(not(target_arch = "wasm32"))]
pub mod client;
#[cfg(all(feature = "ui", not(target_arch = "wasm32")))]
pub mod server;

#[cfg(all(feature = "ui", not(target_arch = "wasm32")))]
pub use control::{ControlChannel, ControlRequest, ControlResponse, Dispatch, Reply};
#[cfg(not(target_arch = "wasm32"))]
pub use protocol::REPLY_DEADLINE;
#[cfg(all(feature = "ui", not(target_arch = "wasm32")))]
pub use screenshot::Captures;
#[cfg(not(target_arch = "wasm32"))]
pub use security::AutomationError;

/// The trials this crate registers, for its integration binary.
#[cfg(feature = "screen-testing")]
pub mod testing {
    craft_ui::testing_registered!();
}

#[cfg(all(test, feature = "screen-testing"))]
extern crate self as craft_automation;
#[cfg(all(test, feature = "screen-testing"))]
#[path = "../tests/it/main.rs"]
mod integration;

#[cfg(test)]
fn main() {
    craft_ui::testing::run(module_path!(), env!("CARGO_PKG_NAME"), &craft_ui::testing::registered_in(env!("CARGO_PKG_NAME")));
}
