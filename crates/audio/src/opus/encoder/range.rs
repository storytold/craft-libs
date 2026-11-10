//! Range encoder (RFC 6716 §5.1), the exact inverse of the §4.1 range decoder.
//!
//! Range-coded symbols are written from the front of the buffer and raw bits from the back; the
//! bit accounting ([`RangeEncoder::tell`], [`RangeEncoder::tell_frac`]) matches the decoder's, so
//! every budget decision the decoder takes can be mirrored exactly.

const SYM_BITS: u32 = 8;
const CODE_BITS: u32 = 32;
const SYM_MAX: u32 = (1 << SYM_BITS) - 1;
const CODE_SHIFT: u32 = CODE_BITS - SYM_BITS - 1;
const CODE_TOP: u32 = 1 << (CODE_BITS - 1);
const CODE_BOT: u32 = CODE_TOP >> SYM_BITS;
/// Bit resolution of [`RangeEncoder::tell_frac`] (1/8 bit).
pub const BITRES: u32 = 3;

/// Number of bits needed to represent `x` (0 for 0).
#[inline]
pub fn ilog(x: u32) -> i32 {
    (32 - x.leading_zeros()) as i32
}

#[derive(Clone, Debug)]
pub struct RangeEncoder {
    out: Vec<u8>,
    end: Vec<u8>,
    end_window: u32,
    nend_bits: i32,
    nbits_total: i32,
    rng: u32,
    low: u32,
    rem: i32,
    ext: u32,
}

impl Default for RangeEncoder {
    fn default() -> Self {
        Self::new()
    }
}

impl RangeEncoder {
    pub fn new() -> Self {
        RangeEncoder {
            out: Vec::new(),
            end: Vec::new(),
            end_window: 0,
            nend_bits: 0,
            nbits_total: (CODE_BITS + 1) as i32,
            rng: CODE_TOP,
            low: 0,
            rem: -1,
            ext: 0,
        }
    }

    /// Current range (equals the decoder's range after the same symbols).
    #[cfg(test)]
    pub fn rng(&self) -> u32 {
        self.rng
    }

    fn carry_out(&mut self, c: u32) {
        if c != SYM_MAX {
            let carry = c >> SYM_BITS;
            if self.rem >= 0 {
                self.out.push((self.rem as u32 + carry) as u8);
            }
            if self.ext > 0 {
                let sym = (SYM_MAX + carry) & SYM_MAX;
                for _ in 0..self.ext {
                    self.out.push(sym as u8);
                }
                self.ext = 0;
            }
            self.rem = (c & SYM_MAX) as i32;
        } else {
            self.ext += 1;
        }
    }

    #[inline]
    fn normalize(&mut self) {
        while self.rng <= CODE_BOT {
            self.carry_out(self.low >> CODE_SHIFT);
            self.low = (self.low << SYM_BITS) & (CODE_TOP - 1);
            self.rng <<= SYM_BITS;
            self.nbits_total += SYM_BITS as i32;
        }
    }

    /// Encodes the symbol with cumulative range `[fl, fh)` of total `ft`.
    pub fn encode(&mut self, fl: u32, fh: u32, ft: u32) {
        let r = self.rng / ft;
        if fl > 0 {
            self.low += self.rng - r * (ft - fl);
            self.rng = r * (fh - fl);
        } else {
            self.rng -= r * (ft - fh);
        }
        self.normalize();
    }

    /// Like [`Self::encode`] with `ft = 1 << bits`.
    pub fn encode_bin(&mut self, fl: u32, fh: u32, bits: u32) {
        let r = self.rng >> bits;
        let ft = 1u32 << bits;
        if fl > 0 {
            self.low += self.rng - r * (ft - fl);
            self.rng = r * (fh - fl);
        } else {
            self.rng -= r * (ft - fh);
        }
        self.normalize();
    }

