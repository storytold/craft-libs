//! CPU-only [`GpuContext`](craft_gpu::GpuContext) for tests and CI.

#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

use craft_gpu::{DeviceCaps, GpuContext, GpuFault, HandleId, ReadOutcome, ReadRequest, WorkerKind};

/// In-memory buffer store; no GPU hardware required. Reads resolve synchronously (there is
/// nothing to wait for), so [`request_read`](GpuContext::request_read) and
/// [`poll_reads`](GpuContext::poll_reads) satisfy the trait trivially, and the default
/// [`read_buffer`](GpuContext::read_buffer) never loops more than once.
pub struct NullContext {
    caps: DeviceCaps,
    worker_kind: WorkerKind,
    buffers: Vec<Vec<u8>>,
    next_request: u64,
    ready: Vec<ReadOutcome>,
}

impl NullContext {
    pub fn new(worker_kind: WorkerKind) -> Self {
        Self {
            caps: DeviceCaps { backend_name: "null".into(), device_name: "null".into(), ..DeviceCaps::default() },
            worker_kind,
            buffers: Vec::new(),
            next_request: 0,
            ready: Vec::new(),
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

    fn request_read(&mut self, handle: HandleId) -> Result<ReadRequest, GpuFault> {
        let bytes = self.buffers.get(handle.0 as usize).cloned().ok_or_else(|| GpuFault::InvalidJob("bad handle".into()))?;
        let request = ReadRequest(self.next_request);
        self.next_request = self.next_request.wrapping_add(1);
        self.ready.push(ReadOutcome { request, handle, result: Ok(bytes) });
        Ok(request)
    }

    fn poll_reads(&mut self) -> Vec<ReadOutcome> {
        std::mem::take(&mut self.ready)
    }
}

#[cfg(test)]
mod tests {
    use craft_gpu::GpuContext;

    use super::NullContext;
    use craft_gpu::WorkerKind;

    #[test]
    fn request_read_then_poll_reads_delivers_the_same_bytes_as_read_buffer() {
        let mut ctx = NullContext::new(WorkerKind::Main);
        let handle = ctx.upload_buffer("in", b"abc").expect("upload_buffer");
        let request = ctx.request_read(handle).expect("request_read");
        let outcomes = ctx.poll_reads();
        assert_eq!(outcomes.len(), 1);
        assert_eq!(outcomes[0].request, request);
        assert_eq!(outcomes[0].handle, handle);
        assert_eq!(outcomes[0].result.as_deref(), Ok(b"abc".as_slice()));
    }

    #[test]
    fn read_buffer_default_still_works() {
        let mut ctx = NullContext::new(WorkerKind::SharedWorker);
        let handle = ctx.upload_buffer("in", b"xyz").expect("upload_buffer");
        assert_eq!(ctx.read_buffer(handle).expect("read_buffer"), b"xyz".to_vec());
    }
}
