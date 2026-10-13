use craft_cms::{
    Builtin, CacheLimits, Transform, TransformCache, TransformOptions, cache_limits, cache_stats, cached, clear_cache, set_cache_limits,
};

#[test]
fn test_table_bytes() {
    let srgb = Builtin::Srgb.profile();
    let p3 = Builtin::DisplayP3.profile();
    let opts = TransformOptions::default();

    let t = Transform::with_options(srgb, p3, opts).expect("transform");
    assert!(t.table_bytes() > 0);
}

#[test]
fn test_global_lru_cache() {
    clear_cache();
    let initial_limits = cache_limits();
    assert_eq!(cache_stats().entries, 0);
    assert_eq!(cache_stats().total_bytes, 0);

    let srgb = Builtin::Srgb.profile();
    let p3 = Builtin::DisplayP3.profile();
    let adobe = Builtin::AdobeRgbCompat.profile();
    let rec2020 = Builtin::Rec2020.profile();

    let opts = TransformOptions::default();

    // Set cache limit to 2 entries
    set_cache_limits(CacheLimits { max_entries: 2, max_bytes: 64 * 1024 * 1024 });

    // Insert entry 1: sRGB -> P3
    let _ = cached(srgb, p3, opts).expect("cached 1");
    assert_eq!(cache_stats().entries, 1);
    assert!(cache_stats().total_bytes > 0);

    // Insert entry 2: sRGB -> Adobe
    let _ = cached(srgb, adobe, opts).expect("cached 2");
    assert_eq!(cache_stats().entries, 2);

    // Touch entry 1 by querying it again (makes entry 2 the LRU)
    let _ = cached(srgb, p3, opts).expect("cached 1 hit");
    assert_eq!(cache_stats().entries, 2);

    // Insert entry 3: sRGB -> Rec2020 (should evict entry 2)
    let _ = cached(srgb, rec2020, opts).expect("cached 3");
    assert_eq!(cache_stats().entries, 2);

    // Byte budget eviction test: set limit so only 1 transform fits
    let single_bytes = cache_stats().total_bytes / 2;
    set_cache_limits(CacheLimits { max_entries: 10, max_bytes: single_bytes + 10 });
    assert_eq!(cache_stats().entries, 1);

    // Clear cache
    clear_cache();
    assert_eq!(cache_stats().entries, 0);
    assert_eq!(cache_stats().total_bytes, 0);

    // Restore initial limits
    set_cache_limits(initial_limits);
}

