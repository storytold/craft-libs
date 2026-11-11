use craft_gpu::{GpuContext, GpuFault, ProviderRegistry, WorkerKind};
use craft_gpu_null::NullContext;
use craft_gpu_providers::register_all_into;
use craft_tier_store::{DefaultTierPolicy, MirrorStore, PendingWrite, StoreKey};
use craft_worker_host::{GpuJobResult, HostMessage, PendingFlushEntry, WorkerMessage};

use crate::dispatch_gpu_jobs;
use crate::flush::{FlushOutcome, FlushSink};

/// Which [`GpuContext`] implementation the runtime uses.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ContextBackend {
    #[default]
    Null,
    #[cfg(feature = "web")]
    Wgpu,
}

fn default_worker_kind() -> WorkerKind {
    if cfg!(feature = "shared-worker") { WorkerKind::SharedWorker } else { WorkerKind::Main }
}

/// Configures registry + GPU context before [`Runtime::build`].
pub struct RuntimeBuilder {
    registry: ProviderRegistry,
    backend: ContextBackend,
    worker_kind: WorkerKind,
    default_providers: bool,
    mirror_budget: usize,
    flush_sink: Option<Box<dyn FlushSink>>,
}

impl Default for RuntimeBuilder {
    fn default() -> Self {
        Self::new()
    }
}

impl RuntimeBuilder {
    pub fn new() -> Self {
        Self {
            registry: ProviderRegistry::new(),
            backend: ContextBackend::Null,
            worker_kind: default_worker_kind(),
            default_providers: false,
            mirror_budget: 64 * 1024 * 1024,
            flush_sink: None,
        }
    }

    pub fn flush_sink(mut self, sink: Box<dyn FlushSink>) -> Self {
        self.flush_sink = Some(sink);
        self
    }

    pub fn mirror_byte_budget(mut self, bytes: usize) -> Self {
        self.mirror_budget = bytes;
        self
    }

    pub fn with_worker_kind(mut self, kind: WorkerKind) -> Self {
        self.worker_kind = kind;
        self
    }

    pub fn null_context(mut self) -> Self {
        self.backend = ContextBackend::Null;
        self
    }

    // No `#[cfg(not(feature = "web"))]` fallback here on purpose (CodeRabbit finding on
    // runtime.rs:79): one used to return `self` unchanged, so a caller that omitted the `web`
    // feature and called this got a `Null` backend with no diagnostic at all -- build() would
    // silently construct a `NullContext` for a caller that asked for wgpu. Omitting the method
    // entirely makes that configuration a compile error instead of a silent downgrade.
    #[cfg(feature = "web")]
    pub fn wgpu_context(mut self) -> Self {
        self.backend = ContextBackend::Wgpu;
        self
    }

    /// Registers [`craft_gpu_providers::register_all`] providers.
    pub fn with_default_providers(mut self) -> Self {
        self.default_providers = true;
        self
    }

    pub fn registry_mut(&mut self) -> &mut ProviderRegistry {
        &mut self.registry
    }

    pub fn build(mut self) -> Runtime {
        if self.default_providers {
            register_all_into(&mut self.registry);
        }
        let ctx = match self.backend {
            ContextBackend::Null => RuntimeContext::Null(NullContext::new(self.worker_kind)),
            #[cfg(feature = "web")]
            ContextBackend::Wgpu => RuntimeContext::Wgpu(craft_gpu_wgpu::WgpuContext::new(self.worker_kind)),
        };
        Runtime {
            registry: self.registry,
            ctx,
            mirror: MirrorStore::new(DefaultTierPolicy, self.mirror_budget),
            app_id: None,
            flush_sink: self.flush_sink,
            exported_pending: Vec::new(),
        }
    }
}

enum RuntimeContext {
    Null(NullContext),
    #[cfg(feature = "web")]
    Wgpu(craft_gpu_wgpu::WgpuContext),
}

impl GpuContext for RuntimeContext {
    fn caps(&self) -> craft_gpu::DeviceCaps {
        match self {
            Self::Null(c) => c.caps(),
            #[cfg(feature = "web")]
            Self::Wgpu(c) => c.caps(),
        }
    }

    fn worker_kind(&self) -> WorkerKind {
        match self {
            Self::Null(c) => c.worker_kind(),
            #[cfg(feature = "web")]
            Self::Wgpu(c) => c.worker_kind(),
        }
    }

