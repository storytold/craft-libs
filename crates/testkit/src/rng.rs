//! Deterministic generators for test input: seeded, repeatable, and not suitable for cryptography.
//!
//! [`XorShift64`] is Marsaglia's xorshift64 with the shift triple 13, 7, 17. [`XorShift64Star`] is
//! Vigna's xorshift64*: shifts 12, 25 and 27 on the state, and the output is the state multiplied by
//! `0x2545_F491_4F6C_DD1D`. The state keeps the unmultiplied value.
//!
//! Both generators take a nonzero seed: with a zero state, either recurrence returns zero forever, so
//! [`XorShift64::new`] and [`XorShift64Star::new`] return [`SeedError::ZeroSeed`] for it.
//!
//! [`below`](XorShift64::below) reduces one draw modulo the bound, and a bound of 0 returns 0 after one
//! draw. A draw is never 0, so it is one of the 2^64 - 1 values from 1 to 2^64 - 1. When the bound
//! divides 2^64 - 1, every result is equally likely: `below(3)` is unbiased over all nonzero states.
//! For any other bound, the results from 1 to (2^64 - 1) mod the bound each come from one more state
//! than the other results, and the bias can be large: on a 64-bit target, `below(2^63 + 1)` returns 1
//! twice as often as 0.
//! [`byte`](XorShift64::byte) returns the low 8 bits of one draw, and [`fill`](XorShift64::fill) sets
//! each byte of a slice to one [`byte`](XorShift64::byte) in index order.

use std::fmt;

/// The seed passed to a generator's constructor cannot start a sequence.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SeedError {
    /// The seed is 0, which the xorshift recurrences map to 0 on every draw.
    ZeroSeed,
}

impl fmt::Display for SeedError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SeedError::ZeroSeed => f.write_str("an xorshift generator needs a nonzero seed"),
        }
    }
}

impl std::error::Error for SeedError {}

/// Marsaglia's xorshift64 generator (`x ^= x << 13; x ^= x >> 7; x ^= x << 17`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct XorShift64(u64);

impl XorShift64 {
    /// Start a sequence from `seed`. Returns [`SeedError::ZeroSeed`] when `seed` is 0.
    pub fn new(seed: u64) -> Result<Self, SeedError> {
        if seed == 0 { Err(SeedError::ZeroSeed) } else { Ok(Self(seed)) }
    }

    /// Advance the state and return it.
    #[allow(clippy::should_implement_trait)]
    pub fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }

    /// One draw reduced modulo `n`, so the result is below `n`. A bound of 0 returns 0 and still takes a draw.
    pub fn below(&mut self, n: usize) -> usize {
        (self.next() % n.max(1) as u64) as usize
    }

    /// The low 8 bits of one draw.
    pub fn byte(&mut self) -> u8 {
        self.next() as u8
    }

    /// Set each byte of `out` to one [`byte`](Self::byte), in index order. An empty slice takes no draw.
    pub fn fill(&mut self, out: &mut [u8]) {
        for b in out {
            *b = self.byte();
        }
    }
}

/// Vigna's xorshift64* generator: the state shifts by 12, 25 and 27, and each output is the new state
/// multiplied by `0x2545_F491_4F6C_DD1D`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct XorShift64Star(u64);

impl XorShift64Star {
    /// Start a sequence from `seed`. Returns [`SeedError::ZeroSeed`] when `seed` is 0.
    pub fn new(seed: u64) -> Result<Self, SeedError> {
        if seed == 0 { Err(SeedError::ZeroSeed) } else { Ok(Self(seed)) }
    }