#[test]
fn test_isolated_transform_cache_instance() {
    let srgb = Builtin::Srgb.profile();
    let p3 = Builtin::DisplayP3.profile();
    let adobe = Builtin::AdobeRgbCompat.profile();
    let rec2020 = Builtin::Rec2020.profile();
    let opts = TransformOptions::default();

    // Isolated cache with 2-entry capacity
    let local_cache = TransformCache::with_limits(CacheLimits { max_entries: 2, max_bytes: 32 * 1024 * 1024 });
    assert_eq!(local_cache.stats().entries, 0);

    let _ = local_cache.get(srgb, p3, opts).expect("local 1");
    let _ = local_cache.get(srgb, adobe, opts).expect("local 2");
    assert_eq!(local_cache.stats().entries, 2);

    // Touch entry 1
    let _ = local_cache.get(srgb, p3, opts).expect("local 1 hit");

    // Insert entry 3 -> evicts entry 2
    let _ = local_cache.get(srgb, rec2020, opts).expect("local 3");
    assert_eq!(local_cache.stats().entries, 2);

    local_cache.clear();
    assert_eq!(local_cache.stats().entries, 0);
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn test_concurrent_misses_and_contention() {
    use std::sync::Arc;
    use std::thread;

    let cache = Arc::new(TransformCache::with_limits(CacheLimits { max_entries: 4, max_bytes: 64 * 1024 * 1024 }));

    let srgb = Builtin::Srgb.profile();
    let p3 = Builtin::DisplayP3.profile();
    let adobe = Builtin::AdobeRgbCompat.profile();
    let rec2020 = Builtin::Rec2020.profile();
    let opts = TransformOptions::default();

    // 1. Concurrent misses on the same uncached transform
    let mut handles = Vec::new();
    for _ in 0..8 {
        let c = Arc::clone(&cache);
        handles.push(thread::spawn(move || {
            let t = c.get(srgb, p3, opts).expect("get transform concurrently");
            let mut out = [0.0f32; 3];
            t.eval(&[0.5, 0.5, 0.5], &mut out).expect("eval");
            assert!(out[0] > 0.0);
        }));
    }
    for h in handles {
        h.join().expect("thread join");
    }

    // Cache should contain exactly 1 entry despite concurrent misses
    assert_eq!(cache.stats().entries, 1);
    assert!(cache.stats().total_bytes > 0);

    // 2. High contention concurrent queries across multiple transforms with strict entry and byte bounds
    let tight_cache = Arc::new(TransformCache::with_limits(CacheLimits { max_entries: 2, max_bytes: 2 * 1024 * 1024 }));

    let mut contention_handles = Vec::new();
    let profiles = [srgb, p3, adobe, rec2020];

    for id in 0..16 {
        let c = Arc::clone(&tight_cache);
        let src = profiles[id % profiles.len()];
        let dst = profiles[(id + 1) % profiles.len()];
        contention_handles.push(thread::spawn(move || {
            for _ in 0..10 {
                let t = c.get(src, dst, opts).expect("contention get");
                let mut out = [0.0f32; 3];
                t.eval(&[0.2, 0.4, 0.6], &mut out).expect("eval under contention");
            }
        }));
    }

    for h in contention_handles {
        h.join().expect("contention thread join");
    }

    let stats = tight_cache.stats();
    assert!(stats.entries <= 2, "entries ({}) exceeded max_entries (2)", stats.entries);
    assert!(stats.total_bytes <= 2 * 1024 * 1024, "total_bytes ({}) exceeded max_bytes", stats.total_bytes);
}

fn make_lut16_profile_65() -> craft_cms::Profile {
    let mut lut = b"mft2\0\0\0\0".to_vec();
    lut.extend_from_slice(&[3, 3, 65, 0]);
    for k in 0..9 {
        let v: i32 = if k % 4 == 0 { 0x10000 } else { 0 };
        lut.extend_from_slice(&v.to_be_bytes());
    }
    lut.extend_from_slice(&[0, 2, 0, 2]);
    for _ in 0..3 {
        lut.extend_from_slice(&[0, 0, 0xff, 0xff]);
    }
    let clut_vals = 65usize * 65 * 65 * 3;
    lut.resize(lut.len() + clut_vals * 2, 0x80);
    for _ in 0..3 {
        lut.extend_from_slice(&[0, 0, 0xff, 0xff]);
    }

    let mut icc = vec![0u8; 128];
    icc[8] = 4;
    icc[12..16].copy_from_slice(b"mntr");
    icc[16..20].copy_from_slice(b"RGB ");
    icc[20..24].copy_from_slice(b"XYZ ");
    icc[36..40].copy_from_slice(b"acsp");
    icc.extend_from_slice(&1u32.to_be_bytes());
    icc.extend_from_slice(b"A2B1");
    icc.extend_from_slice(&144u32.to_be_bytes());
    icc.extend_from_slice(&(lut.len() as u32).to_be_bytes());
    icc.extend_from_slice(&lut);
    let size = icc.len() as u32;
    icc[..4].copy_from_slice(&size.to_be_bytes());

    craft_cms::Profile::parse(&icc).expect("valid profile with 65x65x65 lut16")
}

#[test]
fn test_retained_pipeline_clut_memory_accounting_and_eviction() {
    let src = make_lut16_profile_65();
    let dst = Builtin::Srgb.profile();
    let opts = TransformOptions::default();

    let transform = Transform::with_options(&src, dst, opts).expect("transform");
    let table_bytes = transform.table_bytes();

    // Verify accounting includes the retained 65^3 * 3 * 4 = 3,295,500 bytes CLUT from the pipeline
    assert!(table_bytes >= 3_295_500, "table_bytes ({table_bytes}) must account for retained pipeline CLUT data (at least 3,295,500 bytes)");

    // Cache with configured limit of 1 MiB (1,048,576 bytes) as reported by @euisuh
    let limit_bytes = 1_048_576;
    let cache = TransformCache::with_limits(CacheLimits { max_entries: 10, max_bytes: limit_bytes });

    // The transform exceeds the 1 MiB budget (it is ~3.7 MiB)
    let t = cache.get(&src, dst, opts).expect("get transform");
    assert!(t.table_bytes() >= 3_295_500);

    // The transform must not remain resident in cache because it exceeds max_bytes
    let stats = cache.stats();
    assert_eq!(stats.entries, 0, "oversized transform must not remain resident in cache");
    assert_eq!(stats.total_bytes, 0, "reported cache memory must be 0 when entry exceeds byte budget");

    // When cache byte budget is large enough to accommodate the transform (e.g. 8 MiB)
    cache.set_limits(CacheLimits { max_entries: 10, max_bytes: 8 * 1024 * 1024 });
    let t2 = cache.get(&src, dst, opts).expect("get transform with sufficient budget");
    let stats2 = cache.stats();
    assert_eq!(stats2.entries, 1);
    assert_eq!(stats2.total_bytes, t2.table_bytes());

    // Reducing the limit back to 1 MiB should evict the oversized transform
    cache.set_limits(CacheLimits { max_entries: 10, max_bytes: limit_bytes });
    let stats3 = cache.stats();
    assert_eq!(stats3.entries, 0);
    assert_eq!(stats3.total_bytes, 0);
}
