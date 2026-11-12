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

#[test]
fn hostile_input_never_panics() {
    let (to, flags) = ipbb();
    let seeds = [build(7, |i| vec![i as u8; 100 + i], &to, &flags), build(5, prores_like, &to, &flags), build(4, prores_like, &[], &flags)];
    for (i, s) in seeds.iter().enumerate() {
        craft_media_testkit::hostile::hammer(&format!("mxf seed {i}"), s, 6_000, exercise);
    }
    // The metadata sets and index segments, parsed directly.
    let seed = &seeds[0];
    let index_key = [0x06, 0x0E, 0x2B, 0x34, 0x02, 0x53, 0x01, 0x01, 0x0D, 0x01, 0x02, 0x01, 0x01, 0x10, 0x01, 0x00];
    if let Some(v) = value_of(seed, &index_key) {
        craft_media_testkit::hostile::hammer("mxf index segment", v, 30_000, |d| {
            let _ = IndexSegment::parse(d);
        });
    }
    let primer_key = [0x06, 0x0E, 0x2B, 0x34, 0x02, 0x05, 0x01, 0x01, 0x0D, 0x01, 0x02, 0x01, 0x01, 0x05, 0x01, 0x00];
    if let Some(v) = value_of(seed, &primer_key) {
        craft_media_testkit::hostile::hammer("mxf primer", v, 5_000, |d| {
            let _ = Primer::parse(d);
        });
    }
    let set = seed.windows(16).position(|w| w == set_key(0x18)).and_then(|at| value_of(&seed[at..], &set_key(0x18)));
    if let Some(v) = set {
        craft_media_testkit::hostile::hammer("mxf set", v, 20_000, |d| {
            let _ = Set::parse(Ul(set_key(0x18)), d, &Primer::default());
        });
    }
}
