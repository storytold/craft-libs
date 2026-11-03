//! The transform cache under concurrent use (#63, noted in review): threads that share it get the
//! transforms they asked for, its byte budget holds, and two threads building the same transform
//! at once do not count its bytes twice.
//!
//! A test binary of its own, with one test: the cache is process-wide, so a second test running
//! beside this one would move the numbers it checks.

use craft_cms::{Builtin, Intent, MAX_CACHED, Profile, TransformOptions, cache_budget, cache_usage, cached, clear_cache, set_cache_budget};

fn profile(b: Builtin) -> &'static Profile {
    b.profile()
}

/// The transforms the threads share: (source, destination, intent, input channels, output channels).
fn keys() -> Vec<(Builtin, Builtin, Intent, usize, usize)> {
    vec![
        (Builtin::Srgb, Builtin::LabD50, Intent::RelativeColorimetric, 3, 3),
        (Builtin::Srgb, Builtin::LabD50, Intent::Saturation, 3, 3),
        (Builtin::Srgb, Builtin::LabD50, Intent::Perceptual, 3, 3),
        (Builtin::CoatedCmyk, Builtin::Srgb, Intent::Perceptual, 4, 3),
        (Builtin::Srgb, Builtin::CoatedCmyk, Intent::RelativeColorimetric, 3, 4),
    ]
}

fn eval(k: &(Builtin, Builtin, Intent, usize, usize)) -> (std::sync::Arc<craft_cms::Transform>, Vec<f32>) {
    let (src, dst, intent, n_in, n_out) = *k;
    let t = cached(profile(src), profile(dst), TransformOptions { intent, ..Default::default() }).unwrap();
    let input: Vec<f32> = (0..n_in).map(|i| 0.15 + 0.2 * i as f32).collect();
    let mut out = vec![0.0f32; n_out];
    t.eval(&input, &mut out).unwrap();
    (t, out)
}

#[test]
fn the_transform_cache_is_shared_safely_between_threads() {
    let keys = keys();
    let saved = cache_budget();

    // Each transform's size and result, built one at a time.
    clear_cache();
    set_cache_budget(usize::MAX);
    let alone: Vec<(usize, Vec<f32>)> = keys.iter().map(|k| eval(k)).map(|(t, out)| (t.table_bytes(), out)).collect();
    let total: usize = alone.iter().map(|(b, _)| b).sum();
    assert_eq!(cache_usage(), (keys.len(), total), "built one at a time, every transform is counted once");

    // 1. Eight threads over a budget that holds about two of the small ones: every call gets the
    //    transform it asked for (its result is the one built alone), while the cache evicts
    //    beneath them.
    clear_cache();
    let budget = alone[0].0 * 2 + alone[0].0 / 2;
    set_cache_budget(budget);
    std::thread::scope(|s| {
        for thread in 0..8 {
            let (keys, alone) = (&keys, &alone);
            s.spawn(move || {
                for i in 0..60 {
                    let which = (thread + i) % keys.len();
                    let (_, out) = eval(&keys[which]);
                    assert_eq!(out, alone[which].1, "thread {thread}, call {i}: key {which}");
                }
            });
        }
    });
    let (entries, bytes) = cache_usage();
    assert!((1..=MAX_CACHED).contains(&entries), "{entries} entries");
    assert!(bytes <= budget || entries == 1, "{bytes} bytes in {entries} entries over a budget of {budget}");

    // 2. Every thread asks for every transform at once, on an empty cache with room for all of
    //    them: several threads build the same transform and insert it, and the bytes are still
    //    each transform's once.
    clear_cache();
    set_cache_budget(usize::MAX);
    let start = std::sync::Barrier::new(8);
    std::thread::scope(|s| {
        for _ in 0..8 {
            let (keys, start) = (&keys, &start);
            s.spawn(move || {
                start.wait();
                for k in keys {
                    eval(k);
                }
            });
        }
    });
    assert_eq!(cache_usage(), (keys.len(), total), "a transform built by two threads is counted once");

    set_cache_budget(saved);
    clear_cache();
}
