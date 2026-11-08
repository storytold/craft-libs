// Carved from VectorCraft's crates/pathops (tests/it/main.rs) at storytold/vectorcraft ff9deee46d1c59dc9ecef0b668c25e57cd80b55e.
// Copyright (c) 2026 ArtCraft Team and the VectorCraft contributors. MIT OR Apache-2.0.

//! Integration tests: one test binary for the crate, one module per area.
// Unwrapping and panicking on failure is fine here, unlike in shipped code.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

#[path = "../common/mod.rs"]
mod common;

mod hostile_params;
mod limits;
mod ops;
mod pathfinder_pinned;
mod prop_pathops;
mod props;
mod regressions;
mod review;
mod review2;
mod review3;
