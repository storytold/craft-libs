use craft_tier_store::PendingWrite;

/// Where warm-tier (OPFS / FS Access) flush payloads go after `MirrorStore::take_pending`.
pub trait FlushSink: Send {
    /// Persist pending mirror writes. Returns how many keys were accepted.
    fn write_all(&mut self, pending: &[PendingWrite]) -> usize;
}

/// In-memory sink for native tests and worker glue that records payloads.
#[derive(Default)]
pub struct MemoryFlushSink {
    pub writes: Vec<PendingWrite>,
}

impl FlushSink for MemoryFlushSink {
    fn write_all(&mut self, pending: &[PendingWrite]) -> usize {
        let count = pending.len();
        self.writes.extend_from_slice(pending);
        count
    }
}
