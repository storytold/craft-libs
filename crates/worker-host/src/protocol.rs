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

/// Worker → tab.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum WorkerMessage {
    Ready { caps: DeviceCaps },
    GpuBatchDone { batch_id: u64, outcomes: Vec<GpuJobOutcome> },
    StorageFlushed { pending_count: usize },
    Error { message: String },
}
