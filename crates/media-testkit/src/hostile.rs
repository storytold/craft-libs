//! Deterministic hostile-input hammer: truncations, bit flips and extreme-value overwrites of a
//! valid seed, each run under `catch_unwind` with a time budget. No dependencies, no `unsafe`.

use std::fmt::Write as _;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::time::{Duration, Instant};

/// xorshift64* PRNG.
pub struct Rng(pub u64);

impl Rng {
    /// The next value.
    pub fn next_u64(&mut self) -> u64 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }
    /// A value below `n` (0 for `n` = 0).
    pub fn below(&mut self, n: usize) -> usize {
        (self.next_u64() % n.max(1) as u64) as usize
    }
}

const EXTREMES: [u64; 8] = [0, 1, 0x7F, 0x80, 0xFF, 0xFFFF, 0x8000_0000, u64::MAX];

fn run_one(name: &str, what: &str, data: &[u8], run: &dyn Fn(&[u8])) {
    let t = Instant::now();
    let r = catch_unwind(AssertUnwindSafe(|| run(data)));
    let el = t.elapsed();
    assert!(r.is_ok(), "{name}: PANIC on {what} (len {})", data.len());
    assert!(el < Duration::from_secs(5), "{name}: HANG/slow ({el:?}) on {what} (len {})", data.len());
}

/// Run `run` over every truncation of `seed` (stepped beyond 4096 lengths), then `iters` random
/// mutations (bit flips, random bytes, 1/2/4/8-byte extreme-value overwrites in both
/// endiannesses, deletions, truncation). Panics, naming the mutation, on the first panic or on
/// a run slower than 5 s.
pub fn hammer(name: &str, seed: &[u8], iters: usize, run: impl Fn(&[u8])) {
    run_one(name, "the unmodified seed", seed, &run);
    let step = (seed.len() / 4096).max(1);
    for n in (0..seed.len()).step_by(step) {
        run_one(name, &format!("truncation to {n}"), &seed[..n], &run);
    }
    let mut rng = Rng((0x5EED_F00D ^ seed.len() as u64) | 1);
    // CRAFT_MEDIA_HOSTILE_MULT=10 runs ten times the iterations (soak runs).
    let mult = std::env::var("CRAFT_MEDIA_HOSTILE_MULT").ok().and_then(|v| v.parse::<usize>().ok()).unwrap_or(1);
    for _ in 0..iters.saturating_mul(mult) {
        let mut d = seed.to_vec();
        let mut what = String::new();
        for _ in 0..1 + rng.below(4) {
            if d.is_empty() {
                break;
            }
            let pos = rng.below(d.len());
            match rng.below(7) {
                0 => {
                    let b = rng.below(8);
                    d[pos] ^= 1 << b;
                    let _ = write!(what, "flip[{pos}].{b} ");
                }
                1 => {
                    d[pos] = rng.next_u64() as u8;
                    let _ = write!(what, "set[{pos}]={:#x} ", d[pos]);
                }
                2..=4 => {
                    let w = [1usize, 2, 4, 8][rng.below(4)];
                    let v = EXTREMES[rng.below(EXTREMES.len())];
                    let be = rng.below(2) == 0;
                    let bytes = if be { v.to_be_bytes()[8 - w..].to_vec() } else { v.to_le_bytes()[..w].to_vec() };
                    let end = (pos + w).min(d.len());
                    d[pos..end].copy_from_slice(&bytes[..end - pos]);
                    let _ = write!(what, "put{w}{}[{pos}]={v:#x} ", if be { "be" } else { "le" });
                }
                5 => {
                    let n = 1 + rng.below(16);
                    let end = (pos + n).min(d.len());
                    d.drain(pos..end);
                    let _ = write!(what, "del[{pos}..{end}] ");
                }
                _ => {
                    d.truncate(pos);
                    let _ = write!(what, "trunc{pos} ");
                }
            }
        }
        run_one(name, &what, &d, &run);
    }
}
