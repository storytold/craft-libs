use craft_tier_store::PendingWrite;

/// Outcome of attempting to persist one [`PendingWrite`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FlushOutcome {
    Persisted,
    Failed,
}

/// Where warm-tier (OPFS / FS Access) flush payloads go after `MirrorStore::take_pending`.
pub trait FlushSink: Send {
    /// Persist pending mirror writes. Returns one outcome per entry in `pending`, in the same
    /// order, so the caller can tell exactly which keys landed rather than inferring it from a
    /// bare count. A sink that returns fewer outcomes than it was given is treated as failed for
    /// every entry past the end of what it returned.
    fn write_all(&mut self, pending: &[PendingWrite]) -> Vec<FlushOutcome>;
}

/// In-memory sink for native tests and worker glue that records payloads and always succeeds.
#[derive(Default)]
pub struct MemoryFlushSink {
    pub writes: Vec<PendingWrite>,
}

impl FlushSink for MemoryFlushSink {
    fn write_all(&mut self, pending: &[PendingWrite]) -> Vec<FlushOutcome> {
        self.writes.extend_from_slice(pending);
        vec![FlushOutcome::Persisted; pending.len()]
    }
}
