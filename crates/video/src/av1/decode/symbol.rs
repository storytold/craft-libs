//! Symbol (arithmetic) decoder, spec 8.2.
//!
//! The state follows the specification literally: `SymbolValue` / `SymbolRange` are 15/16-bit
//! quantities renormalised by reading `bits` new bits at a time, and `SymbolMaxBits` counts the
//! bits still available (going negative while zero padding is consumed).

const EC_PROB_SHIFT: u32 = 6;
const EC_MIN_PROB: u32 = 4;

pub(crate) struct SymbolDecoder<'a> {
    data: &'a [u8],
    /// Next byte of `data` to load into the window.
    next: usize,
    /// SymbolValue followed by `k` look-ahead bits: the bitstream with every bit inverted
    /// (the spec's renormalisation XORs the new bits with ones), zero bits past the end of
    /// the data reading as ones. SymbolValue = win >> k.
    win: u64,
    k: u32,
    range: u32,
    pub disable_update: bool,
}

impl<'a> SymbolDecoder<'a> {
    /// `init_symbol( sz )` over `data` (the tile's bytes).
    pub fn new(data: &'a [u8], disable_update: bool) -> Self {
        let mut d = SymbolDecoder { data, next: 0, win: 0, k: 0, range: 1 << 15, disable_update };
        // SymbolValue = ((1 << 15) - 1) ^ (first 15 bits): 15 inverted bits.
        let mut filled = 0;
        while filled < 15 + 32 {
            d.win = (d.win << 8) | d.inverted_byte() as u64;
            filled += 8;
        }
        d.k = filled - 15;
        d
    }

    #[inline(always)]
    fn inverted_byte(&mut self) -> u8 {
        let b = self.data.get(self.next).copied().unwrap_or(0);
        self.next += 1;
        !b
    }

    /// `read_symbol( cdf )`: `cdf` has N + 1 entries (the last is the adaptation counter).
    #[inline]
    pub fn read_symbol(&mut self, cdf: &mut [u16]) -> usize {
        let n = cdf.len() - 1;
        let symbol = self.decode(cdf, n);
        if !self.disable_update {
            update_cdf(cdf, n, symbol);
        }
        symbol
    }

    #[inline(always)]
    fn decode(&mut self, cdf: &[u16], n: usize) -> usize {
        let value = (self.win >> self.k) as u32;
        let mut cur = self.range;
        let mut symbol = 0usize;
        let mut prev;
        loop {
            prev = cur;
            let f = (1u32 << 15) - cdf[symbol] as u32;
            cur = ((self.range >> 8) * (f >> EC_PROB_SHIFT)) >> (7 - EC_PROB_SHIFT);
            cur += EC_MIN_PROB * (n - symbol - 1) as u32;
            if value >= cur {
                break;
            }
            symbol += 1;
        }
        self.range = prev - cur;
        self.win -= (cur as u64) << self.k;
        self.renormalize();
        symbol
    }

    /// Renormalisation (8.2.6): SymbolValue takes `bits` more (inverted) bits, which are
    /// already in the window.
    #[inline(always)]
    fn renormalize(&mut self) {
        let bits = 15 - (31 - self.range.leading_zeros());
        self.range <<= bits;
        self.k -= bits;
        if self.k < 16 {
            // SymbolValue < 2^16, so 16 + k <= 64 bits stay in the window.
            while self.k <= 40 {
                self.win = (self.win << 8) | self.inverted_byte() as u64;
                self.k += 8;
            }
        }
    }

    /// `read_bool()`
    #[inline]
    pub fn read_bool(&mut self) -> u32 {
        let cdf = [1u16 << 14, 1 << 15, 0];
        self.decode(&cdf, 2) as u32
    }

    /// `read_literal( n )` (L(n))
    pub fn read_literal(&mut self, n: u32) -> u32 {
        let mut x = 0;
        for _ in 0..n {
            x = (x << 1) | self.read_bool();
        }
        x
    }

    /// NS(n)
    pub fn read_ns(&mut self, n: u32) -> u32 {
        if n <= 1 {
            return 0;
        }
        let w = 32 - n.leading_zeros();
        let m = (1u32 << w) - n;
        let v = self.read_literal(w - 1);
        if v < m {
            return v;
        }
        let extra = self.read_literal(1);
        (v << 1) - m + extra
    }
}

/// CDF adaptation (8.2.6).
#[inline(always)]
pub(crate) fn update_cdf(cdf: &mut [u16], n: usize, symbol: usize) {
    let count = cdf[n];
    let rate = 3 + (count > 15) as u32 + (count > 31) as u32 + (31 - (n as u32).leading_zeros()).min(2);
    // Entries below `symbol` move towards 0, the others towards 1 << 15 (the specification's
    // comparison with tmp, resolved per entry: c >= 0 and c <= 1 << 15), without branching on
    // the decoded symbol.
    for (i, e) in cdf[..n - 1].iter_mut().enumerate() {
        let c = *e as u32;
        *e = if i < symbol { c - (c >> rate) } else { c + (((1 << 15) - c) >> rate) } as u16;
    }
    cdf[n] += (count < 32) as u16;
}
