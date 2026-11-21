#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]
// On wasm32 `MaybeSendSync` has no bound, because browser engines hold JS handles and the page has one
// thread. An `Arc` of such a type is correct there. Native builds keep the lint.
#![cfg_attr(target_arch = "wasm32", allow(clippy::arc_with_non_send_sync))]

//! Plugin host for craft-abi 1.
//!
//! - [`engine`]: the engine traits (`Engine`, `Module`, `Instance`), budgets, traps, the
//!   `Clock` the host injects, the package loader, and the engines.
//! - [`services`]: the trait the app implements (commands, document, log, notices, undo scope).
//! - [`dispatch`]: the host side of `craft_host.call`.
//! - [`invoke`]: one loaded generation and the guest call path.
//! - [`budget`]: budget classes and the circuit breaker.
//! - [`store`]: installed packages, grants, and the `state.*` store.
//! - [`lifecycle`] and [`contrib`]: install, reload, and command, hook, panel and format
//!   registration (native only).
//! - `link`: a Craft Link client that implements the link seam of [`services`] (feature `link`,
//!   native only).
//! - `shim`: adapters for the v1 plugin ABIs of EffectCraft, PhotoCraft and VectorCraft (feature
//!   `legacy-v1`).
//! - [`missing`]: the document side record of a plugin that is not installed.

pub mod budget;
pub mod dispatch;
pub mod engine;
pub mod invoke;
pub mod missing;
pub mod params;
pub mod process;
pub mod services;
#[cfg(feature = "legacy-v1")]
pub mod shim;
pub mod store;
pub mod version;

#[cfg(not(target_arch = "wasm32"))]
pub mod contrib;
#[cfg(not(target_arch = "wasm32"))]
pub mod lifecycle;
#[cfg(all(feature = "link", not(target_arch = "wasm32")))]
pub mod link;

#[cfg(all(feature = "link", target_arch = "wasm32"))]
compile_error!("feature `link` needs local sockets and does not build for wasm32");
