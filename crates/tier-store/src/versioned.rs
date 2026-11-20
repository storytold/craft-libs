use std::collections::HashMap;

use crate::{MirrorStore, PendingWrite, StoreKey, TierPolicy};

/// Per-key monotonic version for app adapters (EffectCraft `Entry::version`).
pub struct VersionedMirror<P: TierPolicy> {
    inner: MirrorStore<P>,
    versions: HashMap<StoreKey, u64>,
}

impl<P: TierPolicy> VersionedMirror<P> {
    pub fn new(policy: P, byte_budget: usize) -> Self {
        Self { inner: MirrorStore::new(policy, byte_budget), versions: HashMap::new() }
    }

    pub fn get(&mut self, key: &StoreKey) -> Option<&[u8]> {
        self.inner.get(key)
    }

    pub fn version(&self, key: &StoreKey) -> u64 {
        self.versions.get(key).copied().unwrap_or(0)
    }

    pub fn put(&mut self, key: StoreKey, bytes: Vec<u8>) -> u64 {
        let next = self.versions.get(&key).copied().unwrap_or(0).saturating_add(1);
        self.versions.insert(key.clone(), next);
        self.inner.put(key, bytes);
        next
    }

    pub fn take_pending(&mut self) -> Vec<PendingWrite> {
        self.inner.take_pending()
    }

    /// Mark pending writes durably saved once a sink confirms it persisted them. Forwards to
    /// [`MirrorStore::ack_saved`]. CodeRabbit finding on versioned.rs:32: `VersionedMirror`
    /// exposed `put` and `take_pending` but none of `MirrorStore`'s acknowledgment or retry
    /// surface, so a caller that drained a batch through this wrapper had no way to confirm or
    /// retry it when persistence failed.
    pub fn ack_saved(&mut self, writes: &[PendingWrite]) {
        self.inner.ack_saved(writes);
    }

    /// Restore writes a failed flush could not persist, keeping them queued for retry. Forwards
    /// to [`MirrorStore::restore_pending`].
    pub fn restore_pending(&mut self, writes: impl IntoIterator<Item = PendingWrite>) {
        self.inner.restore_pending(writes);
    }

    /// Queue `key` again after a failed flush, using its resident or retained-eviction bytes.
    /// Forwards to [`MirrorStore::requeue`].
    pub fn requeue(&mut self, key: &StoreKey) {
        self.inner.requeue(key);
    }

    /// True when `key`'s current revision is confirmed durably persisted. Forwards to
    /// [`MirrorStore::is_saved`].
    pub fn is_saved(&self, key: &StoreKey) -> bool {
        self.inner.is_saved(key)
    }

    /// True when the wrapped mirror is over its byte budget. Forwards to
    /// [`MirrorStore::over_budget`].
    pub fn over_budget(&self) -> bool {
        self.inner.over_budget()
    }
}