    /// Encodes a binary symbol whose probability of being 1 is `1/2^logp`.
    pub fn bit_logp(&mut self, val: bool, logp: u32) {
        let r = self.rng;
        let s = r >> logp;
        let r2 = r - s;
        if val {
            self.low += r2;
            self.rng = s;
        } else {
            self.rng = r2;
        }
        self.normalize();
    }

    /// Encodes symbol `s` with an "inverse CDF" table (`ft = 1 << ftb`).
    pub fn icdf(&mut self, s: usize, icdf: &[u8], ftb: u32) {
        let r = self.rng >> ftb;
        if s > 0 {
            self.low += self.rng - r * icdf[s - 1] as u32;
            self.rng = r * (icdf[s - 1] as u32 - icdf[s] as u32);
        } else {
            self.rng -= r * icdf[s] as u32;
        }
        self.normalize();
    }

    /// Encodes a uniformly distributed integer `fl` in `[0, ft)` (`ft >= 2`).
    pub fn uint(&mut self, fl: u32, ft: u32) {
        debug_assert!(ft > 1 && fl < ft);
        let ft1 = ft - 1;
        let ftb = ilog(ft1);
        if ftb > 8 {
            let ftb = (ftb - 8) as u32;
            let top = (ft1 >> ftb) + 1;
            let v = fl >> ftb;
            self.encode(v, v + 1, top);
            self.bits(fl & ((1 << ftb) - 1), ftb);
        } else {
            self.encode(fl, fl + 1, ft);
        }
    }

    /// Writes `bits` raw bits (0..=25) to the end of the buffer.
    pub fn bits(&mut self, val: u32, bits: u32) {
        if bits == 0 {
            return;
        }
        let mut window = self.end_window;
        let mut used = self.nend_bits;
        if used as u32 + bits > CODE_BITS {
            while used >= SYM_BITS as i32 {
                self.end.push((window & SYM_MAX) as u8);
                window >>= SYM_BITS;
                used -= SYM_BITS as i32;
            }
        }
        window |= val << used;
        used += bits as i32;
        self.end_window = window;
        self.nend_bits = used;
        self.nbits_total += bits as i32;
    }

    /// Number of bits written so far (rounded up), identical to the decoder's `tell`.
    #[inline]
    pub fn tell(&self) -> i32 {
        self.nbits_total - ilog(self.rng)
    }

    /// Number of 1/8 bits written so far (rounded up), identical to the decoder's `tell_frac`.
    pub fn tell_frac(&self) -> i32 {
        let nbits = (self.nbits_total as u32) << BITRES;
        let mut l = ilog(self.rng) as u32;
        let mut r = self.rng >> (l - 16);
        for _ in 0..BITRES {
            r = (r * r) >> 15;
            let b = r >> 16;
            l = (l << 1) | b;
            r >>= b;
        }
        nbits.wrapping_sub(l) as i32
    }

    /// Finalises the stream into a buffer of exactly `size` bytes (range data at the front, raw
    /// bits at the back, zero padding in between; a partial raw-bit byte is merged into the byte
    /// just before the full raw-bit bytes).
    pub fn done(mut self, size: usize) -> Vec<u8> {
        let mut l = CODE_BITS as i32 - ilog(self.rng);
        let mut msk = (CODE_TOP - 1) >> l;
        let mut end = (self.low.wrapping_add(msk)) & !msk;
        if (end | msk) >= self.low.wrapping_add(self.rng) {
            l += 1;
            msk >>= 1;
            end = (self.low.wrapping_add(msk)) & !msk;
        }
        while l > 0 {
            self.carry_out(end >> CODE_SHIFT);
            end = (end << SYM_BITS) & (CODE_TOP - 1);
            l -= SYM_BITS as i32;
        }
        if self.rem >= 0 || self.ext > 0 {
            self.carry_out(0);
        }
        let mut window = self.end_window;
        let mut used = self.nend_bits;
        while used >= SYM_BITS as i32 {
            self.end.push((window & SYM_MAX) as u8);
            window >>= SYM_BITS;
            used -= SYM_BITS as i32;
        }
        let mut buf = vec![0u8; size];
        let full = self.end.len().min(size);
        let room = size - full;
        let n = self.out.len().min(room);
        buf[..n].copy_from_slice(&self.out[..n]);
        for (k, &b) in self.end.iter().take(full).enumerate() {
            buf[size - 1 - k] = b;
        }
        if used > 0 && room > 0 {
            buf[room - 1] |= window as u8;
        }
        buf
    }
}

