use std::collections::{HashMap, VecDeque};

use crate::{KeyClass, StoreKey, TierPolicy};

/// Coalesced async flush (newest write wins per key).
///
/// `deleted` is a tombstone: the warm tier should remove the key. Bytes are empty then.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PendingWrite {
    pub key: StoreKey,
    pub bytes: Vec<u8>,
    pub deleted: bool,
}

/// In-memory hot tier with optional LRU byte cap on cache-class keys.
pub struct MirrorStore<P: TierPolicy> {
    policy: P,
    data: HashMap<StoreKey, Vec<u8>>,
    pending: Vec<PendingWrite>,
    /// Front = least recently used among keys still in `data`.
    cache_lru: VecDeque<StoreKey>,
    byte_budget: usize,
    bytes_used: usize,
}

impl<P: TierPolicy> MirrorStore<P> {
    pub fn new(policy: P, byte_budget: usize) -> Self {
        Self { policy, data: HashMap::new(), pending: Vec::new(), cache_lru: VecDeque::new(), byte_budget, bytes_used: 0 }
    }

    pub fn get(&mut self, key: &StoreKey) -> Option<&[u8]> {
        let hit = self.data.get(key).is_some();
        if hit && self.policy.classify(key) == KeyClass::Cache {
            self.lru_touch(key.clone());
        }
        self.data.get(key).map(|v| v.as_slice())
    }

    /// Put flushed writes back when a sink accepted only part of a batch.
    pub fn restore_pending(&mut self, writes: impl IntoIterator<Item = PendingWrite>) {
        self.pending.splice(0..0, writes);
    }

    /// Resident bytes from a warm-tier load. Nothing is queued to flush.
    pub fn seed(&mut self, key: StoreKey, bytes: Vec<u8>) {
        self.insert_resident(key, bytes);
    }

    pub fn put(&mut self, key: StoreKey, bytes: Vec<u8>) {
        self.insert_resident(key.clone(), bytes.clone());
        self.pending.push(PendingWrite { key, bytes, deleted: false });
    }

    /// Drop the key and queue a warm-tier delete.
    pub fn delete(&mut self, key: StoreKey) {
        if let Some(old) = self.data.remove(&key) {
            self.bytes_used = self.bytes_used.saturating_sub(old.len());
        }
        self.cache_lru.retain(|k| k != &key);
        self.pending.push(PendingWrite { key, bytes: Vec::new(), deleted: true });
    }

    /// Queue the key again after a failed flush. Latest resident bytes, or a tombstone if gone.
    pub fn requeue(&mut self, key: &StoreKey) {
        if let Some(bytes) = self.data.get(key).cloned() {
            self.pending.push(PendingWrite { key: key.clone(), bytes, deleted: false });
        } else {
            self.pending.push(PendingWrite { key: key.clone(), bytes: Vec::new(), deleted: true });
        }
    }

    pub fn has_pending(&self) -> bool {
        !self.pending.is_empty()
    }

    fn insert_resident(&mut self, key: StoreKey, bytes: Vec<u8>) {
        let class = self.policy.classify(&key);
        if let Some(old) = self.data.insert(key.clone(), bytes.clone()) {
            self.bytes_used = self.bytes_used.saturating_sub(old.len());
        }
        self.bytes_used = self.bytes_used.saturating_add(bytes.len());
        if class == KeyClass::Cache {
            self.lru_touch(key);
        }
        self.enforce_budget();
    }

    /// Drain pending writes, keeping only the latest payload per key (insertion order of first touch).
    pub fn take_pending(&mut self) -> Vec<PendingWrite> {
        let pending = std::mem::take(&mut self.pending);
        let mut index_by_key: HashMap<StoreKey, usize> = HashMap::new();
        let mut coalesced: Vec<PendingWrite> = Vec::new();
        for write in pending {
            if let Some(&idx) = index_by_key.get(&write.key) {
                coalesced[idx].bytes = write.bytes;
                coalesced[idx].deleted = write.deleted;
            } else {
                index_by_key.insert(write.key.clone(), coalesced.len());
                coalesced.push(write);
            }
        }
        coalesced
    }

    fn enforce_budget(&mut self) {
        while self.bytes_used > self.byte_budget {
            let Some(victim) = self.pick_cache_victim() else {
                break;
            };
            if let Some(bytes) = self.data.remove(&victim) {
                self.bytes_used = self.bytes_used.saturating_sub(bytes.len());
            }
        }
    }

    fn pick_cache_victim(&mut self) -> Option<StoreKey> {
        while let Some(key) = self.cache_lru.pop_front() {
            if self.data.contains_key(&key) {
                return Some(key);
            }
        }
        None
    }

    fn lru_touch(&mut self, key: StoreKey) {
        if self.policy.classify(&key) != KeyClass::Cache {
            return;
        }
        self.cache_lru.retain(|k| k != &key);
        self.cache_lru.push_back(key);
    }
}
