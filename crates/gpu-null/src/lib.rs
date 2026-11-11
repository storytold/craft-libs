//! CPU-only [`GpuContext`](craft_gpu::GpuContext) for tests and CI.

#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

use craft_gpu::{DeviceCaps, GpuContext, GpuFault, HandleId, ReadOutcome, ReadRequest, WorkerKind};

/// In-memory buffer store; no GPU hardware required. Reads resolve synchronously (there is
/// nothing to wait for), so [`request_read`](GpuContext::request_read) and
/// [`poll_reads`](GpuContext::poll_reads) satisfy the trait trivially.
/// [`read_buffer`](GpuContext::read_buffer) is overridden to take only its own completion,
/// leaving completions for other outstanding requests queued for the next
/// [`poll_reads`](GpuContext::poll_reads).
pub struct NullContext {
    caps: DeviceCaps,
    worker_kind: WorkerKind,
    /// `None` means freed (or never allocated): handles are stable indices into this Vec, so a
    /// freed slot cannot simply be removed without shifting every later handle's index.
    buffers: Vec<Option<Vec<u8>>>,
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
        self.buffers.push(Some(bytes.to_vec()));
        Ok(id)
    }

    fn buffer_len(&self, handle: HandleId) -> Result<usize, GpuFault> {
        self.buffers.get(handle.0 as usize).and_then(Option::as_ref).map(Vec::len).ok_or_else(|| GpuFault::InvalidJob("bad handle".into()))
    }

    fn request_read(&mut self, handle: HandleId) -> Result<ReadRequest, GpuFault> {
        let bytes = self.buffers.get(handle.0 as usize).and_then(Option::as_ref).cloned().ok_or_else(|| GpuFault::InvalidJob("bad handle".into()))?;
        let request = ReadRequest(self.next_request);
        self.next_request = self.next_request.wrapping_add(1);
        self.ready.push(ReadOutcome { request, handle, result: Ok(bytes) });
        Ok(request)
    }

    fn poll_reads(&mut self) -> Vec<ReadOutcome> {
        std::mem::take(&mut self.ready)
    }

    fn read_buffer(&mut self, handle: HandleId) -> Result<Vec<u8>, GpuFault> {
        let request = self.request_read(handle)?;
        match self.ready.iter().position(|o| o.request == request) {
            Some(i) => self.ready.remove(i).result,
            None => Err(GpuFault::Operation("request_read did not resolve synchronously".into())),
        }
    }

    /// Replaces the slot with `None` rather than removing it: handles are stable indices, and
    /// removing would shift every later handle's index out from under its caller.
    fn free_buffer(&mut self, handle: HandleId) {
        if let Some(slot) = self.buffers.get_mut(handle.0 as usize) {
            *slot = None;
        }
    }
}

#[cfg(test)]
mod tests {
    use craft_gpu::{GpuContext, GpuFault, HandleId, WorkerKind};

    use super::NullContext;

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

    #[test]
    fn read_buffer_keeps_another_outstanding_requests_completion_queued() {
        let mut ctx = NullContext::new(WorkerKind::Main);
        let a = ctx.upload_buffer("a", b"aa").expect("upload a");
        let b = ctx.upload_buffer("b", b"bb").expect("upload b");
        let request_a = ctx.request_read(a).expect("request_read a");
        assert_eq!(ctx.read_buffer(b).expect("read_buffer b"), b"bb".to_vec());
        let outcomes = ctx.poll_reads();
        assert_eq!(outcomes.len(), 1, "a's completion must survive b's read_buffer");
        assert_eq!(outcomes[0].request, request_a);
        assert_eq!(outcomes[0].result.as_deref(), Ok(b"aa".as_slice()));
    }

    #[test]
    fn an_empty_buffer_uploads_and_reads_back_empty() {
        let mut ctx = NullContext::new(WorkerKind::Main);
        let handle = ctx.upload_buffer("empty", b"").expect("upload empty");
        assert_eq!(ctx.buffer_len(handle).expect("buffer_len"), 0);
        assert_eq!(ctx.read_buffer(handle).expect("read_buffer"), Vec::<u8>::new());
    }

    /// CodeRabbit finding on gpu-wgpu/src/lib.rs:313: nothing released an uploaded buffer's
    /// resources, so a backend's buffer table could only ever grow. free_buffer must actually
    /// reclaim the payload and make the handle behave as invalid afterward, not just forget it
    /// exists while still holding its bytes.
    #[test]
    fn free_buffer_invalidates_the_handle_and_does_not_shift_other_handles() {
        let mut ctx = NullContext::new(WorkerKind::Main);
        let a = ctx.upload_buffer("a", b"aa").expect("upload a");
        let b = ctx.upload_buffer("b", b"bb").expect("upload b");

        ctx.free_buffer(a);

        assert!(matches!(ctx.buffer_len(a), Err(GpuFault::InvalidJob(_))), "a freed handle must behave as invalid");
        assert_eq!(ctx.buffer_len(b).expect("b must be unaffected by freeing a"), 2);
        assert_eq!(ctx.read_buffer(b).expect("read_buffer b"), b"bb".to_vec(), "b's own index must not have shifted");
    }

    /// Freeing a handle twice, or a handle that never existed, is a no-op: callers are not
    /// expected to track whether they already freed one.
    #[test]
    fn free_buffer_is_a_harmless_no_op_when_repeated_or_given_an_unknown_handle() {
        let mut ctx = NullContext::new(WorkerKind::Main);
        let a = ctx.upload_buffer("a", b"aa").expect("upload a");
        ctx.free_buffer(a);
        ctx.free_buffer(a); // already freed
        ctx.free_buffer(HandleId(999)); // never existed
    }
}
