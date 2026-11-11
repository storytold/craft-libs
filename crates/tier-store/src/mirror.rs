use std::cell::Cell;
use std::collections::{HashMap, HashSet, VecDeque};

use crate::{KeyClass, StoreKey, TierPolicy};

/// Coalesced async flush (newest write wins per key).
///
/// `deleted` is a tombstone: the warm tier should remove the key. Bytes are empty then.
/// `revision` is the key's [`MirrorStore`] revision at the moment this write was captured (bumped
/// on every `put`/`delete`): a later ack or restore compares against the key's *current*
/// revision to tell a stale, superseded attempt from the one that still matters.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PendingWrite {
    pub key: StoreKey,
    pub bytes: Vec<u8>,
    pub deleted: bool,
    pub revision: u64,
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
    /// Bytes for a cache key dropped from `data` while a flush may still need them. Only keys
    /// whose revision is not yet [`saved_revisions`](Self::saved_revisions) are retained here;
    /// retention itself is capped against `byte_budget` (see `enforce_eviction_retention_budget`).
    evicted: HashMap<StoreKey, Vec<u8>>,
    /// Eviction order (oldest first) for `evicted`, so retention pressure drops the oldest
    /// already-saved payload first rather than picking arbitrarily.
    evicted_order: VecDeque<StoreKey>,
    /// Bytes currently retained in `evicted`.
    evicted_bytes_used: usize,
    /// Highest revision per key that a flush has confirmed as durably persisted. Compared
    /// against `revisions` to tell a current save from one a later `put`/`delete` superseded.
    saved_revisions: HashMap<StoreKey, u64>,
    /// Bytes reserved for in-flight ranged hydrations (or whole-file fallbacks) that have not
    /// yet landed in `data` via `put`/`seed`. Released when the returned
    /// [`HydrationGuard`] drops. A `Cell` rather than a plain field so
    /// [`begin_range_hydration`](Self::begin_range_hydration) can hand out a guard that holds a
    /// shared reference: an in-flight reservation must not block every other read of the store
    /// (`over_budget`, `tracked_bytes`) for as long as the fetch it covers is running.
    inflight_range_bytes: Cell<usize>,
    /// Bytes a caller has exported across the worker boundary (a flush batch handed to a host
    /// awaiting [`PendingWrite`]-style acknowledgment) and not yet confirmed or restored. A
    /// `Cell`, like `inflight_range_bytes`, so charging and releasing it needs only `&self`. See
    /// [`charge_pending_export`](Self::charge_pending_export).
    exported_pending_bytes: Cell<usize>,
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
            evicted_order: VecDeque::new(),
            evicted_bytes_used: 0,
            saved_revisions: HashMap::new(),
            inflight_range_bytes: Cell::new(0),
            exported_pending_bytes: Cell::new(0),
        }
    }

    pub fn get(&mut self, key: &StoreKey) -> Option<&[u8]> {
        let hit = self.data.contains_key(key);
        if hit && self.policy.classify(key) == KeyClass::Cache {
            self.lru_touch(key.clone());
        }
        self.data.get(key).map(|v| v.as_slice())
    }

    /// Put flushed writes back when a sink failed to persist them. Only entries whose captured
    /// revision still matches the key's current revision are restored: a write superseded by a
    /// newer `put`/`delete` has nothing to retry (the newer write is already pending, or already
    /// saved), and restoring it would risk clobbering that newer state.
    pub fn restore_pending(&mut self, writes: impl IntoIterator<Item = PendingWrite>) {
        let current: Vec<PendingWrite> = writes.into_iter().filter(|write| self.revisions.get(&write.key).copied() == Some(write.revision)).collect();
        self.pending.splice(0..0, current);
    }

    /// Mark pending writes durably saved once a sink confirms it persisted them. A write whose
    /// captured revision no longer matches the key's current revision is a stale ack (a newer
    /// `put`/`delete` happened since it was taken): it must not mark anything saved, and must
    /// never touch resident bytes. A write that is still current frees any retained eviction
    /// copy for that key immediately -- a durable copy exists now, so nothing needs to keep
    /// backing a `requeue`.
    pub fn ack_saved(&mut self, writes: &[PendingWrite]) {
        for write in writes {
            if self.revisions.get(&write.key).copied() != Some(write.revision) {
                continue;
            }
            self.saved_revisions.insert(write.key.clone(), write.revision);
            self.drop_evicted_retention(&write.key);
        }
    }

    /// True when the key's current revision is confirmed durably persisted.
    pub fn is_saved(&self, key: &StoreKey) -> bool {
        matches!((self.saved_revisions.get(key), self.revisions.get(key)), (Some(saved), Some(current)) if saved == current)
    }

    /// Bytes currently retained for evicted-but-possibly-unflushed cache entries.
    pub fn retained_bytes(&self) -> usize {
        self.evicted_bytes_used
    }

    /// True when every byte counted against the budget (see [`tracked_bytes`](Self::tracked_bytes))
    /// exceeds it and nothing safe remains to reclaim. Surfaces that condition explicitly instead
    /// of growing retention without bound or discarding a write nothing has confirmed yet.
    pub fn over_budget(&self) -> bool {
        self.tracked_bytes() > self.byte_budget
    }

    /// Every byte counted against the single `byte_budget`: resident cache bytes, retained
    /// eviction bytes, in-flight ranged-hydration reservations, bytes sitting in unflushed
    /// pending writes (the copies that will cross the worker boundary on the next flush), and
    /// bytes already exported across that boundary but not yet acknowledged.
    ///
    /// RFC-0023 required fix 3: resident and retained-eviction bytes used to be checked against
    /// `byte_budget` in two separate pools, and in-flight hydration, pending-write, and exported
    /// bytes were not counted at all, so the true total could reach roughly twice the nominal
    /// budget (or more) before anything surfaced it. This is the one sum every caller must
    /// check.
    pub fn tracked_bytes(&self) -> usize {
        self.bytes_used
            .saturating_add(self.evicted_bytes_used)
            .saturating_add(self.inflight_range_bytes.get())
            .saturating_add(self.pending_bytes())
            .saturating_add(self.exported_pending_bytes.get())
    }

    /// Bytes still available before [`tracked_bytes`](Self::tracked_bytes) would exceed the
    /// budget. Zero once already over budget (never negative).
    pub fn remaining_budget(&self) -> usize {
        self.byte_budget.saturating_sub(self.tracked_bytes())
    }

    /// Would `put(key, bytes_len bytes)` push [`tracked_bytes`](Self::tracked_bytes) past the
    /// budget? `put` always adds two copies, a resident one and a pending-write one, not the
    /// single copy a naive `bytes_len > remaining_budget()` check assumes: with a 10-byte budget
    /// and nothing resident yet, a 6-byte `config/` put used to pass that check and then leave
    /// `tracked_bytes` at 12 (6 resident + 6 pending), past the budget, with no eviction able to
    /// correct it (`config/` keys are never LRU-evicted). This accounts for both copies, and for
    /// any existing resident bytes under `key` that the new resident copy would replace (so a
    /// same-size or shrinking overwrite near the budget ceiling is not wrongly refused). It does
    /// not assume LRU eviction will free room `put` has not freed yet: a conservative "no" is
    /// safe, a wrong "yes" is not.
    pub fn would_exceed_budget_after_put(&self, key: &StoreKey, bytes_len: usize) -> bool {
        let replaced_resident = self.data.get(key).map_or(0, Vec::len);
        let resident_delta = bytes_len.saturating_sub(replaced_resident);
        let pending_delta = bytes_len;
        self.tracked_bytes().saturating_add(resident_delta).saturating_add(pending_delta) > self.byte_budget
    }

    /// Charge bytes a caller has exported across the worker boundary (handed to a host as part
    /// of a flush batch) and is holding outside this mirror until an acknowledgment resolves.
    /// Without this, `take_pending` already dropped those bytes from `pending_bytes`, so an
    /// exported-but-unacknowledged batch cost nothing here: a stalled host could let a caller
    /// keep admitting new writes indefinitely while its own retained export memory grew past the
    /// budget unseen. Release with [`release_pending_export`](Self::release_pending_export) once
    /// the export is confirmed, restored, or otherwise no longer held outside the mirror.
    pub fn charge_pending_export(&self, bytes_len: usize) {
        self.exported_pending_bytes.set(self.exported_pending_bytes.get().saturating_add(bytes_len));
    }

    /// Release a charge added by [`charge_pending_export`](Self::charge_pending_export).
    pub fn release_pending_export(&self, bytes_len: usize) {
        self.exported_pending_bytes.set(self.exported_pending_bytes.get().saturating_sub(bytes_len));
    }

    fn pending_bytes(&self) -> usize {
        self.pending.iter().map(|write| write.bytes.len()).sum()
    }

    /// Reserve `bytes` against the budget for an in-flight ranged hydration, or a whole-file
    /// fallback read, that has not landed in `data` yet. Without this, a fetch in flight costs
    /// nothing against the budget while its bytes sit in memory waiting to land, so a burst of
    /// concurrent hydrations could blow well past `byte_budget` before any of them called `put`.
    /// Dropping the returned guard releases the reservation.
    pub fn begin_range_hydration(&self, bytes: usize) -> HydrationGuard<'_, P> {
        self.inflight_range_bytes.set(self.inflight_range_bytes.get().saturating_add(bytes));
        HydrationGuard { mirror: self, bytes }
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
        let revision = self.note_mutation(&key);
        self.tombstones.remove(&key);
        self.drop_evicted_retention(&key);
        self.insert_resident(key.clone(), bytes.clone());
        self.pending.push(PendingWrite { key, bytes, deleted: false, revision });
    }

    /// Drop the key and queue a warm-tier delete.
    pub fn delete(&mut self, key: StoreKey) {
        let revision = self.note_mutation(&key);
        self.drop_evicted_retention(&key);
        self.tombstones.insert(key.clone());
        if let Some(old) = self.data.remove(&key) {
            self.bytes_used = self.bytes_used.saturating_sub(old.len());
        }
        self.cache_lru.retain(|k| k != &key);
        self.pending.push(PendingWrite { key, bytes: Vec::new(), deleted: true, revision });
    }

    /// Queue the key again after a failed flush.
    ///
    /// A no-op when the key's current revision is already confirmed saved: nothing needs a
    /// retry. Otherwise uses resident bytes when they are still hot, or the retained eviction
    /// copy. A real `delete` stays a tombstone.
    pub fn requeue(&mut self, key: &StoreKey) {
        if self.is_saved(key) {
            return;
        }
        let revision = self.revisions.get(key).copied().unwrap_or(0);
        if self.tombstones.contains(key) {
            self.pending.push(PendingWrite { key: key.clone(), bytes: Vec::new(), deleted: true, revision });
            return;
        }
        let Some(bytes) = self.data.get(key).cloned().or_else(|| self.evicted.get(key).cloned()) else {
            return;
        };
        self.pending.push(PendingWrite { key: key.clone(), bytes, deleted: false, revision });
    }

    fn note_mutation(&mut self, key: &StoreKey) -> u64 {
        let revision = self.revisions.entry(key.clone()).or_insert(0);
        *revision = revision.saturating_add(1);
        *revision
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
                coalesced[idx].revision = write.revision;
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
                if self.needs_retention_on_evict(&victim) {
                    self.retain_evicted(victim, bytes);
                }
                // Else: a durable copy already exists (saved, or loaded from the warm tier via
                // `seed` and never mutated) -- drop the bytes outright, no retention needed.
            }
        }
        self.enforce_eviction_retention_budget();
    }

    /// A key needs its evicted bytes retained only if it has an outstanding write (a `put` or
    /// `delete` revision) that is not yet confirmed saved. A key only ever `seed`ed has no
    /// pending obligation at all: the warm tier already has it.
    fn needs_retention_on_evict(&self, key: &StoreKey) -> bool {
        match self.revisions.get(key) {
            None => false,
            Some(revision) => self.saved_revisions.get(key).copied() != Some(*revision),
        }
    }

    fn retain_evicted(&mut self, key: StoreKey, bytes: Vec<u8>) {
        let added = bytes.len();
        if let Some(old) = self.evicted.insert(key.clone(), bytes) {
            self.evicted_bytes_used = self.evicted_bytes_used.saturating_sub(old.len());
        } else {
            self.evicted_order.push_back(key);
        }
        self.evicted_bytes_used = self.evicted_bytes_used.saturating_add(added);
    }

    fn drop_evicted_retention(&mut self, key: &StoreKey) {
        if let Some(bytes) = self.evicted.remove(key) {
            self.evicted_bytes_used = self.evicted_bytes_used.saturating_sub(bytes.len());
            self.evicted_order.retain(|k| k != key);
        }
    }

    /// Reclaim retained-eviction bytes once they exceed the byte budget, oldest first, but only
    /// payloads already confirmed saved: nothing else is safe to drop. When nothing safe remains
    /// and retention is still over budget, stop -- that is backpressure, surfaced via
    /// [`over_budget`](Self::over_budget), never a silent discard of an unflushed write.
    fn enforce_eviction_retention_budget(&mut self) {
        while self.tracked_bytes() > self.byte_budget {
            if !self.drop_oldest_saved_eviction() {
                break;
            }
        }
    }

    fn drop_oldest_saved_eviction(&mut self) -> bool {
        let Some(pos) = self.evicted_order.iter().position(|key| self.is_saved(key)) else {
            return false;
        };
        let Some(key) = self.evicted_order.remove(pos) else {
            return false;
        };
        if let Some(bytes) = self.evicted.remove(&key) {
            self.evicted_bytes_used = self.evicted_bytes_used.saturating_sub(bytes.len());
        }
        true
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

/// Holds an in-flight ranged-hydration (or whole-file fallback) reservation open against a
/// [`MirrorStore`]'s byte budget. Returned by
/// [`MirrorStore::begin_range_hydration`]; releases the reservation on drop, including on an
/// early return from the fetch, so a failed or cancelled hydration never leaks budget.
pub struct HydrationGuard<'a, P: TierPolicy> {
    mirror: &'a MirrorStore<P>,
    bytes: usize,
}

impl<P: TierPolicy> Drop for HydrationGuard<'_, P> {
    fn drop(&mut self) {
        self.mirror.inflight_range_bytes.set(self.mirror.inflight_range_bytes.get().saturating_sub(self.bytes));
    }
}
