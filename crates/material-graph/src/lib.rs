// SPDX-License-Identifier: MIT OR Apache-2.0
//! Material graphs parsed from text and evaluated to float RGBA images. The CPU evaluator provides a reference
//! implementation. Hosts can supply an [`accelerator::Accelerator`] to evaluate supported operations on a GPU.
//! Callers provide per-evaluation settings through [`context::EvalContext`] and allocate their own [`graph::NodeCache`].
//! The graph text format is documented in [`docs/format.md`](../docs/format.md).
#![forbid(unsafe_code)]
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing, clippy::unimplemented, clippy::todo, clippy::unreachable)]
pub mod accelerator;
pub mod atomic;
pub mod checksum;
pub mod compiled_ops;
pub mod context;
pub mod curve;
pub mod deflate;
pub mod expr;
pub mod exr;
pub mod font;
pub mod func;
pub mod fxmap;
pub mod graph;
pub mod grunge;
pub mod half;
pub mod lcg_rand;
pub mod math;
pub mod measured;
pub mod names;
pub mod nodes2;
pub mod noise;
pub mod ops;
pub mod par;
pub mod params;
pub mod patterns;
pub mod png;
pub mod profile;
pub mod scatter;
pub mod tex;
pub mod vector;
pub mod versions;

/// The data-parallel library the engine runs on, for hosts that build an [`context::EvalContext::pool`].
pub use rayon;

#[cfg(test)]
mod rule_tables;
