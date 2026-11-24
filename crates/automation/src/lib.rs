#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::unreachable)]

//! Drive a running Craft app from tests, agents and the CLI, and record and play actions.
//!
//! [`actions`] and [`ui::actions`] build on every target: the recorded-action list, its player
//! and the Actions panel; so do the agent settings in [`agents`] and, with `ui`, the control
//! channel, its screenshots, synthetic input and the access-tree lookups (a web page feeds
//! [`control::ControlChannel`] from its own request queue). The server, client, token files,
//! profiler and trace are desktop only.
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
#![warn(missing_docs)]

pub mod actions;
pub mod agents;
#[cfg(feature = "ui")]
pub mod ui;

#[cfg(feature = "ui")]
pub mod access;
#[cfg(not(target_arch = "wasm32"))]
pub mod args;
#[cfg(feature = "ui")]
pub mod control;
#[cfg(feature = "ui")]
pub mod dialogs;
#[cfg(all(feature = "ui", not(target_arch = "wasm32")))]
pub mod dock;
#[cfg(feature = "ui")]
pub mod files;
#[cfg(not(target_arch = "wasm32"))]
pub mod profiler;
pub mod protocol;
#[cfg(feature = "ui")]
pub mod screenshot;
#[cfg(not(target_arch = "wasm32"))]
pub mod security;
#[cfg(feature = "ui")]
pub mod synthetic;
#[cfg(not(target_arch = "wasm32"))]
pub mod trace;
#[cfg(all(feature = "ui", not(target_arch = "wasm32")))]
pub mod window;

#[cfg(not(target_arch = "wasm32"))]
pub mod client;
#[cfg(all(feature = "ui", not(target_arch = "wasm32")))]
pub mod server;

#[cfg(feature = "ui")]
pub use control::{ControlChannel, ControlRequest, ControlResponse, Dispatch, Reply};
pub use protocol::REPLY_DEADLINE;
#[cfg(feature = "ui")]
pub use screenshot::Captures;
#[cfg(not(target_arch = "wasm32"))]
pub use security::AutomationError;

/// The trials this crate registers, for its integration binary.
#[cfg(feature = "screen-testing")]
pub mod testing {

    #[cfg(feature = "suite")]
    craft_ui::registry!();
}

#[cfg(all(test, feature = "screen-testing"))]
extern crate self as craft_automation;
#[cfg(all(test, feature = "screen-testing"))]
#[path = "../tests/it/main.rs"]
pub(crate) mod integration;

#[cfg(test)]
fn main() {
    craft_ui::registry!();
    craft_ui::testing::run(module_path!(), env!("CARGO_PKG_NAME"), &craft_ui::testing::Registry::of(&[register]));
}
