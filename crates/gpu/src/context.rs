use crate::{DeviceCaps, GpuFault, HandleId};

/// Where the context was created (main thread vs worker).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WorkerKind {
    Main,
    DedicatedWorker,
    SharedWorker,
}

/// Opaque in-flight readback request handed out by [`GpuContext::request_read`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct ReadRequest(pub u64);

/// One completed (or failed) readback, delivered by [`GpuContext::poll_reads`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReadOutcome {
    pub request: ReadRequest,
    pub handle: HandleId,
    pub result: Result<Vec<u8>, GpuFault>,
}

/// Device/queue abstraction implemented by `craft-gpu-wgpu` and `craft-gpu-null`.
///
/// ## Device loss
///
/// A context can lose its device at any time: a driver reset, OS-level eviction under memory
/// pressure, or an explicit `device.destroy()`. Device loss is a fault, not a panic. It surfaces
/// through [`GpuFault::DeviceLost`] from the next call that would otherwise touch the device, and
/// also through [`poll_reads`](Self::poll_reads), so a host's event loop can observe it even
/// between buffer calls. Once a context has reported [`GpuFault::DeviceLost`], every
/// [`DeviceCaps`] it reported before that point is stale. The consumer's recovery, per RFC-0022,
/// is one of:
///
/// - fall back to the portable path (a [`DeviceCaps`]-gated path with no fast-path features, or a
///   different, CPU-backed [`GpuContext`]) for the rest of the session, or
/// - re-acquire a device, meaning construct a fresh [`GpuContext`], and re-probe
///   [`caps`](Self::caps) before resuming GPU jobs.
///
/// No backend here supports continuing to dispatch jobs against a context that has already
/// reported device loss, using caps read before the loss.
///
/// ## Readback
///
/// [`read_buffer`](Self::read_buffer) is a blocking convenience built on
/// [`request_read`](Self::request_read) and [`poll_reads`](Self::poll_reads): those two are the
/// primitives every backend implements directly, because WebGPU (and therefore native wgpu's
/// `buffer.map_async`) can only map a buffer asynchronously. The completion is delivered the same
/// way a `GpuJob` batch's outcome is delivered to a worker's host: queued, then collected on a
/// later poll, analogous to `craft-worker-host`'s `HostMessage::GpuJobs` being answered later by
/// `WorkerMessage::GpuBatchDone`.
///
/// A backend that can honestly block (the null context, which never has anything to wait for;
/// native wgpu driving `Device::poll` itself) uses `read_buffer`'s default body. A backend that
/// cannot honestly block without freezing its single thread (WebGPU on `wasm32`, where a map
/// completes only via the browser's own event loop) overrides `read_buffer` to return
/// [`GpuFault::Operation`] instead of deadlocking; callers there must drive `request_read` and
/// `poll_reads` from the worker's own message loop. See `docs/gpu-backends.md` for which backend
/// does which.
pub trait GpuContext {
    fn caps(&self) -> DeviceCaps;

    fn worker_kind(&self) -> WorkerKind;

    /// Register an opaque buffer blob; returns a handle for job graphs.
    fn upload_buffer(&mut self, label: &str, bytes: &[u8]) -> Result<HandleId, GpuFault>;

    fn buffer_len(&self, handle: HandleId) -> Result<usize, GpuFault>;

    /// Queue an asynchronous readback of `handle`. Completion is delivered by
    /// [`poll_reads`](Self::poll_reads).
    fn request_read(&mut self, handle: HandleId) -> Result<ReadRequest, GpuFault>;

    /// Drain every read completed (or failed) since the last call. Also where a device-lost
    /// fault surfaces for callers that are polling rather than issuing buffer calls. A context
    /// that resolves reads synchronously (the null context; native wgpu with no device) may
    /// resolve a request on the very next call after [`request_read`](Self::request_read).
    fn poll_reads(&mut self) -> Vec<ReadOutcome>;

    /// Blocking convenience for callers that need bytes now, such as the CPU-stand-in providers
    /// in `craft-gpu-providers`. The default drains [`request_read`](Self::request_read) and
    /// [`poll_reads`](Self::poll_reads) in a loop; see the trait docs for which backends honestly
    /// support blocking here and which override this to fail instead of hanging.
    fn read_buffer(&mut self, handle: HandleId) -> Result<Vec<u8>, GpuFault> {
        let request = self.request_read(handle)?;
        loop {
            for outcome in self.poll_reads() {
                if outcome.request == request {
                    return outcome.result;
                }
            }
        }
    }
}
