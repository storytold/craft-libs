//! Hostile-input robustness for ProRes: truncation, header corruption,
//! and random bit mutations must never panic or crash the decoder.

use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::atomic::{AtomicUsize, Ordering};

use craft_video::prores::{ChromaFormat, DecodeOptions, Encoder, EncoderConfig, Frame, Profile, decode_frame, decode_frame_with, probe};

struct Rng(u64);
impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
    fn below(&mut self, n: usize) -> usize {
        (self.next() % n.max(1) as u64) as usize
    }
}

static PANICS: AtomicUsize = AtomicUsize::new(0);

fn count_panics() {
    static ONCE: std::sync::Once = std::sync::Once::new();
    ONCE.call_once(|| {
        std::panic::set_hook(Box::new(|info| {
            PANICS.fetch_add(1, Ordering::SeqCst);
            eprintln!("PANIC: {info}");
        }));
    });
}

fn run_one(data: &[u8], label: &str) {
    craft_media_testkit::set_progress(label);
    count_panics();
    let before = PANICS.load(Ordering::SeqCst);
    let res = catch_unwind(AssertUnwindSafe(|| {
        let _ = probe(data);
        let _ = decode_frame(data);
        let opts = DecodeOptions { bit_depth: Some(10), threads: false };
        let _ = decode_frame_with(data, &opts);
    }));
    assert!(res.is_ok() && PANICS.load(Ordering::SeqCst) == before, "panic on input: {label}");
}

fn make_seeds() -> Vec<(&'static str, Vec<u8>)> {
    let mut seeds = Vec::new();

    // Seed 1: ProRes Standard 422
    let (w, h) = (64, 32);
    let mut frame = Frame::new(w, h, ChromaFormat::Yuv422, 10, false).unwrap();
    frame.y.fill(512);
    frame.cb.fill(512);
    frame.cr.fill(512);
    let mut enc = Encoder::new(Profile::Standard, w, h);
    if let Ok(data) = enc.encode(&frame) {
        seeds.push(("prores_std_422", data));
    }

    // Seed 2: ProRes 4444 with alpha
    let (w, h) = (64, 32);
    let mut frame = Frame::new(w, h, ChromaFormat::Yuv444, 12, true).unwrap();
    frame.y.fill(2048);
    frame.cb.fill(2048);
    frame.cr.fill(2048);
    if let Some(alpha) = frame.alpha.as_mut() {
        alpha.fill(4095);
    }
    let mut cfg = EncoderConfig::new(Profile::P4444, w, h);
    cfg.encode_alpha = true;
    let mut enc = Encoder::with_config(cfg);
    if let Ok(data) = enc.encode(&frame) {
        seeds.push(("prores_4444_alpha", data));
    }

    seeds
}

const DEADLINE_SECS: u64 = 300;

#[test]
fn hostile_prores_streams_never_panic() {
    craft_media_testkit::with_deadline(DEADLINE_SECS, sweep);
}

fn sweep() {
    let seeds = make_seeds();
    assert!(!seeds.is_empty(), "failed to generate initial prores seeds");

    for (name, seed) in &seeds {
        // 1. Truncate at every offset
        let len = seed.len();
        for n in (0..len).step_by((len / 150).max(1)) {
            let truncated = &seed[..n];
            run_one(truncated, &format!("{name} truncate {n}"));
        }

        // 2. Sweep header region (first 128 bytes)
        let header_len = seed.len().min(128);
        for p in 0..header_len {
            for v in [0u8, 0xff, 0x80, 0x01, 0x7f, 0x55] {
                let mut mutated = seed.clone();
                mutated[p] = v;
                run_one(&mutated, &format!("{name} byte {p}={v:#x}"));
            }
            for b in 0..8 {
                let mut mutated = seed.clone();
                mutated[p] ^= 1 << b;
                run_one(&mutated, &format!("{name} flip {p}.{b}"));
            }
        }

        // 3. Random mutations
        let mut rng = Rng(0xdeadbeef ^ name.len() as u64);
        let iters = std::env::var("FILMCRAFT_FUZZ_ITERS").ok().and_then(|s| s.parse().ok()).unwrap_or(200);

        for i in 0..iters {
            let mut mutated = seed.clone();
            let count = 1 + rng.below(8);
            for _ in 0..count {
                let p = rng.below(mutated.len());
                if rng.below(2) == 0 {
                    mutated[p] ^= 1 << rng.below(8);
                } else {
                    mutated[p] = rng.next() as u8;
                }
            }
            run_one(&mutated, &format!("{name} random {i}"));
        }
    }

    // 4. Also test empty and garbage inputs
    run_one(&[], "empty input");
    run_one(&[0u8; 16], "all zeros");
    run_one(&[0xffu8; 64], "all ones");
}
