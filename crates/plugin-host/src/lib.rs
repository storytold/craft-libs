#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]
// On wasm32 `MaybeSendSync` has no bound, because browser engines hold JS handles and the page has one
// thread. An `Arc` of such a type is correct there. Native builds keep the lint.
#![cfg_attr(target_arch = "wasm32", allow(clippy::arc_with_non_send_sync))]

//! Plugin engines for craft-abi 1.
//!
//! [`engine`] holds the engine traits (`Engine`, `Module`, `Instance`), budgets, traps, the
//! `Clock` the host injects, the package loader, and the engines: wasmi (default), wasmtime
//! (feature `wasmtime`, native only) and the browser engines (feature `browser`, wasm32 only).

pub mod engine;
