//! Hostile-input hammer: truncation, bit flips and extreme field values must never panic or hang.

use super::*;
use crate::mxf::klv::ber_length;
use crate::mxf::meta::{Primer, Set};

fn exercise(d: &[u8]) {
    let Ok(m) = open(d) else { return };
    let _ = m.operational_pattern.name();
    for p in &m.partitions {
        let _ = (p.closed(), p.complete());
    }
    for (ti, t) in m.tracks.iter().enumerate() {
        let n = t.samples.len();
        for i in [0, n / 2, n.saturating_sub(1), n, usize::MAX] {
            let _ = m.read_sample(d, ti, i);
            let _ = t.sync_before(i);
        }
        for s in [0u64, 100, 48_000, u64::MAX] {
            for f in [0usize, 1, 1920, 1 << 20] {
                let _ = m.read_pcm(d, ti, s, f);
            }
        }
        for x in [i64::MIN, -1, 0, 5, i64::MAX] {
            let _ = t.sample_at(x);
        }
        let _ = (t.first_edit_unit(), t.stored_sample_frames(), t.samples_per_edit_unit());
        let _ = t.sound.as_ref().map(|s| s.frame_bytes());
    }
    if let Some(tc) = &m.timecode {
        let _ = tc.format();
    }
}

/// The value of the first KLV triplet with the given key prefix.
fn value_of<'a>(file: &'a [u8], key: &[u8]) -> Option<&'a [u8]> {
    let at = file.windows(key.len()).position(|w| w == key)?;
    let (len, n) = ber_length(file.get(at + 16..)?)?;
    file.get(at + 16 + n..)?.get(..usize::try_from(len).ok()?)
}

/// Regression: `sync_before` indexed `samples[0]` on a track with no samples.
#[test]
fn sync_before_on_an_empty_track() {
    let (to, flags) = ipbb();
    let mut m = open(&build(5, prores_like, &to, &flags)).unwrap();
    for t in &mut m.tracks {
        t.samples.clear();
        assert_eq!(t.sync_before(0), 0);
        assert_eq!(t.sync_before(usize::MAX), 0);
    }
}

// Keep each deterministic hammer independent so test runners can schedule the seeds separately.
// The labels, mutation counts, every-truncation coverage and per-case watchdog stay unchanged.
fn hostile_seeds() -> [Vec<u8>; 3] {
    let (to, flags) = ipbb();
    [build(7, |i| vec![i as u8; 100 + i], &to, &flags), build(5, prores_like, &to, &flags), build(4, prores_like, &[], &flags)]
}