#[cfg(test)]
pub(crate) mod dec {
    //! Minimal range decoder (RFC 6716 §4.1) used to verify the encoder.
    use super::*;
    const CODE_EXTRA: u32 = (CODE_BITS - 2) % SYM_BITS + 1;

    pub struct RangeDecoder<'a> {
        buf: &'a [u8],
        offs: usize,
        end_offs: usize,
        end_window: u32,
        nend_bits: i32,
        nbits_total: i32,
        rng: u32,
        val: u32,
        rem: u32,
    }

    impl<'a> RangeDecoder<'a> {
        pub fn new(buf: &'a [u8]) -> Self {
            let mut d = RangeDecoder {
                buf,
                offs: 0,
                end_offs: 0,
                end_window: 0,
                nend_bits: 0,
                nbits_total: (CODE_BITS + 1 - ((CODE_BITS - CODE_EXTRA) / SYM_BITS) * SYM_BITS) as i32,
                rng: 1 << CODE_EXTRA,
                val: 0,
                rem: 0,
            };
            d.rem = d.read_byte();
            d.val = d.rng - 1 - (d.rem >> (SYM_BITS - CODE_EXTRA));
            d.normalize();
            d
        }
        fn read_byte(&mut self) -> u32 {
            if self.offs < self.buf.len() {
                self.offs += 1;
                self.buf[self.offs - 1] as u32
            } else {
                0
            }
        }
        fn read_byte_from_end(&mut self) -> u32 {
            if self.end_offs < self.buf.len() {
                self.end_offs += 1;
                self.buf[self.buf.len() - self.end_offs] as u32
            } else {
                0
            }
        }
        fn normalize(&mut self) {
            while self.rng <= CODE_BOT {
                self.nbits_total += SYM_BITS as i32;
                self.rng <<= SYM_BITS;
                let sym = self.rem;
                self.rem = self.read_byte();
                let sym = ((sym << SYM_BITS) | self.rem) >> (SYM_BITS - CODE_EXTRA);
                self.val = ((self.val << SYM_BITS).wrapping_add(SYM_MAX & !sym)) & (CODE_TOP - 1);
            }
        }
        pub fn rng(&self) -> u32 {
            self.rng
        }
        pub fn decode(&mut self, ft: u32) -> u32 {
            let ext = self.rng / ft;
            let s = self.val / ext;
            ft - (s + 1).min(ft)
        }
        pub fn update(&mut self, fl: u32, fh: u32, ft: u32) {
            let ext = self.rng / ft;
            let s = ext * (ft - fh);
            self.val = self.val.wrapping_sub(s);
            self.rng = if fl > 0 { ext * (fh - fl) } else { self.rng - s };
            self.normalize();
        }
        pub fn bit_logp(&mut self, logp: u32) -> bool {
            let r = self.rng;
            let d = self.val;
            let s = r >> logp;
            let ret = d < s;
            if !ret {
                self.val = d - s;
            }
            self.rng = if ret { s } else { r - s };
            self.normalize();
            ret
        }
        pub fn icdf(&mut self, icdf: &[u8], ftb: u32) -> usize {
            let mut s = self.rng;
            let d = self.val;
            let r = s >> ftb;
            let mut ret = 0usize;
            let mut t;
            loop {
                t = s;
                s = r * icdf[ret] as u32;
                if d >= s {
                    break;
                }
                ret += 1;
            }
            self.val = d - s;
            self.rng = t - s;
            self.normalize();
            ret
        }
        pub fn uint(&mut self, ft: u32) -> u32 {
            let ft1 = ft - 1;
            let ftb = ilog(ft1);
            if ftb > 8 {
                let ftb = (ftb - 8) as u32;
                let top = (ft1 >> ftb) + 1;
                let s = self.decode(top);
                self.update(s, s + 1, top);
                (s << ftb) | self.bits(ftb)
            } else {
                let s = self.decode(ft);
                self.update(s, s + 1, ft);
                s
            }
        }
        pub fn bits(&mut self, bits: u32) -> u32 {
            let mut window = self.end_window;
            let mut available = self.nend_bits;
            if (available as u32) < bits {
                loop {
                    window |= self.read_byte_from_end() << available;
                    available += SYM_BITS as i32;
                    if available > (CODE_BITS - SYM_BITS) as i32 {
                        break;
                    }
                }
            }
            let ret = window & ((1u32 << bits) - 1);
            self.end_window = window >> bits;
            self.nend_bits = available - bits as i32;
            self.nbits_total += bits as i32;
            ret
        }
        pub fn tell(&self) -> i32 {
            self.nbits_total - ilog(self.rng)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::dec::RangeDecoder;
    use super::*;

    struct Lcg(u32);
    impl Lcg {
        fn next(&mut self) -> u32 {
            self.0 = self.0.wrapping_mul(1664525).wrapping_add(1013904223);
            self.0 >> 8
        }
    }

    #[test]
    fn starts_at_one_bit() {
        let e = RangeEncoder::new();
        assert_eq!(e.tell(), 1);
        assert_eq!(e.tell_frac(), 8);
    }

    #[test]
    fn roundtrip_mixed_symbols_and_tell_matches() {
        let mut rng = Lcg(7);
        let icdf: [u8; 4] = [200, 120, 30, 0];
        for trial in 0..300 {
            let n = 1 + (rng.next() % 300) as usize;
            let mut ops = Vec::new();
            for _ in 0..n {
                match rng.next() % 5 {
                    0 => {
                        let ft = 2 + rng.next() % 1000;
                        ops.push((0u8, rng.next() % ft, ft));
                    }
                    1 => {
                        let logp = 1 + rng.next() % 15;
                        ops.push((1, rng.next().is_multiple_of(5) as u32, logp));
                    }
                    2 => {
                        let bits = 1 + rng.next() % 24;
                        ops.push((2, rng.next() & ((1 << bits) - 1), bits));
                    }
                    3 => {
                        let ft = 2 + rng.next() % 100_000_000;
                        ops.push((3, rng.next() % ft, ft));
                    }
                    _ => ops.push((4, rng.next() % 4, 0)),
                }
            }
            let mut e = RangeEncoder::new();
            let mut tells = Vec::new();
            for &(k, v, p) in &ops {
                match k {
                    0 => e.encode(v, v + 1, p),
                    1 => e.bit_logp(v != 0, p),
                    2 => e.bits(v, p),
                    3 => e.uint(v, p),
                    _ => e.icdf(v as usize, &icdf, 8),
                }
                tells.push(e.tell());
            }
            // Exactly the number of bytes `tell` says were used must suffice.
            let size = (e.tell() as usize).div_ceil(8);
            let final_rng = e.rng();
            let buf = e.done(size);
            assert_eq!(buf.len(), size);
            let mut d = RangeDecoder::new(&buf);
            for (i, &(k, v, p)) in ops.iter().enumerate() {
                let got = match k {
                    0 => {
                        let s = d.decode(p);
                        d.update(s, s + 1, p);
                        s
                    }
                    1 => d.bit_logp(p) as u32,
                    2 => d.bits(p),
                    3 => d.uint(p),
                    _ => d.icdf(&icdf, 8) as u32,
                };
                assert_eq!(got, v, "trial {trial} op {i} kind {k}");
                assert_eq!(d.tell(), tells[i], "tell mismatch trial {trial} op {i}");
            }
            assert_eq!(d.rng(), final_rng);
        }
    }
}