    fn upload_buffer(&mut self, label: &str, bytes: &[u8]) -> Result<craft_gpu::HandleId, GpuFault> {
        match self {
            Self::Null(c) => c.upload_buffer(label, bytes),
            #[cfg(feature = "web")]
            Self::Wgpu(c) => c.upload_buffer(label, bytes),
        }
    }

    fn buffer_len(&self, handle: craft_gpu::HandleId) -> Result<usize, GpuFault> {
        match self {
            Self::Null(c) => c.buffer_len(handle),
            #[cfg(feature = "web")]
            Self::Wgpu(c) => c.buffer_len(handle),
        }
    }

    fn request_read(&mut self, handle: craft_gpu::HandleId) -> Result<craft_gpu::ReadRequest, GpuFault> {
        match self {
            Self::Null(c) => c.request_read(handle),
            #[cfg(feature = "web")]
            Self::Wgpu(c) => c.request_read(handle),
        }
    }

    fn poll_reads(&mut self) -> Vec<craft_gpu::ReadOutcome> {
        match self {
            Self::Null(c) => c.poll_reads(),
            #[cfg(feature = "web")]
            Self::Wgpu(c) => c.poll_reads(),
        }
    }

    // Forwarded explicitly (not left to the trait default): NullContext and WgpuContext each
    // override read_buffer to put an unrelated outstanding request's completion back for the
    // next poll_reads, but the trait's own default body has nowhere to put one and drops it
    // instead. Falling through to that default here would silently discard another job's
    // in-flight read whenever two jobs in a batch had overlapping outstanding reads. Matching
    // on the backend and calling its own read_buffer keeps that guarantee.
    fn read_buffer(&mut self, handle: craft_gpu::HandleId) -> Result<Vec<u8>, GpuFault> {
        match self {
            Self::Null(c) => c.read_buffer(handle),
            #[cfg(feature = "web")]
            Self::Wgpu(c) => c.read_buffer(handle),
        }
    }

    fn free_buffer(&mut self, handle: craft_gpu::HandleId) {
        match self {
            Self::Null(c) => c.free_buffer(handle),
            #[cfg(feature = "web")]
            Self::Wgpu(c) => c.free_buffer(handle),
        }
    }

    // Forwarded explicitly, like read_buffer above: the trait's own default is always Ok(()),
    // which would silently hide WgpuContext's real device-loss status behind this facade.
    fn device_status(&self) -> Result<(), GpuFault> {
        match self {
            Self::Null(c) => c.device_status(),
            #[cfg(feature = "web")]
            Self::Wgpu(c) => c.device_status(),
        }
    }
}

/// Worker-side runtime: provider registry + GPU context + host RPC handling.
pub struct Runtime {
    registry: ProviderRegistry,
    ctx: RuntimeContext,
    mirror: MirrorStore<DefaultTierPolicy>,
    app_id: Option<String>,
    flush_sink: Option<Box<dyn FlushSink>>,
    /// Writes exported to the host (JS) by a [`HostMessage::StorageFlush`] on the no-native-sink
    /// path, kept here until [`HostMessage::StorageFlushAck`] confirms or rejects each one by
    /// revision. Without this, `take_pending` already removed them from `mirror` by the time
    /// they were exported, so a lost or failed host persistence had nothing to restore from: a
    /// second flush could report zero pending writes while the host had actually saved nothing.
    exported_pending: Vec<PendingWrite>,
}

impl Runtime {
    pub fn builder() -> RuntimeBuilder {
        RuntimeBuilder::new()
    }

    pub fn registry(&self) -> &ProviderRegistry {
        &self.registry
    }

    pub fn caps(&self) -> craft_gpu::DeviceCaps {
        self.ctx.caps()
    }

    pub fn store_get(&mut self, key: &StoreKey) -> Option<Vec<u8>> {
        self.mirror.get(key).map(|s| s.to_vec())
    }

    pub fn upload_buffer(&mut self, label: &str, bytes: &[u8]) -> Result<craft_gpu::HandleId, GpuFault> {
        self.ctx.upload_buffer(label, bytes)
    }

