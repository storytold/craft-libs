#![allow(clippy::expect_used)]

use crate::{DefaultTierPolicy, ExternalVolume, MirrorStore, StoreKey};

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
fn external_volume_serde_roundtrip() {
    let vol = ExternalVolume::new("cold-root-7").with_label("Exports");
    let json = serde_json::to_string(&vol).expect("serialize");
    let back: ExternalVolume = serde_json::from_str(&json).expect("deserialize");
    assert_eq!(back, vol);
}
