//! RGBA to 4:2:0 Y'CbCr on the GPU, for encoders and scopes.
//!
//! - [`YuvProvider`] is the `craft.yuv` [`craft_gpu::GpuProvider`]: a host registers it in a
//!   [`craft_gpu::ProviderRegistry`] and dispatches [`YuvJob`]s, on the null context or on a device.
//! - [`Converter`] holds the render pipelines of one [`Conversion`]: a [`Format`] (NV12, P010 or
//!   I420), a [`Matrix`] (BT.601, BT.709 or BT.2020) and a [`Range`] (limited or full), for a
//!   caller that records the passes into its own command encoder.
//! - [`Converter::bind`] binds a source texture view once; [`Converter::planes`] makes the plane
//!   textures of one output size; [`Converter::draw`] records the two passes. The source is scaled
//!   to the output size.
//! - [`Planes::copy_to_buffer`] records the copies into a read-back buffer, and
//!   [`FrameLayout::unpack`] turns the mapped buffer into one tightly packed frame.
//! - [`cpu::convert`] is the same conversion on the CPU.
//!
//! The matrix applies to the stored values as they are: the input must already be in the
//! output's primaries and transfer, and alpha is ignored.

#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

pub mod cpu;
mod gpu;
mod job;
mod layout;
mod spec;

pub use gpu::{Converter, Planes, Source};
pub use job::{PROVIDER_ID, YuvJob, YuvProvider};
pub use layout::{FrameLayout, PlaneLayout};
pub use spec::{Conversion, Error, Format, Matrix, Range};

#[cfg(test)]
mod job_tests;
#[cfg(test)]
mod oracle_tests;
#[cfg(test)]
mod tests;
