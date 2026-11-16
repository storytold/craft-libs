//! Multi-symbol arithmetic encoder: the exact inverse of the symbol decoder of spec 8.2.
//!
//! The decoder keeps `SymbolValue`, the bitwise complement of a 15-bit window into the tile data
//! minus the lower bounds of the symbols decoded so far, and selects symbol `k` when
//! `cur(k) <= SymbolValue < cur(k - 1)` with
//! `cur(k) = ((SymbolRange >> 8) * ((32768 - cdf[k]) >> 6) >> 1) + 4 * (N - k - 1)` and
//! `cur(-1) = SymbolRange`. The encoder therefore runs an ordinary carry-propagating range
//! coder over the complemented bit stream: coding symbol `k` adds `cur(k)` to `low` and sets the
//! range to `cur(k - 1) - cur(k)`; both are renormalised exactly like the decoder's range.
//!
//! At the end (spec 8.2.4 exit process) the decoder has consumed `S` renormalisation bits plus a
//! 15-bit window, and the bits from position `S` on must be a one followed by zeros. In the
//! complemented domain that window is `0b011_1111_1111_1111`, so the encoder picks the `S`-bit
//! prefix `D` with `D * 2^15 + 2^14 - 1` inside `[low, low + range)`, i.e.
//! `D = (low + 2^14) >> 15`, and writes `!D`, a one bit and zero padding.

pub(crate) struct SymbolWriter {
    /// Low end of the interval, holding `15 + cnt` bits not yet moved to `out` (plus carries).
    low: u64,
    rng: u32,
    cnt: u32,
    out: Vec<u8>,
    pub disable_update: bool,
}

impl SymbolWriter {
    pub fn new(disable_update: bool) -> Self {
        SymbolWriter { low: 0, rng: 1 << 15, cnt: 0, out: Vec::with_capacity(1 << 16), disable_update }
    }

    /// Adds `k` to the bytes already output (carry propagation).
    fn carry(&mut self, mut k: u64) {
        let mut i = self.out.len();
        while k != 0 {
            i -= 1;
            let t = self.out[i] as u64 + k;
            self.out[i] = t as u8;
            k = t >> 8;
        }
    }

    #[inline(always)]
    fn encode(&mut self, cdf: &[u16], n: usize, s: usize) {
        let r8 = self.rng >> 8;
        let cur = |k: usize| -> u32 { ((r8 * ((32768 - cdf[k] as u32) >> 6)) >> 1) + 4 * (n - k - 1) as u32 };
        let hi = if s == 0 { self.rng } else { cur(s - 1) };
        let lo = cur(s);
        debug_assert!(hi > lo, "symbol {s} has an empty interval");
        self.low += lo as u64;
        self.rng = hi - lo;
        let d = 15 - (31 - self.rng.leading_zeros());
        if d == 0 {
            return;
        }
        self.rng <<= d;
        self.low <<= d;
        self.cnt += d;
        while self.cnt >= 8 {
            let sh = 15 + self.cnt - 8;
            let c = self.low >> sh;
            self.low &= (1u64 << sh) - 1;
            self.cnt -= 8;
            self.carry(c >> 8);
            self.out.push((c & 0xff) as u8);
        }
    }

    /// Codes `s` with the adaptive CDF `cdf` (N values plus the adaptation counter).
    #[inline]
    pub fn symbol(&mut self, s: usize, cdf: &mut [u16]) {
        let n = cdf.len() - 1;
        debug_assert!(s < n);
        self.encode(cdf, n, s);
        if !self.disable_update {
            update_cdf(cdf, n, s);
        }
    }

    /// Codes `s` with a CDF that is not adapted.
    pub fn symbol_fixed(&mut self, s: usize, cdf: &[u16]) {
        let n = cdf.len() - 1;
        self.encode(cdf, n, s);
    }

    /// `read_bool()` counterpart.
    pub fn bool(&mut self, b: bool) {
        self.encode(&[1 << 14, 1 << 15, 0], 2, b as usize);
    }

    /// `L(n)` counterpart (most significant bit first).
    #[cfg(test)]
    pub fn literal(&mut self, n: u32, v: u32) {
        for i in (0..n).rev() {
            self.bool((v >> i) & 1 != 0);
        }
    }

    /// Finishes the tile: returns its bytes (with the trailing one bit and zero padding).
    pub fn finish(mut self) -> Vec<u8> {
        let low = self.low + (1 << 14);
        let mut tail = low >> 15;
        self.carry(tail >> self.cnt);
        tail &= (1u64 << self.cnt) - 1;
        let mut bytes: Vec<u8> = self.out.iter().map(|b| !b).collect();
        let n = self.cnt + 1; // 1..=8 bits: !tail then the trailing one bit
        let v = (((!tail) & ((1u64 << self.cnt) - 1)) << 1) | 1;
        bytes.push((v << (8 - n)) as u8);
        bytes
    }
}

