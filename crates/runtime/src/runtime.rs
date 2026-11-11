use craft_gpu::{GpuContext, GpuFault, ProviderRegistry, WorkerKind};
use craft_gpu_null::NullContext;
use craft_gpu_providers::register_all_into;
use craft_tier_store::{DefaultTierPolicy, MirrorStore, StoreKey};
use craft_worker_host::{HostMessage, PendingFlushEntry, WorkerMessage};

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

    #[cfg(feature = "web")]
    pub fn wgpu_context(mut self) -> Self {
        self.backend = ContextBackend::Wgpu;
        self
    }

    #[cfg(not(feature = "web"))]
    pub fn wgpu_context(self) -> Self {
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

    // read_buffer is intentionally not overridden here: both NullContext and WgpuContext
    // already override it to preserve unrelated in-flight completions (see
    // craft-gpu's GpuContext::read_buffer docs), so RuntimeContext forwards through the
    // trait's default body, which calls request_read/poll_reads on whichever backend is
    // active rather than duplicating each backend's own override.
}

/// Worker-side runtime: provider registry + GPU context + host RPC handling.
pub struct Runtime {
    registry: ProviderRegistry,
    ctx: RuntimeContext,
    mirror: MirrorStore<DefaultTierPolicy>,
    app_id: Option<String>,
    flush_sink: Option<Box<dyn FlushSink>>,
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
            HostMessage::Init { app_id, capabilities: _ } => {
                self.app_id = Some(app_id);
                vec![WorkerMessage::Ready { caps: self.ctx.caps() }]
            }
            HostMessage::StoragePut { key, bytes } => {
                self.mirror.put(StoreKey::new(key), bytes);
                Vec::new()
            }
            HostMessage::StorageFlush => {
                let pending = self.mirror.take_pending();
                match self.flush_sink.as_mut() {
                    None => {
                        // No native sink: hand the batch to the host (JS), which is responsible
                        // for actually persisting it (and for not claiming success it can't back
                        // up -- see assets/craft-worker-host-routing.js).
                        let pending_count = pending.len();
                        let pending_msg = pending
                            .into_iter()
                            .map(|write| PendingFlushEntry { key: write.key.0, bytes: write.bytes, revision: write.revision })
                            .collect();
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
                            // Every entry failed (or the sink reported nothing at all): the
                            // pending writes are already restored above. Never report success
                            // for data that was not actually persisted.
                            vec![WorkerMessage::Error { message: "storage flush: sink persisted none of the pending writes".into() }]
                        } else {
                            vec![WorkerMessage::StorageFlushed { pending_count: count, pending: None }]
                        }
                    }
                }
            }
            HostMessage::GpuJobs { batch_id, jobs } => match dispatch_gpu_jobs(&self.registry, &mut self.ctx, &jobs) {
                Ok(outcomes) => vec![WorkerMessage::GpuBatchDone { batch_id, outcomes }],
                Err(fault) => vec![WorkerMessage::Error { message: fault.to_string() }],
            },
            HostMessage::Cancel { .. } => Vec::new(),
        }
    }
}
