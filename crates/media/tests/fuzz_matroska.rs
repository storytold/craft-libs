//! Deterministic mutation fuzzing of the Matroska demuxer: it must never panic on corrupted input.

use craft_media::matroska::*;
use std::io::Cursor;

struct Rng(u64);
impl Rng {
    fn next(&mut self) -> u64 {
        // xorshift64*
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }
    fn below(&mut self, n: usize) -> usize {
        (self.next() % n.max(1) as u64) as usize
    }
}

fn synthetic_files() -> Vec<Vec<u8>> {
    let mut out = Vec::new();
    for lacing in [LacingMode::None, LacingMode::Xiph, LacingMode::Ebml, LacingMode::FixedOrEbml] {
        let mut v = TrackSpec::new(TrackKind::Video, "V_MJPEG");
        v.video_size = Some((16, 16));
        v.default_duration_ns = Some(40_000_000);
        let mut a = TrackSpec::new(TrackKind::Audio, "A_PCM/INT/LIT");
        a.audio = Some((48000.0, 2, Some(16)));
        a.default_duration_ns = Some(10_000_000);
        let mut s = TrackSpec::new(TrackKind::Subtitle, "S_TEXT/UTF8");
        s.language = Some("ger".into());
        let opts = MuxOptions { lacing, cluster_duration_ns: 200_000_000, title: Some("fuzz".into()), ..Default::default() };
        let mut w = MkvWriter::new(Cursor::new(Vec::new()), vec![v, a, s], opts).unwrap();
        for i in 0..20i64 {
            w.write_frame(0, i * 40_000_000, i % 5 == 0, &[i as u8; 24], None).unwrap();
            for k in 0..4 {
                let size = if k == 3 { 7 } else { 40 };
                w.write_frame(1, i * 40_000_000 + k * 10_000_000, true, &vec![k as u8; size], None).unwrap();
            }
            if i % 7 == 0 {
                w.write_frame(2, i * 40_000_000, true, b"text", Some(120_000_000)).unwrap();
            }
        }
        out.push(w.finish().unwrap().into_inner());
    }
    out
}

fn exercise(data: &[u8]) -> bool {
    let mut parsed = false;
    for opts in [OpenOptions { index: true, verify_crc: true }, OpenOptions { index: false, verify_crc: false }] {
        let Ok(f) = open_with(data, &opts) else { continue };
        parsed = true;
        let _ = f.duration_ns();
        for (ti, t) in f.tracks.iter().enumerate() {
            let _ = t.video.as_ref().map(|v| (v.pixel_aspect(), v.display_rotation()));
            let n = t.samples.len();
            for i in [0, n / 2, n.saturating_sub(1)] {
                let _ = f.read_sample(data, ti, i);
                let _ = f.read_sample_addition(data, ti, i);
                let _ = t.sync_sample_before(i);
            }
            for time in [i64::MIN, -1, 0, 500_000_000, i64::MAX] {
                let _ = f.keyframe_before(ti, time);
                let _ = t.sample_at_pts(t.ns_to_ticks(time));
            }
        }
        let Ok(mut d) = Demuxer::with_options(data, opts) else { continue };
        for p in d.by_ref().take(10_000) {
            if p.is_err() {
                break;
            }
        }
        for track in 0..d.tracks().len() {
            for time in [i64::MIN, 0, 300_000_000, i64::MAX] {
                if d.seek(track, time).is_ok() {
                    let _ = d.next_packet();
                }
            }
            let _ = d.keyframe_index(track);
        }
    }
    parsed
}

fn mutate_all(base: &[u8], seed: u64, iterations: usize) {
    let mut rng = Rng(seed | 1);
    let mut parsed = 0usize;
    for _ in 0..iterations {
        let mut d = base.to_vec();
        for _ in 0..1 + rng.below(4) {
            let pos = rng.below(d.len());
            match rng.below(6) {
                0 => d[pos] ^= 1 << rng.below(8),
                1 => d[pos] = rng.next() as u8,
                2 | 3 => {
                    // An extreme value, often landing on an EBML size or a timestamp.
                    let v: u64 = match rng.below(5) {
                        0 => 0,
                        1 => 0x01FF_FFFF_FFFF_FFFF,
                        2 => u64::MAX,
                        3 => 0x0100_0000_0000_0000 | (rng.next() >> 8),
                        _ => rng.next(),
                    };
                    let end = (pos + 8).min(d.len());
                    d[pos..end].copy_from_slice(&v.to_be_bytes()[..end - pos]);
                }
                4 => d.truncate(pos.max(4)),
                _ => {
                    let n = rng.below(16).min(d.len() - pos);
                    d.drain(pos..pos + n);
                }
            }
            if d.is_empty() {
                break;
            }
        }
        parsed += exercise(&d) as usize;
    }
    eprintln!("seed {seed:#x}: {parsed}/{iterations} mutated files still parsed");
}

#[test]
fn mutated_matroska_files_never_panic() {
    for (i, f) in synthetic_files().iter().enumerate() {
        assert!(exercise(f));
        mutate_all(f, 0x4D4B_0000 + i as u64, 2000);
    }
}

#[test]
fn matroska_truncations_never_panic() {
    for f in synthetic_files().iter().take(1) {
        for n in 0..f.len() {
            exercise(&f[..n]);
        }
    }
}
