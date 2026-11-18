#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

//! GPU start-up choices per platform for apps on eframe's wgpu backend: which backends to try, the power preference,
//! the DX12 shader compiler and, on Linux, which DRM device to render on. An app calls [`configure`] on the
//! `WgpuSetupCreateNew` it hands to eframe.
#![warn(missing_docs)]

mod device;

pub use device::configure;

#[cfg(test)]
mod device_tests;
