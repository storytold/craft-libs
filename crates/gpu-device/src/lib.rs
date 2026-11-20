#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]
#![forbid(unsafe_code)]

//! Choosing and opening a wgpu device for a Craft app.
//!
//! - [`list`] gives every adapter as plain [`Candidate`] data for a settings page, and
//!   [`AdapterKey`] identifies one across runs.
//! - [`select`] and [`ranked`] choose an adapter for a [`Preference`] (Auto, low power, a named
//!   adapter, software) with wgpu's own rule and two corrections for drivers that crash
//!   ([`choose`]); [`Notice`] says when the choice was not the one asked for.
//! - [`open`] opens a device, moving to the next adapter and last to a software one when a device
//!   fails to open. [`pick`] fits egui-wgpu's adapter selector for a window. `list` and `open`
//!   block on wgpu's futures and are native only; [`list_async`] and [`open_async`] are the same
//!   as futures, for a `wasm32` web worker as well as native.
//! - In a browser, WebGPU offers one adapter for a power preference, so a named adapter cannot be
//!   chosen, and wgpu 30 cannot ask for the browser's fallback (software) adapter. `open_async`
//!   asks for the adapter with [`Preference::power`]. [`Capabilities`] and [`Recovery`] work
//!   there as they do on native, and so does [`DeviceHealth`] apart from its blocking waits,
//!   which are native only.
//! - [`Capabilities`] reads what the device can do (NV12 and P010 textures, 16-bit norm formats,
//!   `f16`, timestamps, the vendor, a software adapter, sizes) from the `craft_gpu::DeviceCaps`
//!   it holds, which [`device_caps`] and [`Opened::device_caps`] give for a `craft-gpu` context.
//! - [`DeviceHealth`] records a device loss or an uncaptured error instead of panicking, and
//!   [`Recovery`] says what to open after a loss. [`DeviceHealth::device_status`] reports a loss
//!   as `craft_gpu::GpuFault::DeviceLost`, and a [`Fault`] converts to a `craft_gpu::GpuFault`.
//!
//! The selection functions are pure, so tests run without a GPU.

mod adapter;
mod caps;
mod health;
mod open;
mod preference;
mod recovery;

pub use adapter::{Candidate, choose, intel_gen9_to_gen11, native_descriptor, native_instance};
pub use caps::{Capabilities, Vendor, device_caps};
pub use health::{DeviceHealth, Fault, OUT_OF_MEMORY, WaitOutcome};
pub use open::{OpenError, Opened, Request, list_async, open_async, pick};
#[cfg(not(target_arch = "wasm32"))]
pub use open::{list, open};
pub use preference::{AdapterKey, Notice, Preference, Selection, backend_name, ranked, select};
pub use recovery::{Recovery, RecoveryPolicy, RecoveryStep};
