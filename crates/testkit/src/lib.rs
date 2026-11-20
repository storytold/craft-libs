#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]
#![forbid(unsafe_code)]

//! Source-level platform regression guards and verification utilities for the Craft application suite.
//!
//! This crate provides non-panicking source analysis guards that detect known platform hazards
//! and lifecycle regressions across Craft applications:
//!
//! * [`guards::predefined_quit`]: flags direct calls to `muda`'s predefined Quit item that bypass
//!   dirty document save prompts and window teardown on macOS.
//! * [`guards::wgsl_float_literals`]: flags WGSL float literals above the finite range of `f32`
//!   (or of `f16` for an `h` suffix), which browser WebGPU compilers reject.
//! * [`guards::std_instant`]: flags uses of `std::time::Instant` in code paths targeting the web,
//!   where `std::time::Instant::now()` panics on `wasm32`.
//! * [`guards::rust_sources`]: directory crawler collecting `.rs` sources while excluding tests, with
//!   size limits ([`guards::rust_sources_with_limits`]). It is not built for `wasm32`.
//!
//! On targets other than `wasm32`, [`deadline::with_deadline`] runs a test body on its own thread
//! and fails the test when the body does not finish within its limit, and [`deadline::wait_until`]
//! polls a condition until it holds and fails the test when the limit passes first. Both read an
//! optional limit variable that the caller names, `CRAFT_TEST_TIMEOUT_SECS` by default.
//!
//! [`hostile::mutations`] yields a fixed sequence of damaged copies of a valid seed, on every
//! target. On targets other than `wasm32`, [`hostile::hammer`] runs a parser on the seed, every
//! truncation of it and those copies, one after another on a worker thread with a time limit for
//! each, and fails the test with the case that panicked or did not finish.
//!
//! ### Epistemic Scope: What Source Guards Establish vs What They Cannot
//!
//! Source-level guards provide deterministic syntactic evidence about codebase structure.
//! They establish that checked source files do not contain forbidden symbol names, known invalid
//! literal patterns, or unsafe platform invocations.
//!
//! They do **not** establish runtime correctness, protocol concurrency safety, or platform behavior
//! under foreign host environments. They are lightweight syntactic regression gates designed to complement,
//! rather than replace, platform integration tests and CI execution.

#[cfg(not(target_arch = "wasm32"))]
pub mod deadline;
pub mod guards;
pub mod hostile;
pub mod sample_damage;

#[cfg(not(target_arch = "wasm32"))]
pub use deadline::{DeadlineLimit, wait_until, with_deadline};
pub use guards::{AsFileSource, Finding, predefined_quit, std_instant, wgsl_float_literals, wgsl_float_literals_in};

#[cfg(not(target_arch = "wasm32"))]
pub use guards::{SourceLimits, rust_sources, rust_sources_with_limits};
