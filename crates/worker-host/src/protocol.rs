use craft_gpu::{DeviceCaps, GpuJob, GpuJobOutcome};
use serde::{Deserialize, Serialize};

/// Tab → SharedWorker / dedicated worker.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum HostMessage {
    Init {
        app_id: String,
        capabilities: Vec<String>,
    },
    /// Write into the worker mirror (hot tier); coalesced flush on [`StorageFlush`].
    StoragePut {
        key: String,
        bytes: Vec<u8>,
    },
    /// Delete a key from the worker mirror (hot tier); coalesced flush on [`StorageFlush`] like
    /// [`StoragePut`](Self::StoragePut), carried through as a tombstone (see
    /// [`PendingFlushEntry::deleted`]) so the warm tier removes the file instead of writing one.
    StorageDelete {
        key: String,
    },
    StorageFlush,
    /// Host confirmation of an exported flush batch, revision by revision: `persisted: true`
    /// marks that revision durably saved, `persisted: false` restores it to the pending queue
    /// for retry. An entry for a `(key, revision)` the runtime no longer has outstanding (already
    /// acked, or superseded by a newer write) is ignored.
    StorageFlushAck {
        results: Vec<FlushAckResult>,
    },
    GpuJobs {
        batch_id: u64,
        jobs: Vec<GpuJob>,
    },
    Cancel {
        batch_id: u64,
    },
}

/// One coalesced mirror write exported to JS warm-tier flush (OPFS).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PendingFlushEntry {
    pub key: String,
    pub bytes: Vec<u8>,
    /// A tombstone: the warm tier should remove this key's file rather than write `bytes`
    /// (which is empty for a deletion). Mirrors [`craft_tier_store::PendingWrite::deleted`].
    #[serde(default)]
    pub deleted: bool,
    /// The mirror's revision for `key` when this write was captured. Carried through so a host
    /// that acks a flush later can tell a current write from one a newer `put`/`delete`
    /// superseded in the meantime.
    pub revision: u64,
}

/// One flush-batch entry's outcome, reported back via [`HostMessage::StorageFlushAck`].
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct FlushAckResult {
    pub key: String,
    pub revision: u64,
    pub persisted: bool,
}

/// One [`HostMessage::GpuJobs`] batch entry's outcome, in input order, reported via
/// [`WorkerMessage::GpuBatchDone`]. Mirrors [`craft_gpu::ProviderRegistry::dispatch_batch`]'s own
/// per-job contract: a later job's failure never discards an earlier job's successful outcome,
/// and the tab can keep the done jobs' bytes or handles while retrying only the named failures.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum GpuJobResult {
    Done { outcome: GpuJobOutcome },
    Failed { message: String },
}

/// Worker → tab.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum WorkerMessage {
    Ready {
        caps: DeviceCaps,
    },
    GpuBatchDone {
        batch_id: u64,
        results: Vec<GpuJobResult>,
    },
    StorageFlushed {
        pending_count: usize,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pending: Option<Vec<PendingFlushEntry>>,
    },
    Error {
        message: String,
    },
}
