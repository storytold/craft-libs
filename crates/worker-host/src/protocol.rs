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
    StorageFlush,
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
        outcomes: Vec<GpuJobOutcome>,
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
