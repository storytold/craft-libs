#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]
#![forbid(unsafe_code)]
//! Shared test support for the Craft apps. Test support only: unpublished, and used as a
//! dev-dependency.
//!
//! [`serial`] holds [`serial::TestLock`], a lock that a `static` can hold and that the thread
//! holding it can take again, and [`serial::gpu_lock`], the one lock every GPU test of a process
//! takes. It is native only.

#[cfg(not(target_arch = "wasm32"))]
pub mod serial;
