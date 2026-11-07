use crate::{DeviceCaps, GpuFault, HandleId};

/// Where the context was created (main thread vs worker).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WorkerKind {
    Main,
    DedicatedWorker,
    SharedWorker,
}

/// Device/queue abstraction implemented by `craft-gpu-wgpu` and `craft-gpu-null`.
pub trait GpuContext {
    fn caps(&self) -> DeviceCaps;

    fn worker_kind(&self) -> WorkerKind;

    /// Register an opaque buffer blob; returns a handle for job graphs.
    fn upload_buffer(&mut self, label: &str, bytes: &[u8]) -> Result<HandleId, GpuFault>;

    fn buffer_len(&self, handle: HandleId) -> Result<usize, GpuFault>;

    fn read_buffer(&self, handle: HandleId) -> Result<Vec<u8>, GpuFault>;
}
