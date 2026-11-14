#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]
#![forbid(unsafe_code)]
//! Test support shared by the Craft apps and the craft-libs crates. Apps and craft-libs crates
//! take it under `[dev-dependencies]`.
//!
//! [`bin`] finds the executable of one of the package's binaries from an integration test, also
//! when the test runs from a relocated nextest archive ([`bin_path!`]), and runs it with a limit on
//! the bytes it may write to stdout and stderr ([`bin::run`]). It is native only.

#[cfg(not(target_arch = "wasm32"))]
pub mod bin;
