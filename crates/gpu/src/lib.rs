//! GPU traits, job envelope, and provider registry for Craft apps and the web runtime.
//!
//! Application crates register [`GpuProvider`] implementations; generic hosts dispatch
//! [`GpuJob`] values by [`GpuJob::provider_id`] without importing app project types.
//!
//! With the `startup` feature, the GPU start-up choices per platform for apps on eframe's wgpu backend: which backends
//! to try, the power preference, the DX12 shader compiler and, on Linux, which DRM device to render on. An app calls
//! `configure` on the `WgpuSetupCreateNew` it hands to eframe.

#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

mod caps;
mod chunk;
mod context;
#[cfg(feature = "startup")]
mod device;
mod error;
mod job;
mod provider;
mod readback;
mod registry;

#[cfg(test)]
mod tests;

#[cfg(test)]
#[cfg(feature = "startup")]
mod device_tests;

pub use caps::DeviceCaps;
pub use chunk::chunk_buffer;
pub use context::{GpuContext, ReadOutcome, ReadRequest, WorkerKind};
#[cfg(feature = "startup")]
pub use device::configure;
pub use error::GpuFault;
pub use job::{GpuJob, GpuJobOutcome, GpuJobParams, HandleId};
pub use provider::GpuProvider;
pub use readback::{f16_rgba_to_f32_bytes, f16_to_f32, f16_to_f32_fast};
pub use registry::ProviderRegistry;
