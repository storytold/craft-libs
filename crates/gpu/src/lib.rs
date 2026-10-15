//! GPU traits, job envelope, and provider registry for Craft apps and the web runtime.
//!
//! Application crates register [`GpuProvider`] implementations; generic hosts dispatch
//! [`GpuJob`] values by [`ProviderId`] without importing app project types.

#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

mod caps;
mod chunk;
mod readback;
mod context;
mod error;
mod job;
mod provider;
mod registry;

#[cfg(test)]
mod tests;

pub use caps::DeviceCaps;
pub use chunk::chunk_buffer;
pub use readback::{f16_rgba_to_f32_bytes, f16_to_f32, f16_to_f32_fast};
pub use context::{GpuContext, WorkerKind};
pub use error::GpuFault;
pub use job::{GpuJob, GpuJobOutcome, GpuJobParams, HandleId};
pub use provider::GpuProvider;
pub use registry::ProviderRegistry;
