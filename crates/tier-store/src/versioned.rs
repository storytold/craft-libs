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
}
