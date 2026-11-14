#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]
#![forbid(unsafe_code)]
//! Test support shared by the Craft apps and the craft-libs crates. Test support only: unpublished,
//! and used as a dev-dependency.
//!
//! [`deadline::with_deadline`] runs a test body on its own thread and fails the test when the body
//! does not finish within its limit. [`deadline::wait_until`] polls a condition until it holds and
//! fails the test when the limit passes first. Both read an optional limit variable that the caller
//! names, `CRAFT_TEST_TIMEOUT_SECS` by default. The module is not built for `wasm32`.

#[cfg(not(target_arch = "wasm32"))]
pub mod deadline;

#[cfg(not(target_arch = "wasm32"))]
pub use deadline::{DeadlineLimit, wait_until, with_deadline};
