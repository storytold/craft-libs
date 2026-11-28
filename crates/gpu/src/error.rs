use thiserror::Error;

/// Neutral GPU failure (no wgpu/WebGPU types cross this boundary).
#[derive(Clone, Debug, PartialEq, Eq, Error)]
pub enum GpuFault {
    #[error("provider not registered: {0}")]
    UnknownProvider(String),
    #[error("invalid job: {0}")]
    InvalidJob(String),
    #[error("device unavailable: {0}")]
    DeviceUnavailable(String),
    /// The device was lost (driver reset, OS eviction, explicit `destroy()`) after it was
    /// created. Every [`crate::DeviceCaps`] read before this point is stale: per
    /// [`crate::GpuContext`]'s trait docs, fall back to the portable path or re-acquire a device
    /// and re-probe caps. Never keep dispatching jobs against a context that reported this.
    #[error("device lost: {0}")]
    DeviceLost(String),
    #[error("operation failed: {0}")]
    Operation(String),
}
