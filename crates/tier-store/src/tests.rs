#![allow(clippy::expect_used)]

use crate::{DefaultTierCoordinator, DefaultTierPolicy, ExternalVolume, MirrorStore, StoreKey, TierCoordinator, VersionedMirror};

#[test]
fn mirror_round_trip_and_pending_flush() {
    let mut store = MirrorStore::new(DefaultTierPolicy, 1024);
    let key = StoreKey::new("config/prefs.json");
    store.put(key.clone(), br#"{"a":1}"#.to_vec());
    assert_eq!(store.get(&key), Some(br#"{"a":1}"#.as_slice()));
    let pending = store.take_pending();
    assert_eq!(pending.len(), 1);
    assert_eq!(pending[0].key, key);
}

#[test]
fn cache_evicted_when_over_budget() {
    let mut store = MirrorStore::new(DefaultTierPolicy, 10);
    store.put(StoreKey::new("cache/a"), vec![0u8; 8]);
    store.put(StoreKey::new("cache/b"), vec![1u8; 8]);
    assert!(store.get(&StoreKey::new("cache/a")).is_none() || store.get(&StoreKey::new("cache/b")).is_none());
}

#[test]
fn cache_lru_evicts_least_recently_used() {
    let mut store = MirrorStore::new(DefaultTierPolicy, 20);
    let a = StoreKey::new("cache/a");
    let b = StoreKey::new("cache/b");
    let c = StoreKey::new("cache/c");
    store.put(a.clone(), vec![0u8; 8]);
    store.put(b.clone(), vec![1u8; 8]);
    assert!(store.get(&a).is_some());
    store.put(c.clone(), vec![2u8; 8]);
    assert!(store.get(&b).is_none(), "b should be LRU victim");
    assert!(store.get(&a).is_some());
    assert!(store.get(&c).is_some());
}

#[test]
fn restore_pending_prepends_before_newer_puts() {
    let mut store = MirrorStore::new(DefaultTierPolicy, 1024);
    let key = StoreKey::new("files/one");
    store.put(key.clone(), vec![1]);
    let batch = store.take_pending();
    assert_eq!(batch.len(), 1);
    store.put(key.clone(), vec![9]);
    store.restore_pending(batch);
    let pending = store.take_pending();
    assert_eq!(pending.len(), 1);
    assert_eq!(pending[0].bytes, vec![9]);
}

#[test]
fn a_late_seed_does_not_overwrite_a_put_or_delete() {
    let mut store = MirrorStore::new(DefaultTierPolicy, 1024);
    let key = StoreKey::new("config/session.ecproj");
    store.put(key.clone(), b"new".to_vec());
    store.seed(key.clone(), b"stale".to_vec());
    assert_eq!(store.get(&key), Some(b"new".as_slice()));
    let pending = store.take_pending();
    assert_eq!(pending[0].bytes, b"new");
    store.seed(key.clone(), b"stale".to_vec());
    assert_eq!(store.get(&key), Some(b"new".as_slice()), "an in-flight flush still owns the key");

    store.delete(key.clone());
    store.seed(key.clone(), b"stale".to_vec());
    assert!(store.get(&key).is_none());
    let pending = store.take_pending();
    assert!(pending[0].deleted);
}

#[test]
fn requeue_of_an_evicted_cache_entry_keeps_its_bytes() {
    let mut store = MirrorStore::new(DefaultTierPolicy, 4);
    let key = StoreKey::new("cache/slice");
    store.put(key.clone(), vec![1, 2, 3, 4, 5]);
    assert!(store.get(&key).is_none(), "over budget, so the resident copy is gone");
    let pending = store.take_pending();
    assert_eq!(pending[0].bytes, vec![1, 2, 3, 4, 5]);
    assert!(!pending[0].deleted);
    store.requeue(&key);
    let again = store.take_pending();
    assert_eq!(again, pending);
}

#[test]
fn seed_does_not_flush_and_delete_is_a_tombstone() {
    let mut store = MirrorStore::new(DefaultTierPolicy, 1024);
    let key = StoreKey::new("config/session.ecproj");
    store.seed(key.clone(), b"old".to_vec());
    assert!(!store.has_pending());
    store.delete(key.clone());
    let pending = store.take_pending();
    assert_eq!(pending.len(), 1);
    assert!(pending[0].deleted);
    assert!(store.get(&key).is_none());
}

#[test]
fn take_pending_coalesces_per_key() {
    let mut store = MirrorStore::new(DefaultTierPolicy, 1024);
    let k1 = StoreKey::new("files/one");
    let k2 = StoreKey::new("files/two");
    store.put(k1.clone(), vec![1]);
    store.put(k2.clone(), vec![2]);
    store.put(k1.clone(), vec![9, 9]);
    let pending = store.take_pending();
    assert_eq!(pending.len(), 2);
    assert_eq!(pending[0].key, k1);
    assert_eq!(pending[0].bytes, vec![9, 9]);
    assert_eq!(pending[1].key, k2);
    assert_eq!(pending[1].bytes, vec![2]);
    assert!(store.take_pending().is_empty());
}

#[test]
fn slice_cache_key_format() {
    let key = StoreKey::slice_cache(&[0xab, 0xcd], 4096, 512);
    assert_eq!(key, StoreKey::new("cache/slice/abcd/4096+512"));
}

#[test]
fn default_coordinator_warm_and_cold_paths() {
    let coord = DefaultTierCoordinator::default();
    let key = StoreKey::new("config/prefs.json");
    assert_eq!(coord.warm_opfs_relative_path(&key), "effectcraft-store/v1/config/prefs.json");
    let vol = ExternalVolume::new("cold-root-7");
    assert!(coord.cold_relative_path(&vol, &key).is_none());
    let media = StoreKey::new("files/clip.mp4");
    assert_eq!(coord.cold_relative_path(&vol, &media), Some("volumes/cold-root-7/files/clip.mp4".into()));
}

#[test]
fn external_volume_serde_roundtrip() {
    let vol = ExternalVolume::new("cold-root-7").with_label("Exports");
    let json = serde_json::to_string(&vol).expect("serialize");
    let back: ExternalVolume = serde_json::from_str(&json).expect("deserialize");
    assert_eq!(back, vol);
}

#[test]
fn versioned_mirror_bumps_per_key_version() {
    let mut store = VersionedMirror::new(DefaultTierPolicy, 4096);
    let key = StoreKey::new("files/clip.mp4");
    assert_eq!(store.version(&key), 0);
    assert_eq!(store.put(key.clone(), b"v1".to_vec()), 1);
    assert_eq!(store.put(key.clone(), b"v2".to_vec()), 2);
    assert_eq!(store.get(&key), Some(b"v2".as_slice()));
    assert_eq!(store.version(&key), 2);
}

// --- review_repro: revision-tied flush acknowledgment (PR #662 review, failure 2) -------------

/// Two overlapping flushes for the same key settle out of order. Regardless of which
/// acknowledgment arrives first, the newer revision's bytes and saved-state must win: an older,
/// stale ack must neither overwrite resident state nor mark its (superseded) revision saved.
#[test]
fn ack_saved_ties_to_revision_so_a_stale_completion_never_wins() {
    let key = StoreKey::new("files/doc");

    // Ordering 1: the newer flush (B) is acknowledged before the older one (A).
    let mut store = MirrorStore::new(DefaultTierPolicy, 1024);
    store.put(key.clone(), b"v1".to_vec());
    let flush_a = store.take_pending();
    store.put(key.clone(), b"v2".to_vec());
    let flush_b = store.take_pending();
    store.ack_saved(&flush_b);
    store.ack_saved(&flush_a); // stale: key's revision has moved past flush_a's
    assert_eq!(store.get(&key), Some(b"v2".as_slice()), "a stale ack must not touch resident state");
    assert!(store.is_saved(&key), "the current (newer) revision must be the one marked saved");

    // Ordering 2: the older flush (A) is acknowledged before the newer one (B).
    let mut store = MirrorStore::new(DefaultTierPolicy, 1024);
    store.put(key.clone(), b"v1".to_vec());
    let flush_a = store.take_pending();
    store.put(key.clone(), b"v2".to_vec());
    let flush_b = store.take_pending();
    store.ack_saved(&flush_a); // stale on arrival: a newer write already exists
    assert!(!store.is_saved(&key), "an ack for a superseded revision must not mark the current one saved");
    store.ack_saved(&flush_b);
    assert_eq!(store.get(&key), Some(b"v2".as_slice()));
    assert!(store.is_saved(&key));
}

/// Once a key's current revision is confirmed saved, `requeue` is a no-op: there is nothing left
/// to retry, matching the gist's H7 repro (an evicted, already-flushed payload must not come back
/// through `requeue`).
#[test]
fn requeue_is_a_noop_once_saved() {
    let mut store = MirrorStore::new(DefaultTierPolicy, 1024);
    let key = StoreKey::new("cache/a");
    store.put(key.clone(), vec![7u8; 16]);
    let batch = store.take_pending();
    store.ack_saved(&batch);
    store.requeue(&key);
    assert!(store.take_pending().is_empty(), "a saved key has nothing to retry");
}

// --- review_repro: retained-eviction bytes are bounded (PR #662 review, failure 3) ------------

/// Mirrors the gist's exact workload (put, then flush-and-ack immediately, repeated): when every
/// write keeps getting durably saved before the budget cycles back around to it, retention stays
/// bounded by the budget instead of growing with every eviction (the gist's 1 MiB budget growing
/// RSS by 32.6 MiB over 512 such writes).
#[test]
fn retained_eviction_bytes_stay_bounded_when_every_write_is_saved_before_its_turn() {
    let mut store = MirrorStore::new(DefaultTierPolicy, 64);
    for i in 0..200 {
        store.put(StoreKey::new(format!("cache/{i}")), vec![9u8; 8]);
        let batch = store.take_pending();
        store.ack_saved(&batch);
        assert!(store.retained_bytes() <= 64, "retained eviction bytes grew unbounded at entry {i}: {} bytes retained", store.retained_bytes());
    }
}

/// When nothing is ever flushed, retention legitimately holds every unflushed byte: there is
/// nothing safe to drop. That must surface as explicit backpressure, and nothing may be silently
/// discarded -- every write stays fully recoverable through `take_pending`.
#[test]
fn retained_eviction_bytes_surface_as_backpressure_when_nothing_is_ever_saved() {
    let mut store = MirrorStore::new(DefaultTierPolicy, 16);
    for i in 0..50 {
        store.put(StoreKey::new(format!("cache/{i}")), vec![9u8; 8]);
    }
    assert!(store.over_budget(), "retention over budget with nothing flushed must surface as backpressure, not silent growth");
    // Nothing was silently discarded: every one of the 50 writes is still queued to flush.
    assert_eq!(store.take_pending().len(), 50);
}

/// A key already confirmed saved before it is ever evicted is dropped outright, not retained:
/// there is nothing left for a retry to need. Compares against an identical, unsaved sibling run
/// so the assertion does not depend on the exact arithmetic of an unrelated eviction cascade.
#[test]
fn a_saved_key_is_dropped_outright_on_eviction_not_retained() {
    let others = |store: &mut MirrorStore<DefaultTierPolicy>| {
        for i in 0..10 {
            store.put(StoreKey::new(format!("cache/other-{i}")), vec![9u8; 8]);
        }
    };

    let mut unsaved = MirrorStore::new(DefaultTierPolicy, 16);
    unsaved.put(StoreKey::new("cache/a"), vec![9u8; 8]);
    others(&mut unsaved);

    let mut saved = MirrorStore::new(DefaultTierPolicy, 16);
    saved.put(StoreKey::new("cache/a"), vec![9u8; 8]);
    let batch = saved.take_pending();
    saved.ack_saved(&batch);
    others(&mut saved);

    assert!(
        saved.retained_bytes() < unsaved.retained_bytes(),
        "a saved key's eviction must retain fewer bytes than an identical unsaved one: saved={} unsaved={}",
        saved.retained_bytes(),
        unsaved.retained_bytes()
    );
}

// --- review_repro: unified byte budget (PR #662 review, RFC-0023 required fix 3) --------------

/// Resident and retained-eviction bytes used to be checked against `byte_budget` as two separate
/// pools (`over_budget` was `bytes_used > budget || evicted_bytes_used > budget`), so a store
/// sitting at 60 percent resident and 60 percent retained passed both checks even though it held
/// 120 percent of the budget in memory. `over_budget` must trip on the true sum.
#[test]
fn over_budget_trips_on_the_combined_total_not_either_pool_alone() {
    let mut store = MirrorStore::new(DefaultTierPolicy, 100);
    let a = StoreKey::new("cache/a");
    let b = StoreKey::new("cache/b");

    // `a` resident at 60% of budget, unsaved. Drain the pending copy so this scenario isolates
    // resident vs. retained bytes; pending-byte accounting is its own test below.
    store.put(a.clone(), vec![1u8; 60]);
    store.take_pending();
    assert!(!store.over_budget(), "60% resident alone must not trip the budget");

    // `b` resident at another 60%: `enforce_budget` evicts `a` (LRU) to bring resident back
    // under budget, retaining its unsaved bytes rather than dropping them, so `a`'s 60 bytes
    // move from resident into retained eviction while `b`'s 60 bytes become the new resident.
    store.put(b.clone(), vec![2u8; 60]);
    store.take_pending();
    assert!(store.get(&a).is_none(), "a should have been evicted to make room for b");
    assert_eq!(store.retained_bytes(), 60, "a's unsaved bytes must be retained, not dropped");

    assert!(
        store.over_budget(),
        "resident (60) + retained (60) = 120% of a 100-byte budget must trip over_budget even though neither pool alone exceeds it"
    );
    assert_eq!(store.tracked_bytes(), 120, "tracked_bytes must report the true combined total");
}

/// Bytes reserved for an in-flight ranged hydration count against the budget even before they
/// land anywhere in `data`: otherwise a fetch in flight is invisible to `over_budget` no matter
/// how large it is.
#[test]
fn inflight_range_hydration_bytes_count_against_the_budget() {
    let store = MirrorStore::new(DefaultTierPolicy, 100);
    assert!(!store.over_budget());
    let guard = store.begin_range_hydration(150);
    assert!(
        store.over_budget(),
        "a 150-byte in-flight reservation against a 100-byte budget must be visible even though nothing landed in `data` yet"
    );
    drop(guard);
    assert!(!store.over_budget(), "dropping the hydration guard must release the reservation");
}

/// Bytes sitting in unflushed pending writes (the copies that will cross the worker boundary on
/// the next flush) count against the budget: they are a second copy of (or, for a delete, no
/// copy of) the resident bytes, held in memory until `take_pending` hands them off.
#[test]
fn pending_write_bytes_count_against_the_budget() {
    let mut store = MirrorStore::new(DefaultTierPolicy, 100);
    store.put(StoreKey::new("files/one"), vec![9u8; 60]);
    // Resident (60) + pending copy (60) = 120% of budget, even though nothing was evicted.
    assert!(store.over_budget(), "a pending write's byte copy must count against the budget alongside its resident copy");
    store.take_pending();
    assert!(!store.over_budget(), "draining pending must release its share of the budget");
}