// Partition parser calls, not the RNG: each original ordinal belongs to exactly one part.
fn partitioned_parser(part: usize, parts: usize, parse: impl Fn(&[u8]) + Send + Sync + 'static) -> impl Fn(&[u8]) + Send + Sync + 'static {
    use std::sync::atomic::{AtomicUsize, Ordering};
    assert!(parts > 0 && part < parts, "invalid hostile-input partition");
    let ordinal = AtomicUsize::new(0);
    move |data| {
        if ordinal.fetch_add(1, Ordering::Relaxed) % parts == part {
            parse(data);
        }
    }
}

// Every truncation and mutation executes exactly once across eight parts per seed.
// Each part regenerates the identical stream, retaining labels and the per-case watchdog.
macro_rules! hostile_seed_parts {
    ($seed:expr, $label:literal, $($name:ident: $part:expr),+ $(,)?) => {
        $(
            #[test]
            fn $name() {
                craft_media_testkit::hostile::hammer(
                    $label, &hostile_seeds()[$seed], 6_000, partitioned_parser($part, 8, exercise),
                );
            }
        )+
    };
}

hostile_seed_parts!(0, "mxf seed 0",
    hostile_input_seed_0_part_0: 0,
    hostile_input_seed_0_part_1: 1,
    hostile_input_seed_0_part_2: 2,
    hostile_input_seed_0_part_3: 3,
    hostile_input_seed_0_part_4: 4,
    hostile_input_seed_0_part_5: 5,
    hostile_input_seed_0_part_6: 6,
    hostile_input_seed_0_part_7: 7,
);

hostile_seed_parts!(1, "mxf seed 1",
    hostile_input_seed_1_part_0: 0,
    hostile_input_seed_1_part_1: 1,
    hostile_input_seed_1_part_2: 2,
    hostile_input_seed_1_part_3: 3,
    hostile_input_seed_1_part_4: 4,
    hostile_input_seed_1_part_5: 5,
    hostile_input_seed_1_part_6: 6,
    hostile_input_seed_1_part_7: 7,
);

hostile_seed_parts!(2, "mxf seed 2",
    hostile_input_seed_2_part_0: 0,
    hostile_input_seed_2_part_1: 1,
    hostile_input_seed_2_part_2: 2,
    hostile_input_seed_2_part_3: 3,
    hostile_input_seed_2_part_4: 4,
    hostile_input_seed_2_part_5: 5,
    hostile_input_seed_2_part_6: 6,
    hostile_input_seed_2_part_7: 7,
);

#[test]
fn hostile_partitions_preserve_exact_order_and_multiplicity() {
    use std::sync::{Arc, Mutex};
    let seed = [0u8, 1, 2, 3];
    for divisor in [1usize, 2, 8] {
        let mut cases = vec![seed.to_vec()];
        cases.extend((0..seed.len()).step_by(divisor).map(|n| seed[..n].to_vec()));
        cases.extend(craft_media_testkit::hostile::mutations(&seed, 37usize.div_ceil(divisor)).map(|m| m.bytes));
        for parts in [1usize, 2, 8, 64] {
            let mut total = 0;
            for part in 0..parts {
                let seen = Arc::new(Mutex::new(Vec::new()));
                let record = Arc::clone(&seen);
                let parse = partitioned_parser(part, parts, move |d| record.lock().unwrap().push(d.to_vec()));
                for case in &cases {
                    parse(case);
                }
                // Sequence equality retains duplicate inputs; a set comparison would not.
                let expected: Vec<_> = cases.iter().skip(part).step_by(parts).cloned().collect();
                assert_eq!(*seen.lock().unwrap(), expected);
                total += expected.len();
            }
            assert_eq!(total, cases.len());
        }
    }
}

#[test]
fn hostile_partitions_reject_invalid_selection() {
    for (part, parts) in [(0, 0), (1, 1), (8, 8)] {
        assert!(std::panic::catch_unwind(|| partitioned_parser(part, parts, |_| {})).is_err());
    }
}

#[test]
fn hostile_index_segment_never_panics() {
    let seed = &hostile_seeds()[0];
    let index_key = [0x06, 0x0E, 0x2B, 0x34, 0x02, 0x53, 0x01, 0x01, 0x0D, 0x01, 0x02, 0x01, 0x01, 0x10, 0x01, 0x00];
    if let Some(v) = value_of(seed, &index_key) {
        craft_media_testkit::hostile::hammer("mxf index segment", v, 30_000, |d| {
            let _ = IndexSegment::parse(d);
        });
    }
}

#[test]
fn hostile_primer_never_panics() {
    let seed = &hostile_seeds()[0];
    let primer_key = [0x06, 0x0E, 0x2B, 0x34, 0x02, 0x05, 0x01, 0x01, 0x0D, 0x01, 0x02, 0x01, 0x01, 0x05, 0x01, 0x00];
    if let Some(v) = value_of(seed, &primer_key) {
        craft_media_testkit::hostile::hammer("mxf primer", v, 5_000, |d| {
            let _ = Primer::parse(d);
        });
    }
}

#[test]
fn hostile_set_never_panics() {
    let seed = &hostile_seeds()[0];
    let set = seed.windows(16).position(|w| w == set_key(0x18)).and_then(|at| value_of(&seed[at..], &set_key(0x18)));
    if let Some(v) = set {
        craft_media_testkit::hostile::hammer("mxf set", v, 20_000, |d| {
            let _ = Set::parse(Ul(set_key(0x18)), d, &Primer::default());
        });
    }
}