    /// Advance the state and return it multiplied by `0x2545_F491_4F6C_DD1D`.
    #[allow(clippy::should_implement_trait)]
    pub fn next(&mut self) -> u64 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }

    /// One draw reduced modulo `n`, so the result is below `n`. A bound of 0 returns 0 and still takes a draw.
    pub fn below(&mut self, n: usize) -> usize {
        (self.next() % n.max(1) as u64) as usize
    }

    /// The low 8 bits of one draw.
    pub fn byte(&mut self) -> u8 {
        self.next() as u8
    }

    /// Set each byte of `out` to one [`byte`](Self::byte), in index order. An empty slice takes no draw.
    pub fn fill(&mut self, out: &mut [u8]) {
        for b in out {
            *b = self.byte();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SEED: u64 = 0x9e37_79b9_7f4a_7c15;
    const JPEG_SEED_2: u64 = 0xD1B5_4A32_D192_ED03;

    /// The generator of craft-libs `crates/video/tests/dnx/fuzz_hostile.rs` and FilmCraft
    /// `crates/dnx/tests/it/fuzz_hostile.rs`, as those suites define it.
    struct DnxRng(u64);
    impl DnxRng {
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

    /// The generator of craft-libs `crates/jpeg/tests/mutation.rs`, as that suite defines it.
    struct JpegRng(u64);
    impl JpegRng {
        fn next(&mut self) -> u64 {
            self.0 ^= self.0 >> 12;
            self.0 ^= self.0 << 25;
            self.0 ^= self.0 >> 27;
            self.0.wrapping_mul(0x2545_F491_4F6C_DD1D)
        }
        fn below(&mut self, n: usize) -> usize {
            (self.next() % n.max(1) as u64) as usize
        }
        fn byte(&mut self) -> u8 {
            self.next() as u8
        }
    }

    fn xs(seed: u64) -> XorShift64 {
        match XorShift64::new(seed) {
            Ok(r) => r,
            Err(e) => panic!("seed {seed:#x}: {e}"),
        }
    }

    fn xss(seed: u64) -> XorShift64Star {
        match XorShift64Star::new(seed) {
            Ok(r) => r,
            Err(e) => panic!("seed {seed:#x}: {e}"),
        }
    }

    #[test]
    fn xorshift64_reference_sequence() {
        let mut r = xs(SEED);
        let got: Vec<u64> = (0..4).map(|_| r.next()).collect();
        assert_eq!(got, [0xdc1b_77ae_0bf3_4dad, 0x64f0_eeb9_026e_6076, 0x7b07_ce91_e590_6136, 0x305f_050c_368d_cc74]);
    }

    #[test]
    fn xorshift64star_reference_sequence() {
        let mut r = xss(SEED);
        let got: Vec<u64> = (0..4).map(|_| r.next()).collect();
        assert_eq!(got, [0x0d83_b3e2_9a21_487a, 0x54c4_4c79_f1fe_9d67, 0xa845_f342_007a_0e78, 0x7d6e_0b87_8a79_4779]);
    }

    #[test]
    fn below_range_and_zero() {
        let bounds = [1usize, 2, 3, 7, 10, 255, 256, 1000, usize::MAX];
        let mut a = xs(SEED);
        let mut b = xss(SEED);
        for _ in 0..64 {
            for n in bounds {
                assert!(a.below(n) < n, "XorShift64::below({n})");
                assert!(b.below(n) < n, "XorShift64Star::below({n})");
            }
        }

        let (mut a, mut a_peer) = (xs(SEED), xs(SEED));
        assert_eq!(a.below(0), 0);
        a_peer.next();
        assert_eq!(a, a_peer, "below(0) takes one draw");

        let (mut b, mut b_peer) = (xss(SEED), xss(SEED));
        assert_eq!(b.below(0), 0);
        b_peer.next();
        assert_eq!(b, b_peer, "below(0) takes one draw");
    }

    #[test]
    fn byte_is_next_low_byte() {
        let (mut a, mut a_peer) = (xs(SEED), xs(SEED));
        let (mut b, mut b_peer) = (xss(SEED), xss(SEED));
        for _ in 0..32 {
            assert_eq!(a.byte(), a_peer.next() as u8);
            assert_eq!(b.byte(), b_peer.next() as u8);
        }
        assert_eq!(a, a_peer);
        assert_eq!(b, b_peer);
    }

    #[test]
    fn fill_matches_successive_bytes() {
        for len in [0usize, 1, 2, 9, 64] {
            let (mut a, mut a_peer) = (xs(SEED), xs(SEED));
            let mut out = vec![0xAAu8; len];
            a.fill(&mut out);
            assert_eq!(out.len(), len);
            let expect: Vec<u8> = (0..len).map(|_| a_peer.byte()).collect();
            assert_eq!(out, expect, "XorShift64::fill of {len} bytes");
            assert_eq!(a.next(), a_peer.next(), "XorShift64 state after fill of {len} bytes");

            let (mut b, mut b_peer) = (xss(SEED), xss(SEED));
            let mut out = vec![0xAAu8; len];
            b.fill(&mut out);
            assert_eq!(out.len(), len);
            let expect: Vec<u8> = (0..len).map(|_| b_peer.byte()).collect();
            assert_eq!(out, expect, "XorShift64Star::fill of {len} bytes");
            assert_eq!(b.next(), b_peer.next(), "XorShift64Star state after fill of {len} bytes");
        }
    }

    #[test]
    fn zero_seed_is_rejected() {
        assert_eq!(XorShift64::new(0), Err(SeedError::ZeroSeed));
        assert_eq!(XorShift64Star::new(0), Err(SeedError::ZeroSeed));
        assert!(XorShift64::new(1).is_ok());
        assert!(XorShift64Star::new(1).is_ok());
        assert!(XorShift64::new(u64::MAX).is_ok());
        assert!(XorShift64Star::new(u64::MAX).is_ok());
    }

    #[test]
    fn dnx_and_jpeg_sequences_match() {
        // The DNx suites seed with `SEED ^ index` for each seed frame.
        for si in 0..5u64 {
            let mut ours = xs(SEED ^ si);
            let mut theirs = DnxRng(SEED ^ si);
            for n in [0usize, 1, 17, 4096, 65_535] {
                assert_eq!(ours.below(n), theirs.below(n));
                assert_eq!(ours.next(), theirs.next());
                assert_eq!(ours.byte(), theirs.next() as u8);
            }
        }

        for seed in [SEED, JPEG_SEED_2] {
            let mut ours = xss(seed);
            let mut theirs = JpegRng(seed);
            for n in [0usize, 1, 17, 4096, 65_535] {
                assert_eq!(ours.below(n), theirs.below(n));
                assert_eq!(ours.byte(), theirs.byte());
                assert_eq!(ours.next(), theirs.next());
            }
        }

        let mut r = xss(JPEG_SEED_2);
        let got: Vec<u64> = (0..4).map(|_| r.next()).collect();
        assert_eq!(got, [0x015f_6958_028f_9348, 0xca16_c6bb_fa7f_a781, 0x5a65_feae_5ef1_f5e3, 0x54c4_89ba_613d_6dff]);
    }
}
