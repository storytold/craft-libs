//! Image filters on premultiplied RGBA8 images on the GPU, with a CPU reference of the same
//! filters, for layer effects in drawing and animation apps.
//!
//! - [`Filter`] is one filter: a separable [`Filter::GaussianBlur`], per-channel lookup tables on
//!   straight colour ([`Filter::Lut`], levels and curves style), or a premultiplied source-over
//!   [`Filter::Composite`] with an opacity.
//! - [`FilterProvider`] is the `craft.filters` [`craft_gpu::GpuProvider`]: a host registers it in a
//!   [`craft_gpu::ProviderRegistry`] and dispatches [`FilterJob`]s, on the null context or on a
//!   device.
//! - [`GpuFilters`] runs the filters as wgpu compute passes on a device the caller holds (native
//!   only).
//! - [`cpu`] has the same filters on the CPU: the reference the passes are tested against.
//!
//! Images are `width × height` pixels, tightly packed, red first, premultiplied by alpha. A colour
//! channel above its pixel's alpha is read as the alpha. Sides run from 1 to [`MAX_SIDE`] and an
//! image holds at most [`MAX_PIXELS`] pixels; a provider also holds the filter's largest buffer to
//! the context's buffer limits. Out-of-range parameters and sizes are errors, never panics.

#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

pub mod cpu;
#[cfg(not(target_arch = "wasm32"))]
mod gpu;
mod job;
mod spec;

#[cfg(not(target_arch = "wasm32"))]
pub use gpu::GpuFilters;
pub use job::{FilterJob, FilterProvider, PROVIDER_ID};
pub use spec::{Error, Filter, Lut, MAX_PIXELS, MAX_SIDE, MAX_SIGMA, kernel};

#[cfg(test)]
mod job_tests;
#[cfg(all(test, not(target_arch = "wasm32")))]
mod oracle_tests;
#[cfg(test)]
mod tests;
