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
//! * `guards::rust_sources`: directory crawler collecting `.rs` sources while excluding tests, with
//!   size limits (`guards::rust_sources_with_limits`). It is not built for `wasm32`.
//!
//! On native targets, the `fixtures` module locates generated test fixtures under `<workspace>/target/fixtures`,
//! generates a missing one into a claimed temporary sibling, publishes it under a per-output lock that
//! keeps a complete file another process published first, and prints the `FIXTURE made`, `cached` and
//! `skipped` lines that FilmCraft's `cargo xtask fixtures` counts.
//!
//! [`rng`] holds two seeded generators for repeatable test input, [`rng::XorShift64`] and
//! [`rng::XorShift64Star`], each with `next`, `below`, `byte` and `fill`.
//!
//! On targets other than `wasm32`, `deadline::with_deadline` runs a test body on its own thread
//! and fails the test when the body still has no result once the limit has passed; a result that
//! arrives before that check is returned. `deadline::wait_until` calls a condition, and when a
//! `deadline::Signal` raise wakes it, checks the condition again until it holds; raises that come
//! during one call of the condition share one recheck. It fails the test when the condition still
//! does not hold at the limit. Both read an optional limit variable that the caller names,
//! `CRAFT_TEST_TIMEOUT_SECS` by default.
//!
//! [`hostile::mutations`] yields a fixed sequence of damaged copies of a valid seed, on every
//! target. On targets other than `wasm32`, `hostile::hammer` runs a parser on the seed, every
//! truncation of it and those copies, one after another on a worker thread with a time limit for
//! each, and fails the test with the case that panicked or did not finish.
//!
//! On targets other than `wasm32`, `serial` holds `serial::TestLock`, a lock that a `static`
//! can hold and that the thread holding it can take again, and `serial::gpu_lock`, the one lock
//! for GPU tests. Every GPU test in a process must take this lock, because a test that takes a
//! different lock can run at the same time as one that takes this one.
//!
//! `skip` decides what a test does when something it needs is missing (a program, a corpus,
//! the craft-fonts faces): it prints one `SKIPPED:` line and returns, or fails the test when the
//! variable the caller names is `1`. A value other than unset, empty or `1` fails the test as a
//! configuration error. `skip::required` reads such a variable under this rule for any caller.
//! It is not built for `wasm32`. `skip::craft_fonts_cover` needs the `font-coverage` feature.
//!
//! With the `corpus` feature, `corpus` (native only) parses pinned-corpus manifests and checks a
//! corpus folder against them: `corpus::Manifest`, `corpus::verify` and the test gate
//! `corpus::pinned`, which skips or fails under the rule of `skip`.
//!
//! With the `mcp` feature, on targets other than `wasm32`, `mcp::McpChild` runs an MCP server as a
//! child process and talks to it over stdin and stdout, with a deadline on every wait and a length
//! limit on every line it reads.
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
//!
//! ### Test fonts
//!
//! The [`fonts`] module makes test fonts from font bytes the caller supplies:
//! [`fonts::rename_family`] gives a face a family name of the same length, and
//! [`fonts::collection`] builds a TrueType collection from standalone faces.

#[cfg(all(feature = "corpus", not(target_arch = "wasm32")))]
pub mod corpus;
#[cfg(not(target_arch = "wasm32"))]
pub mod deadline;
#[cfg(not(target_arch = "wasm32"))]
pub mod fixtures;
pub mod fonts;
pub mod guards;
pub mod hostile;
#[cfg(all(feature = "mcp", not(target_arch = "wasm32")))]
pub mod mcp;
pub mod rng;
pub mod sample_damage;
#[cfg(not(target_arch = "wasm32"))]
pub mod serial;
#[cfg(not(target_arch = "wasm32"))]
pub mod skip;

#[cfg(not(target_arch = "wasm32"))]
pub use deadline::{DeadlineLimit, Signal, wait_until, with_deadline};
pub use guards::{AsFileSource, Finding, predefined_quit, std_instant, wgsl_float_literals, wgsl_float_literals_in};

#[cfg(not(target_arch = "wasm32"))]
pub use guards::{SourceLimits, rust_sources, rust_sources_with_limits};
