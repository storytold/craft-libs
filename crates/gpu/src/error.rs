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
    #[error("operation failed: {0}")]
    Operation(String),
}
