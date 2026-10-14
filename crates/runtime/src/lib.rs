//! Compose tier storage, worker protocol, and GPU dispatch for Craft `-web` apps.

#![deny(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::unimplemented,
    clippy::todo,
    clippy::unreachable
)]

mod gpu;
mod runtime;

#[cfg(test)]
mod tests;

pub use craft_gpu::{
    DeviceCaps, GpuContext, GpuFault, GpuJob, GpuJobOutcome, GpuProvider, HandleId, ProviderRegistry,
    WorkerKind,
};
pub use craft_gpu_null::NullContext;
pub use craft_gpu_providers::{register_all, register_all_into, NoopProvider};
pub use craft_tier_store::{
    DefaultTierPolicy, KeyClass, MirrorStore, PendingWrite, StoreKey, TierPolicy,
};
pub use craft_worker_host::{HostMessage, WorkerMessage};

#[cfg(feature = "cloud-delegate")]
pub use craft_tier_store::CloudBlobRef;

#[cfg(feature = "web")]
pub use craft_gpu_wgpu::WgpuContext;

pub use gpu::dispatch_gpu_jobs;
pub use runtime::{ContextBackend, Runtime, RuntimeBuilder};
