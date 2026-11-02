//! CABAC: context initialisation (9.3.2.2), the arithmetic encoder (9.3.4.x, informative encoder
//! description: EncodeDecision, EncodeBypass, EncodeTerminate, EncodeFlush) and a fractional-bit
//! estimator with the same context-state evolution, used for rate-distortion decisions.

use crate::bitstream::BitWriter;
use crate::tables::{CABAC_INIT, NEXT_STATE, NUM_CTX, RANGE_TAB_LPS};
use std::sync::OnceLock;

/// Context states, each `pStateIdx << 1 | valMps`.
pub type Contexts = [u8; NUM_CTX];

/// Initialise all context variables for `slice_qp` and `init_type` (9.3.2.2).
pub fn init_contexts(slice_qp: i32, init_type: usize) -> Contexts {
    let mut ctx = [0u8; NUM_CTX];
    let qp = slice_qp.clamp(0, 51);
    for (c, &v) in ctx.iter_mut().zip(CABAC_INIT[init_type].iter()) {
        let slope = (v >> 4) as i32;
        let offset = (v & 15) as i32;
        let m = slope * 5 - 45;
        let n = (offset << 3) - 16;
        let pre = (((m * qp) >> 4) + n).clamp(1, 126);
        *c = if pre <= 63 { ((63 - pre) << 1) as u8 } else { (((pre - 64) << 1) | 1) as u8 };
    }
    ctx
}

/// Destination of binarised syntax elements: the real arithmetic coder or the bit estimator.
pub trait Sink {
    fn decision(&mut self, ctx: usize, bin: u32);
    fn bypass(&mut self, bin: u32);
    /// `n` bypass bins carrying the low `n` bits of `v`, MSB first.
    fn bypass_bits(&mut self, v: u32, n: u32) {
        for i in (0..n).rev() {
            self.bypass((v >> i) & 1);
        }
    }
    fn terminate(&mut self, bin: u32);
}

/// Arithmetic encoder writing into a bit writer.
pub struct CabacEncoder {
    pub ctx: Contexts,
    low: u32,
    range: u32,
    outstanding: u32,
    first: bool,
    pub w: BitWriter,
}

impl CabacEncoder {
    /// Start encoding slice data into `w` (which must be byte aligned).
    pub fn new(w: BitWriter, ctx: Contexts) -> Self {
        debug_assert!(w.is_aligned());
        CabacEncoder { ctx, low: 0, range: 510, outstanding: 0, first: true, w }
    }

    #[inline]
    fn put_bit(&mut self, b: u32) {
        if self.first {
            self.first = false;
        } else {
            self.w.bit(b);
        }
        while self.outstanding > 0 {
            self.w.bit(1 - b);
            self.outstanding -= 1;
        }
    }

    #[inline]
    fn renorm(&mut self) {
        while self.range < 256 {
            if self.low < 256 {
                self.put_bit(0);
            } else if self.low >= 512 {
                self.low -= 512;
                self.put_bit(1);
            } else {
                self.low -= 256;
                self.outstanding += 1;
            }
            self.range <<= 1;
            self.low <<= 1;
        }
    }

    /// EncodeFlush after a terminating bin equal to 1; the final bit written is the
    /// rbsp_stop_one_bit. Returns the writer (byte aligned with zero bits).
    pub fn finish(mut self) -> BitWriter {
        self.range = 2;
        self.renorm();
        let b = (self.low >> 9) & 1;
        self.put_bit(b);
        self.w.put(((self.low >> 7) & 3) | 1, 2);
        self.w.align_zero();
        self.w
    }
}

impl Sink for CabacEncoder {
    #[inline]
    fn decision(&mut self, ctx_idx: usize, bin: u32) {
        let s = self.ctx[ctx_idx] as usize;
        let mps = (s & 1) as u32;
        let q = ((self.range >> 6) & 3) as usize;
        let lps = RANGE_TAB_LPS[s >> 1][q] as u32;
        self.range -= lps;
        if bin != mps {
            self.low += self.range;
            self.range = lps;
            self.ctx[ctx_idx] = NEXT_STATE[s][1];
        } else {
            self.ctx[ctx_idx] = NEXT_STATE[s][0];
        }
        self.renorm();
    }

    #[inline]
    fn bypass(&mut self, bin: u32) {
        self.low <<= 1;
        if bin != 0 {
            self.low += self.range;
        }
        if self.low >= 1024 {
            self.put_bit(1);
            self.low -= 1024;
        } else if self.low < 512 {
            self.put_bit(0);
        } else {
            self.low -= 512;
            self.outstanding += 1;
        }
    }

    fn terminate(&mut self, bin: u32) {
        self.range -= 2;
        if bin != 0 {
            self.low += self.range;
            // the caller flushes with finish()
        } else {
            self.renorm();
        }
    }
}

/// Fractional bit cost (1/32768 bit units) of coding the MPS / LPS in each state.
fn cost_table() -> &'static [[u32; 2]; 64] {
    static T: OnceLock<[[u32; 2]; 64]> = OnceLock::new();
    T.get_or_init(|| {
        let mut t = [[0u32; 2]; 64];
        // pLPS(s) = 0.5 * alpha^s with alpha = (0.01875 / 0.5)^(1/63) (9.3.4.3 design)
        let alpha = (0.01875f64 / 0.5).powf(1.0 / 63.0);
        for (s, e) in t.iter_mut().enumerate() {
            let p = 0.5 * alpha.powi(s as i32);
            e[0] = (-(1.0 - p).log2() * 32768.0).round() as u32;
            e[1] = (-p.log2() * 32768.0).round() as u32;
        }
        t
    })
}

pub const BIT: u64 = 32768;

/// Bit estimator: tracks context states like the encoder and accumulates the ideal code length.
#[derive(Clone)]
pub struct Estimator {
    pub ctx: Contexts,
    /// Accumulated cost in 1/32768 bits.
    pub bits: u64,
    table: &'static [[u32; 2]; 64],
}

impl Estimator {
    pub fn new(ctx: Contexts) -> Self {
        Estimator { ctx, bits: 0, table: cost_table() }
    }
}

impl Sink for Estimator {
    #[inline]
    fn decision(&mut self, ctx_idx: usize, bin: u32) {
        let s = self.ctx[ctx_idx] as usize;
        let lps = (bin != (s & 1) as u32) as usize;
        self.bits += self.table[s >> 1][lps] as u64;
        self.ctx[ctx_idx] = NEXT_STATE[s][lps];
    }
    #[inline]
    fn bypass(&mut self, _bin: u32) {
        self.bits += BIT;
    }
    #[inline]
    fn bypass_bits(&mut self, _v: u32, n: u32) {
        self.bits += BIT * n as u64;
    }
    fn terminate(&mut self, _bin: u32) {
        self.bits += 7; // ~0 bits for a 0 bin
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn context_init_formula() {
        let ctx = init_contexts(30, 0);
        // initValue 154: m = 0, n = 64 -> pStateIdx 0, valMps 1
        assert_eq!(ctx[crate::tables::CU_TRANSQUANT_BYPASS], 1);
        // split_cu_flag ctx 0 (initValue 139) at QP 30: preCtxState 62
        assert_eq!(ctx[crate::tables::SPLIT_CU], (63 - 62) << 1);
    }

    #[test]
    fn costs_are_sane() {
        let t = cost_table();
        assert_eq!(t[0][0], 32768);
        assert_eq!(t[0][1], 32768);
        assert!(t[62][0] < 1000 && t[62][1] > 5 * 32768);
    }
}
