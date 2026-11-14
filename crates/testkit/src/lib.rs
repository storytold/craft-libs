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
//! * `image` (feature `image`): RGBA8 and gray views, difference statistics, PSNR for `u8` and
//!   `u16` planes, tolerances with an optional 3×3 neighborhood, a difference image, PNG reading
//!   and writing with size limits, golden reference checks and `assert_similar!`.
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

pub mod guards;
#[cfg(feature = "image")]
pub mod image;

pub use guards::{AsFileSource, Finding, predefined_quit, std_instant, wgsl_float_literals, wgsl_float_literals_in};

#[cfg(not(target_arch = "wasm32"))]
pub use guards::{SourceLimits, rust_sources, rust_sources_with_limits};
