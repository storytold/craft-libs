//! Tiered storage for Craft web apps: synchronous mirror over async persistence.
//!
//! JS host hooks (OPFS, File System Access) live in `craft-runtime` wasm glue; this crate
//! holds the portable model and hot-tier LRU.

#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

mod cloud_ref;
mod coordinator;
mod external_volume;
mod key;
mod mirror;
mod policy;
mod versioned;

#[cfg(test)]
mod tests;

pub use cloud_ref::CloudBlobRef;
pub use coordinator::{DefaultTierCoordinator, TierCoordinator};
pub use external_volume::ExternalVolume;
pub use key::StoreKey;
pub use mirror::{MirrorStore, PendingWrite};
pub use policy::{DefaultTierPolicy, KeyClass, TierPolicy};
pub use versioned::VersionedMirror;
