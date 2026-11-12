//! Hostile-input robustness: truncation, header-region sweeps and random mutation of small
//! ffmpeg-generated VP8 streams (frame by frame) must never panic or run away. Skips without ffmpeg.

use std::cell::Cell;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::process::{Command, Stdio};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use craft_video::vp8::Decoder;

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

/// (name, encoder args, pix_fmt)
const SEEDS: &[(&str, &str, &str)] = &[
    ("vp8_fz_default", "-c:v libvpx -g 4 -b:v 300k", "yuv420p"),
    ("vp8_fz_partitions", "-c:v libvpx -g 6 -b:v 300k -slices 8 -auto-alt-ref 0 -lag-in-frames 0 -error-resilient 1", "yuv420p"),
];

fn ivf_frames(data: &[u8]) -> Vec<Vec<u8>> {
    let mut out = Vec::new();
    let Some(h) = data.get(6..8) else { return out };
    let mut pos = usize::from(u16::from_le_bytes([h[0], h[1]]));
    while let Some(hd) = data.get(pos..pos + 12) {
        let size = u32::from_le_bytes([hd[0], hd[1], hd[2], hd[3]]) as usize;
        pos += 12;
        let end = pos.saturating_add(size).min(data.len());
        out.push(data[pos..end].to_vec());
        pos = end;
    }
    out
}

fn make(ff: &std::path::Path, name: &str, args: &str, pix: &str) -> Option<Vec<Vec<u8>>> {
    let out = craft_media_testkit::fixtures_dir("vp8").join(format!("{name}.ivf"));
    let p = craft_media_testkit::fixtures::generate(&out, |tmp| {
        let a: Vec<&str> = args.split_whitespace().collect();
        matches!(
            Command::new(ff)
                .args(["-hide_banner", "-loglevel", "error", "-y", "-f", "lavfi", "-i", "testsrc2=size=64x48:rate=25", "-frames:v", "6", "-pix_fmt", pix])
                .args(a)
                .args(["-f", "ivf"])
                .arg(tmp)
                .stdin(Stdio::null())
                .status(),
            Ok(s) if s.success()
        )
    })?;
    Some(ivf_frames(&std::fs::read(p).ok()?))
}

static PANICS: AtomicUsize = AtomicUsize::new(0);

thread_local! {
    static IN_FUZZ_RUN: Cell<bool> = const { Cell::new(false) };
}

fn with_panic_accounting(f: impl FnOnce()) {
    let previous = Arc::new(std::panic::take_hook());
    let counted_hook = Arc::clone(&previous);
    std::panic::set_hook(Box::new(move |info| {
        if IN_FUZZ_RUN.with(Cell::get) {
            PANICS.fetch_add(1, Ordering::SeqCst);
        }
        counted_hook(info);
    }));
    let result = catch_unwind(AssertUnwindSafe(f));
    let restored_hook = Arc::clone(&previous);
    std::panic::set_hook(Box::new(move |info| restored_hook(info)));
    if let Err(payload) = result {
        std::panic::resume_unwind(payload);
    }
}

fn count_fuzz_run_panics(f: impl FnOnce()) -> std::thread::Result<()> {
    IN_FUZZ_RUN.with(|active| active.set(true));
    let result = catch_unwind(AssertUnwindSafe(f));
    IN_FUZZ_RUN.with(|active| active.set(false));
    result
}

fn assert_no_fuzz_panics(res: std::thread::Result<()>, before: usize, label: &str) {
    assert!(res.is_ok() && PANICS.load(Ordering::SeqCst) == before, "panic: {label}");
}

fn run_all(frames: &[Vec<u8>], label: &str) {
    craft_media_testkit::set_progress(label);
    let before = PANICS.load(Ordering::SeqCst);
    let res = count_fuzz_run_panics(|| {
        let mut dec = Decoder::new();
        for (i, f) in frames.iter().enumerate() {
            let _ = dec.decode(f, i as i64);
        }
    });
    assert_no_fuzz_panics(res, before, label);
}

/// Random-mutation rounds per seed (raise with FILMCRAFT_FUZZ_ITERS for a long hunt).
fn iters() -> usize {
    std::env::var("FILMCRAFT_FUZZ_ITERS").ok().and_then(|s| s.parse().ok()).unwrap_or(300)
}

/// Seconds the whole sweep may take before it counts as a hang (override: CRAFT_TEST_TIMEOUT_SECS).
const DEADLINE_SECS: u64 = 600;

#[test]
fn hostile_streams_never_panic() {
    with_panic_accounting(|| craft_media_testkit::with_deadline(DEADLINE_SECS, sweep));
}

fn sweep() {
    let Some(ff) = craft_media_testkit::ffmpeg_or_skip("VP8 fuzz") else { return };
    for &(name, args, pix) in SEEDS {
        if !craft_media_testkit::ffmpeg_has_encoder(&ff, "libvpx") {
            craft_media_testkit::skip_test("VP8 fuzz", &format!("this ffmpeg has no libvpx encoder (seed {name})"));
            continue;
        }
        let Some(seed) = make(&ff, name, args, pix) else {
            panic!("ffmpeg has libvpx but could not encode the {name} seed");
        };
        if seed.is_empty() {
            craft_media_testkit::skip_test("VP8 fuzz", &format!("the {name} seed has no frames"));
            continue;
        }
        // truncate each of the first frames at every length
        for fi in 0..seed.len().min(3) {
            let len = seed[fi].len();
            for n in (0..len).step_by((len / 200).max(1)) {
                let mut d = seed.clone();
                d[fi].truncate(n);
                run_all(&d, &format!("{name} frame {fi} truncate {n}"));
            }
        }
        // header region of every frame: every byte to a few values, every bit
        for fi in 0..seed.len() {
            for p in 0..seed[fi].len().min(96) {
                for v in [0u8, 0xff, 0x80, 0x01, 0x7f] {
                    let mut d = seed.clone();
                    d[fi][p] = v;
                    run_all(&d, &format!("{name} frame {fi} byte {p}={v:#x}"));
                }
                for b in 0..8 {
                    let mut d = seed.clone();
                    d[fi][p] ^= 1 << b;
                    run_all(&d, &format!("{name} frame {fi} flip {p}.{b}"));
                }
            }
        }
        let mut rng = Rng(0x9e3779b97f4a7c15 ^ name.len() as u64);
        for i in 0..iters() {
            let mut d = seed.clone();
            for _ in 0..1 + rng.below(6) {
                let fi = rng.below(d.len());
                if d[fi].is_empty() {
                    continue;
                }
                let p = rng.below(d[fi].len());
                if rng.below(2) == 0 {
                    d[fi][p] ^= 1 << rng.below(8);
                } else {
                    d[fi][p] = rng.next() as u8;
                }
            }
            run_all(&d, &format!("{name} random {i}"));
        }
    }
}
