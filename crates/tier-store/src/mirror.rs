use std::collections::{HashMap, HashSet, VecDeque};

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
    /// Bumped on `put` and `delete`. A later `seed` must not overwrite that key.
    revisions: HashMap<StoreKey, u64>,
    /// Explicit deletes. Eviction is not a delete.
    tombstones: HashSet<StoreKey>,
    /// Bytes for a cache key dropped from `data` while a flush may still need them.
    evicted: HashMap<StoreKey, Vec<u8>>,
}

impl<P: TierPolicy> MirrorStore<P> {
    pub fn new(policy: P, byte_budget: usize) -> Self {
        Self {
            policy,
            data: HashMap::new(),
            pending: Vec::new(),
            cache_lru: VecDeque::new(),
            byte_budget,
            bytes_used: 0,
            revisions: HashMap::new(),
            tombstones: HashSet::new(),
            evicted: HashMap::new(),
        }
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
    ///
    /// Ignored when this key was already `put` or `delete`d, including while that write is in
    /// flight. A late load must not put stale bytes back in front of `get`.
    pub fn seed(&mut self, key: StoreKey, bytes: Vec<u8>) {
        if self.revisions.get(&key).copied().unwrap_or(0) != 0 {
            return;
        }
        self.insert_resident(key, bytes);
    }

    pub fn put(&mut self, key: StoreKey, bytes: Vec<u8>) {
        self.note_mutation(&key);
        self.tombstones.remove(&key);
        self.evicted.remove(&key);
        self.insert_resident(key.clone(), bytes.clone());
        self.pending.push(PendingWrite { key, bytes, deleted: false });
    }

    /// Drop the key and queue a warm-tier delete.
    pub fn delete(&mut self, key: StoreKey) {
        self.note_mutation(&key);
        self.evicted.remove(&key);
        self.tombstones.insert(key.clone());
        if let Some(old) = self.data.remove(&key) {
            self.bytes_used = self.bytes_used.saturating_sub(old.len());
        }
        self.cache_lru.retain(|k| k != &key);
        self.pending.push(PendingWrite { key, bytes: Vec::new(), deleted: true });
    }

    /// Queue the key again after a failed flush.
    ///
    /// Uses resident bytes when they are still hot. A cache entry evicted under the byte budget
    /// is retried from the bytes that were queued, not as a delete. A real `delete` stays a tombstone.
    pub fn requeue(&mut self, key: &StoreKey) {
        if self.tombstones.contains(key) {
            self.pending.push(PendingWrite { key: key.clone(), bytes: Vec::new(), deleted: true });
            return;
        }
        let Some(bytes) = self.data.get(key).cloned().or_else(|| self.evicted.get(key).cloned()) else {
            return;
        };
        self.pending.push(PendingWrite { key: key.clone(), bytes, deleted: false });
    }

    fn note_mutation(&mut self, key: &StoreKey) {
        let revision = self.revisions.entry(key.clone()).or_insert(0);
        *revision = revision.saturating_add(1);
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
                self.evicted.insert(victim, bytes);
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