/// CDF adaptation (spec 8.2.6, as in the decoder).
#[inline(always)]
pub(crate) fn update_cdf(cdf: &mut [u16], n: usize, symbol: usize) {
    let count = cdf[n];
    let rate = 3 + (count > 15) as u32 + (count > 31) as u32 + (31 - (n as u32).leading_zeros()).min(2);
    let mut tmp = 0u32;
    for i in 0..n - 1 {
        if i == symbol {
            tmp = 1 << 15;
        }
        let c = cdf[i] as u32;
        if tmp < c {
            cdf[i] = (c - ((c - tmp) >> rate)) as u16;
        } else {
            cdf[i] = (c + ((tmp - c) >> rate)) as u16;
        }
    }
    cdf[n] += (count < 32) as u16;
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The spec's symbol decoder (8.2.2 - 8.2.4), for checking the encoder.
    struct Dec<'a> {
        data: &'a [u8],
        pos: usize,
        value: u32,
        range: u32,
        max_bits: i64,
    }

    impl<'a> Dec<'a> {
        fn new(data: &'a [u8]) -> Self {
            let mut d = Dec { data, pos: 0, value: 0, range: 1 << 15, max_bits: 8 * data.len() as i64 - 15 };
            let nb = (data.len() * 8).min(15) as u32;
            let buf = d.bits(nb);
            d.value = ((1 << 15) - 1) ^ (buf << (15 - nb));
            d
        }
        fn bits(&mut self, n: u32) -> u32 {
            let mut x = 0;
            for _ in 0..n {
                let b = if self.pos < self.data.len() * 8 { (self.data[self.pos >> 3] >> (7 - (self.pos & 7))) & 1 } else { 0 };
                self.pos += 1;
                x = (x << 1) | b as u32;
            }
            x
        }
        fn symbol(&mut self, cdf: &mut [u16], adapt: bool) -> usize {
            let n = cdf.len() - 1;
            let mut cur = self.range;
            let mut s = 0;
            let mut prev;
            loop {
                prev = cur;
                let f = (1u32 << 15) - cdf[s] as u32;
                cur = (((self.range >> 8) * (f >> 6)) >> 1) + 4 * (n - s - 1) as u32;
                if self.value >= cur {
                    break;
                }
                s += 1;
            }
            self.range = prev - cur;
            self.value -= cur;
            let b = 15 - (31 - self.range.leading_zeros());
            if b > 0 {
                self.range <<= b;
                let nb = (b as i64).min(self.max_bits.max(0)) as u32;
                let nd = self.bits(nb) << (b - nb);
                self.value = nd ^ (((self.value + 1) << b) - 1);
                self.max_bits -= b as i64;
            }
            if adapt {
                update_cdf(cdf, n, s);
            }
            s
        }
        /// Position of the trailing one bit (8.2.4).
        fn trailing_position(&self) -> i64 {
            let consumed = (self.pos as i64).min(self.data.len() as i64 * 8);
            consumed - (15i64).min(self.max_bits + 15)
        }
    }

    #[test]
    fn bools_only() {
        for len in 1..200 {
            let bits: Vec<bool> = (0..len).map(|i| (i * 7 + len) % 3 == 0).collect();
            let mut w = SymbolWriter::new(false);
            for &b in &bits {
                w.bool(b);
            }
            let data = w.finish();
            let mut d = Dec::new(&data);
            for (i, &b) in bits.iter().enumerate() {
                assert_eq!(d.symbol(&mut [1 << 14, 1 << 15, 0], false) == 1, b, "len {len} bit {i} data {data:?}");
            }
        }
    }

    #[test]
    fn round_trip_random_symbols() {
        let mut seed = 0x1234_5678u32;
        let mut rnd = || {
            seed ^= seed << 13;
            seed ^= seed >> 17;
            seed ^= seed << 5;
            seed
        };
        for trial in 0..300 {
            let count = 1 + (rnd() % 3000) as usize;
            let mut cdfs_e = vec![[4096u16, 11264, 19328, 32768, 0]; 4];
            let mut cdfs_d = cdfs_e.clone();
            let mut syms = Vec::new();
            let mut w = SymbolWriter::new(false);
            for _ in 0..count {
                let kind = rnd() % 3;
                let ctx = (rnd() % 4) as usize;
                // skewed symbols so that the range also gets small
                let s = if rnd() % 4 == 0 { (rnd() % 4) as usize } else { 0 };
                match kind {
                    0 => w.symbol(s, &mut cdfs_e[ctx]),
                    1 => w.bool(s & 1 == 1),
                    _ => w.literal(5, s as u32 * 7),
                }
                syms.push((kind, ctx, s));
            }
            let data = w.finish();
            let mut d = Dec::new(&data);
            for (idx, &(kind, ctx, s)) in syms.iter().enumerate() {
                let got = match kind {
                    0 => d.symbol(&mut cdfs_d[ctx], true),
                    1 => d.symbol(&mut [1 << 14, 1 << 15, 0], false),
                    _ => {
                        let mut v = 0;
                        for _ in 0..5 {
                            v = (v << 1) | d.symbol(&mut [1 << 14, 1 << 15, 0], false);
                        }
                        v / 7
                    }
                };
                let want = if kind == 1 { s & 1 } else { s };
                assert_eq!(got, want, "trial {trial} idx {idx} kind {kind} of {}", syms.len());
            }
            let tp = d.trailing_position();
            assert!(tp >= 0 && (tp as usize) < data.len() * 8, "trial {trial}: trailing bit position {tp}");
            let tp = tp as usize;
            assert_eq!((data[tp >> 3] >> (7 - (tp & 7))) & 1, 1, "trial {trial}: trailing one bit");
            for p in tp + 1..data.len() * 8 {
                assert_eq!((data[p >> 3] >> (7 - (p & 7))) & 1, 0, "trial {trial}: padding");
            }
        }
    }
}