    /// Handle a single host → worker message; may produce zero or more replies.
    pub fn handle_host_message(&mut self, msg: HostMessage) -> Vec<WorkerMessage> {
        match msg {
            HostMessage::Init { app_id, capabilities: _ } => self.handle_init(app_id),
            HostMessage::StoragePut { key, bytes } => self.handle_storage_put(key, bytes),
            HostMessage::StorageDelete { key } => {
                self.mirror.delete(StoreKey::new(key));
                Vec::new()
            }
            HostMessage::StorageFlush => self.handle_storage_flush(),
            HostMessage::StorageFlushAck { results } => self.handle_storage_flush_ack(results),
            HostMessage::GpuJobs { batch_id, jobs } => {
                let results = dispatch_gpu_jobs(&self.registry, &mut self.ctx, &jobs)
                    .into_iter()
                    .map(|outcome| match outcome {
                        Ok(outcome) => GpuJobResult::Done { outcome },
                        Err(fault) => GpuJobResult::Failed { message: fault.to_string() },
                    })
                    .collect();
                vec![WorkerMessage::GpuBatchDone { batch_id, results }]
            }
            HostMessage::Cancel { .. } => Vec::new(),
        }
    }

    /// CodeRabbit finding on runtime.rs:222: this `Runtime` holds one `MirrorStore`, with no
    /// per-app partition inside it (the SharedWorker router partitions at that level instead;
    /// see `assets/craft-worker-host-routing.js`). Accepting an `Init` for a second, different
    /// app id on an already-initialized `Runtime` would keep the first app's mirror state
    /// reachable, and writable, under the new app id. Rather than build an equivalent per-app
    /// boundary inside `MirrorStore` for a case nothing here is meant to hit (one `Runtime` per
    /// app, for the whole worker lifetime, per the documented message flow), this refuses the
    /// change outright. An `Init` repeating the SAME app id is accepted: that is not a change.
    fn handle_init(&mut self, app_id: String) -> Vec<WorkerMessage> {
        if let Some(existing) = &self.app_id {
            if existing != &app_id {
                return vec![WorkerMessage::Error {
                    message: format!("init: this runtime is already initialized for app {existing:?}; cannot switch to {app_id:?}"),
                }];
            }
        }
        self.app_id = Some(app_id);
        vec![WorkerMessage::Ready { caps: self.ctx.caps() }]
    }

    /// Backpressure, never silent growth (RFC-0023 acceptance requirement 2): refuses a write
    /// that would push the mirror's tracked bytes past its budget instead of letting a
    /// `config/` key (never LRU-evicted) or an unbounded run of puts grow worker memory without
    /// limit. Checks the projected total `put` would actually leave behind (CodeRabbit finding
    /// on runtime.rs:252): `put` adds both a resident and a pending copy, so a plain
    /// `bytes.len() > remaining_budget()` check only ever caught half of what a write costs.
    fn handle_storage_put(&mut self, key: String, bytes: Vec<u8>) -> Vec<WorkerMessage> {
        let store_key = StoreKey::new(key);
        if self.mirror.would_exceed_budget_after_put(&store_key, bytes.len()) {
            return vec![WorkerMessage::Error {
                message: format!(
                    "storage put: {} bytes would exceed the mirror's byte budget ({} bytes remain)",
                    bytes.len(),
                    self.mirror.remaining_budget()
                ),
            }];
        }
        self.mirror.put(store_key, bytes);
        Vec::new()
    }

    fn handle_storage_flush(&mut self) -> Vec<WorkerMessage> {
        let pending = self.mirror.take_pending();
        match self.flush_sink.as_mut() {
            None => {
                // No native sink: hand the batch to the host (JS), which is responsible for
                // actually persisting it (and for not claiming success it can't back up -- see
                // assets/craft-worker-host-routing.js). Keep the batch here, by revision, until
                // StorageFlushAck confirms or rejects each entry: a write must stay recoverable
                // for retry until persistence is confirmed, never only until it was handed to
                // the host.
                let pending_count = pending.len();
                let pending_msg = pending
                    .iter()
                    .map(|write| PendingFlushEntry {
                        key: write.key.0.clone(),
                        bytes: write.bytes.clone(),
                        deleted: write.deleted,
                        revision: write.revision,
                    })
                    .collect();
                // CodeRabbit finding on runtime.rs:281: take_pending already dropped these bytes
                // from the mirror's own pending_bytes accounting, so an export sitting here
                // unacknowledged used to cost nothing against the budget. Charge it until
                // handle_storage_flush_ack releases it.
                for write in &pending {
                    self.mirror.charge_pending_export(write.bytes.len());
                }
                self.exported_pending.extend(pending);
                vec![WorkerMessage::StorageFlushed { pending_count, pending: Some(pending_msg) }]
            }
            Some(_) if pending.is_empty() => {
                vec![WorkerMessage::StorageFlushed { pending_count: 0, pending: None }]
            }
            Some(sink) => {
                let outcomes = sink.write_all(&pending);
                let mut persisted = Vec::new();
                let mut failed = Vec::new();
                for (index, write) in pending.into_iter().enumerate() {
                    if matches!(outcomes.get(index), Some(FlushOutcome::Persisted)) {
                        persisted.push(write);
                    } else {
                        failed.push(write);
                    }
                }
                let count = persisted.len();
                self.mirror.ack_saved(&persisted);
                if !failed.is_empty() {
                    self.mirror.restore_pending(failed);
                }
                if count == 0 {
                    // Every entry failed (or the sink reported nothing at all): the pending
                    // writes are already restored above. Never report success for data that was
                    // not actually persisted.
                    vec![WorkerMessage::Error { message: "storage flush: sink persisted none of the pending writes".into() }]
                } else {
                    vec![WorkerMessage::StorageFlushed { pending_count: count, pending: None }]
                }
            }
        }
    }

