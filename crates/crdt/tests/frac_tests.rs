//! (Test code: assertions may panic — that is their point.)
#![allow(clippy::expect_used, clippy::unwrap_used, clippy::panic)]

//! Fractional-indexing property tests (spec §5.2 P1-P3). Mirrors figcraft's `frac_tests.rs`.

use craft_crdt::frac;

/// P1: any sequence of concurrent inserts produces the same final order everywhere — here, the
/// deterministic generation means equal keys can't happen, but the order invariant must hold.
#[test]
fn between_always_orders() {
    let mut keys = vec![frac::FIRST_KEY.to_string()];
    for _ in 0..500 {
        // Insert at a random-ish position between existing keys.
        let i = keys.len() / 3;
        let prev = keys.get(i.wrapping_sub(1)).cloned();
        let next = keys.get(i).cloned();
        let k = frac::between(prev.as_deref(), next.as_deref()).expect("a key fits");
        keys.insert(i, k);
    }
    // The sequence must be strictly increasing.
    for w in keys.windows(2) {
        assert!(w[0] < w[1], "order broken: {} !< {}", w[0], w[1]);
    }
}

/// P2: append-only keys stay short (logarithmic growth): 100k appends stay under 8 chars.
#[test]
fn append_keys_stay_short() {
    let mut k = frac::FIRST_KEY.to_string();
    let mut longest = 0usize;
    for _ in 0..100_000 {
        let next = frac::between(Some(&k), None).expect("room above");
        longest = longest.max(next.len());
        k = next;
    }
    assert!(longest <= 8, "appending grew keys to {longest} chars");
}

/// Spec §5.2's own examples: between `a1` and `a2` sits `a1V`; the first key is `a0`.
#[test]
fn spec_examples() {
    assert_eq!(frac::between(None, None).expect("first"), "a0");
    let mid = frac::between(Some("a1"), Some("a2")).expect("mid");
    assert_eq!(mid, "a1V");
    assert!(frac::between(Some("a2"), Some("a1")).is_err(), "bounds must be ordered");
}

/// Generated **fractions** never end in `0` — the invariant that keeps room between keys
/// (integer parts may: `a0` and `b00` are integer keys, and valid ones).
#[test]
fn fractions_never_end_in_zero() {
    let mut k = frac::FIRST_KEY.to_string();
    for _ in 0..1_000 {
        let next = frac::between(Some(&k), None).expect("room");
        let mid = frac::between(Some(&k), Some(&next)).expect("mid");
        // The mid key's fraction part is what never ends in 0 (validate enforces it).
        assert!(frac::validate(&mid).is_ok(), "mid key must be valid: {mid}");
        assert!(!mid.ends_with('0') || mid.parse::<i64>().is_ok(), "mid {mid}");
        assert!(frac::validate(&next).is_ok(), "key must be valid: {next}");
        k = next;
    }
}

/// `n_between` yields exactly n keys, strictly between, in order.
#[test]
fn n_between_splits_evenly() {
    let keys = frac::n_between(Some("a1"), Some("a2"), 7).expect("keys");
    assert_eq!(keys.len(), 7);
    for w in keys.windows(2) {
        assert!(w[0] < w[1]);
    }
    assert!(keys[0].as_str() > "a1");
    assert!(keys[6].as_str() < "a2");
}

/// Malformed keys are rejected by validate.
#[test]
fn validation_rejects_malformed() {
    for bad in ["", "0", "a0\n", "zz", "a!", &"x".repeat(200), "a10", "z11"] {
        assert!(frac::validate(bad).is_err(), "{bad:?} should be invalid");
    }
    for good in ["a0", "a1V", "A000000000000000000000000000z", "b00"] {
        assert!(frac::validate(good).is_ok(), "{good:?} should be valid");
    }
}

/// Respace only fires near the threshold and keeps the order monotone (spec §5.2).
#[test]
fn respace_keeps_order_monotone() {
    // Children with one over-long key.
    let long_key = format!("a1{}", "V".repeat(60));
    let keys = vec![(frac::FIRST_KEY.to_string(), 1u64), (long_key, 2u64), ("a3".to_string(), 3u64)];
    let pairs = frac::respace(&keys).expect("respace");
    assert_eq!(pairs.len(), 1, "only the long key moves");
    // The untouched keys stay; the sequence remains sorted.
    let mut seq: Vec<String> = keys.iter().map(|(k, _)| k.clone()).collect();
    for (old, new) in &pairs {
        if let Some(p) = seq.iter_mut().find(|k| *k == old) {
            *p = new.clone();
        }
    }
    assert!(seq.is_sorted(), "order must survive respace: {seq:?}");
    // Short keys alone trigger nothing.
    let short = vec![("a0".to_string(), 1u64), ("a1".to_string(), 2u64)];
    assert_eq!(frac::respace(&short).expect("respace"), [] as [(std::string::String, std::string::String); 0]);
}
