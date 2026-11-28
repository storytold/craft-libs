#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]
#![cfg(not(target_arch = "wasm32"))]

//! Native GPU execution for material graphs. The host supplies an existing wgpu device,
//! queue and a memory-budget callback, then installs the accelerator on a graph evaluation context.
//!
//! Metal uses the reference evaluator for packed affine transforms and nonzero directional warps
//! because GPU sample positions can cross texel boundaries. Other operations can still run on GPU.

mod gpu_graph;
mod gpu_graph_color;
mod gpu_graph_filter;
mod gpu_graph_grey;
mod gpu_graph_half;
mod gpu_graph_patterns;
mod validation;

#[cfg(test)]
mod headless;

pub use gpu_graph::GraphAccelerator;