    /// Host confirmation of an exported flush batch, revision by revision. CodeRabbit CRITICAL
    /// finding on runtime.rs:226: `take_pending` already removed these writes from `mirror`, so
    /// without `exported_pending` keeping them (and their real bytes) until this arrives, a
    /// failed or lost host persistence had nothing to restore.
    fn handle_storage_flush_ack(&mut self, results: Vec<craft_worker_host::FlushAckResult>) -> Vec<WorkerMessage> {
        for result in results {
            let key = StoreKey::new(result.key);
            let Some(pos) = self.exported_pending.iter().position(|write| write.key == key && write.revision == result.revision) else {
                // Unknown to us: already acked, or superseded by a newer put/delete whose own
                // flush will carry its own acknowledgment. Either way, nothing here is still
                // waiting on this specific (key, revision) pair.
                continue;
            };
            let write = self.exported_pending.remove(pos);
            // Either way the export is settled: confirmed saved, or handed back to the mirror's
            // own pending queue (which will count it again through pending_bytes). Release the
            // exported-pending charge regardless of which branch runs below.
            self.mirror.release_pending_export(write.bytes.len());
            if result.persisted {
                self.mirror.ack_saved(std::slice::from_ref(&write));
            } else {
                self.mirror.restore_pending(std::iter::once(write));
            }
        }
        Vec::new()
    }
}

#[cfg(test)]
#[allow(clippy::expect_used)]
mod tests {
    use craft_gpu::GpuContext;

    use super::*;

    // review_repro: RuntimeContext::read_buffer (PR #662 review, CodeRabbit finding on
    // runtime.rs:170). Falling through to GpuContext::read_buffer's trait default here would
    // drop an unrelated outstanding request_read's completion, exactly the bug NullContext and
    // WgpuContext each override read_buffer themselves to avoid. RuntimeContext forwarding to
    // its active backend's own override must keep that guarantee.
    #[test]
    fn runtime_context_read_buffer_keeps_another_outstanding_requests_completion_queued() {
        let mut ctx = RuntimeContext::Null(NullContext::new(WorkerKind::Main));
        let a = ctx.upload_buffer("a", b"aa").expect("upload a");
        let b = ctx.upload_buffer("b", b"bb").expect("upload b");
        let request_a = ctx.request_read(a).expect("request_read a");
        assert_eq!(ctx.read_buffer(b).expect("read_buffer b"), b"bb".to_vec());
        let outcomes = ctx.poll_reads();
        assert_eq!(outcomes.len(), 1, "a's completion must survive b's read_buffer through RuntimeContext");
        assert_eq!(outcomes[0].request, request_a);
        assert_eq!(outcomes[0].result.as_deref(), Ok(b"aa".as_slice()));
    }

    /// GpuContext's free_buffer and device_status (new on the synced #663 carry) must reach the
    /// active backend through RuntimeContext, not silently no-op behind the facade.
    #[test]
    fn runtime_context_forwards_free_buffer_and_device_status() {
        let mut ctx = RuntimeContext::Null(NullContext::new(WorkerKind::Main));
        assert_eq!(ctx.device_status(), Ok(()));

        let handle = ctx.upload_buffer("a", b"aa").expect("upload a");
        ctx.free_buffer(handle);
        assert!(matches!(ctx.buffer_len(handle), Err(GpuFault::InvalidJob(_))), "a freed handle must behave as invalid through RuntimeContext");
    }
}
