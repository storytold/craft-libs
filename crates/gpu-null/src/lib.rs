//! CPU-only [`GpuContext`](craft_gpu::GpuContext) for tests and CI.

#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

use craft_gpu::{DeviceCaps, GpuContext, GpuFault, HandleId, WorkerKind};

/// In-memory buffer store; no GPU hardware required.
pub struct NullContext {
    caps: DeviceCaps,
    worker_kind: WorkerKind,
    buffers: Vec<Vec<u8>>,
}

impl NullContext {
    pub fn new(worker_kind: WorkerKind) -> Self {
        Self {
            caps: DeviceCaps { backend_name: "null".into(), device_name: "null".into(), ..DeviceCaps::default() },
            worker_kind,
            buffers: Vec::new(),
        }
    }
}

impl GpuContext for NullContext {
    fn caps(&self) -> DeviceCaps {
        self.caps.clone()
    }

    fn worker_kind(&self) -> WorkerKind {
        self.worker_kind
    }

    fn upload_buffer(&mut self, _label: &str, bytes: &[u8]) -> Result<HandleId, GpuFault> {
        let id = HandleId(u32::try_from(self.buffers.len()).map_err(|_| GpuFault::Operation("too many buffers".into()))?);
        self.buffers.push(bytes.to_vec());
        Ok(id)
    }

    fn buffer_len(&self, handle: HandleId) -> Result<usize, GpuFault> {
        self.buffers.get(handle.0 as usize).map(|b| b.len()).ok_or_else(|| GpuFault::InvalidJob("bad handle".into()))
    }

    fn read_buffer(&self, handle: HandleId) -> Result<Vec<u8>, GpuFault> {
        self.buffers.get(handle.0 as usize).cloned().ok_or_else(|| GpuFault::InvalidJob("bad handle".into()))
    }
}
