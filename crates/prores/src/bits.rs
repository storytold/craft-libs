//! Fast MSB-first bit reader/writer for slice payloads (the hot path of the codec).
//!
//! Frame and picture headers use `filmcraft_bitstream`; the entropy-coded slice data uses these
//! specialised helpers, which read 64-bit windows and decode a whole codeword per call.

use crate::tables::Codebook;

/// Bit reader over one slice component with a 64-bit MSB-aligned cache. Reads past the end
/// yield zero bits; callers check [`Reader::overrun`] once a component has been consumed.
pub(crate) struct Reader<'a> {
    data: &'a [u8],
    /// Next byte of `data` to load into the cache (may run past the end: zeros are loaded).
    next: usize,
    /// Cached bits, MSB first; bits below `avail` are zero.
    cache: u64,
    avail: u32,
}

impl<'a> Reader<'a> {
    #[inline]
    pub fn new(data: &'a [u8]) -> Self {
        let mut r = Self { data, next: 0, cache: 0, avail: 0 };
        r.refill();
        r
    }

    /// Ensure at least 56 valid bits are cached.
    #[inline(always)]
    fn refill(&mut self) {
        if self.avail >= 56 {
            return;
        }
        if let Some(s) = self.data.get(self.next..self.next + 8) {
            let w = u64::from_be_bytes([s[0], s[1], s[2], s[3], s[4], s[5], s[6], s[7]]);
            self.cache |= w >> self.avail;
            self.next += ((63 - self.avail) >> 3) as usize;
            self.avail |= 56;
        } else {
            while self.avail <= 56 {
                let b = self.data.get(self.next).copied().unwrap_or(0) as u64;
                self.cache |= b << (56 - self.avail);
                self.avail += 8;
                self.next += 1;
            }
        }
    }

    #[inline(always)]
    fn consume(&mut self, n: u32) {
        // n <= 56 <= avail
        self.cache = if n >= 64 { 0 } else { self.cache << n };
        self.avail -= n;
    }

    #[inline]
    fn consumed(&self) -> usize {
        self.next * 8 - self.avail as usize
    }

    #[inline]
    pub fn bits_left(&self) -> isize {
        (self.data.len() * 8) as isize - self.consumed() as isize
    }

    #[inline]
    pub fn overrun(&self) -> bool {
        self.consumed() > self.data.len() * 8
    }

    /// True when every remaining bit is zero (the component's padding), or nothing is left.
    #[inline(always)]
    pub fn only_padding_left(&mut self) -> bool {
        let left = self.bits_left();
        if left > 56 {
            return false;
        }
        if left <= 0 {
            return true;
        }
        self.refill();
        self.cache >> (64 - left as u32) == 0
    }

    #[inline]
    pub fn read(&mut self, n: u32) -> u32 {
        debug_assert!(n <= 32);
        if n == 0 {
            return 0;
        }
        self.refill();
        let v = (self.cache >> (64 - n)) as u32;
        self.consume(n);
        v
    }

    #[inline(always)]
    pub fn read_bit(&mut self) -> bool {
        if self.avail == 0 {
            self.refill();
        }
        let b = self.cache >> 63 == 1;
        self.consume(1);
        b
    }

    /// Decode one codeword; `None` for codewords longer than 56 bits (never produced by valid
    /// streams).
    #[inline(always)]
    pub fn read_cw(&mut self, cb: Codebook) -> Option<u32> {
        self.refill();
        let w = self.cache;
        let q = w.leading_zeros();
        let rq = cb.rice_q_max as u32;
        if q <= rq {
            let rice = cb.rice as u32;
            let len = q + 1 + rice;
            let low = if rice == 0 { 0 } else { ((w << (q + 1)) >> (64 - rice)) as u32 };
            self.consume(len);
            Some((q << rice) | low)
        } else {
            let exp = cb.exp as u32;
            let len = 2 * q - rq + exp;
            if len > 56 {
                return None;
            }
            let code = w >> (64 - len);
            self.consume(len);
            let v = code - (1u64 << exp) + (((rq + 1) as u64) << cb.rice);
            u32::try_from(v).ok()
        }
    }
}

/// Number of bits of the codeword for `val` in codebook `cb`.
#[inline]
pub(crate) fn cw_len(cb: Codebook, val: u32) -> u32 {
    let rq = cb.rice_q_max as u32;
    let rice = cb.rice as u32;
    let rice_span = (rq + 1) << rice;
    if val < rice_span {
        (val >> rice) + 1 + rice
    } else {
        let exp = cb.exp as u32;
        let v = (val - rice_span) as u64 + (1u64 << exp);
        let nbits = 64 - v.leading_zeros(); // n + exp + 1
        let n = nbits - exp - 1;
        (rq + 1 + n) + nbits
    }
}

/// A bit sink: either a real writer or a bit counter (for rate control trials).
pub(crate) trait Sink {
    fn put(&mut self, val: u32, bits: u32);
    fn put_cw(&mut self, cb: Codebook, val: u32);
}

/// Counts bits without storing them.
#[derive(Default)]
pub(crate) struct Counter(pub u64);

impl Sink for Counter {
    #[inline]
    fn put(&mut self, _val: u32, bits: u32) {
        self.0 += bits as u64;
    }
    #[inline]
    fn put_cw(&mut self, cb: Codebook, val: u32) {
        self.0 += cw_len(cb, val) as u64;
    }
}

/// MSB-first bit writer.
#[derive(Default)]
pub(crate) struct Writer {
    pub buf: Vec<u8>,
    acc: u64,
    n: u32,
}

impl Writer {
    pub fn new() -> Self {
        Self::default()
    }
    /// Pad with zero bits to a byte boundary and return the bytes.
    pub fn finish(mut self) -> Vec<u8> {
        if self.n > 0 {
            let pad = 8 - self.n;
            self.put(0, pad);
        }
        self.buf
    }
}

impl Sink for Writer {
    #[inline]
    fn put(&mut self, val: u32, bits: u32) {
        debug_assert!(bits <= 32);
        if bits == 0 {
            return;
        }
        let v = (val as u64) & ((1u64 << bits) - 1);
        self.acc = (self.acc << bits) | v;
        self.n += bits;
        while self.n >= 8 {
            self.n -= 8;
            self.buf.push((self.acc >> self.n) as u8);
        }
        self.acc &= (1u64 << self.n) - 1;
    }

    fn put_cw(&mut self, cb: Codebook, val: u32) {
        let rq = cb.rice_q_max as u32;
        let rice = cb.rice as u32;
        let rice_span = (rq + 1) << rice;
        if val < rice_span {
            let q = val >> rice;
            // q zeros, a one, then `rice` low bits
            self.put(0, q);
            self.put(1, 1);
            self.put(val & ((1 << rice) - 1), rice);
        } else {
            let exp = cb.exp as u32;
            let v = (val - rice_span) as u64 + (1u64 << exp);
            let nbits = 64 - v.leading_zeros();
            let n = nbits - exp - 1;
            let zeros = rq + 1 + n;
            self.put(0, zeros.min(32));
            if zeros > 32 {
                self.put(0, zeros - 32);
            }
            if nbits > 32 {
                self.put((v >> 32) as u32, nbits - 32);
                self.put(v as u32, 32);
            } else {
                self.put(v as u32, nbits);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tables::*;

    #[test]
    fn codeword_roundtrip_all_codebooks() {
        let mut books = vec![FIRST_DC_CB];
        books.extend_from_slice(&DC_CB);
        books.extend_from_slice(&RUN_CB);
        books.extend_from_slice(&LEVEL_CB);
        let vals: Vec<u32> = (0..300).chain([1000, 4095, 65535, 1 << 20]).collect();
        for cb in books {
            let mut w = Writer::new();
            let mut total = 0;
            for &v in &vals {
                w.put_cw(cb, v);
                total += cw_len(cb, v);
            }
            let bits = w.buf.len() as u32 * 8 + w.n;
            assert_eq!(bits, total);
            let data = w.finish();
            let mut r = Reader::new(&data);
            for &v in &vals {
                assert_eq!(r.read_cw(cb), Some(v), "{cb:?}");
            }
            assert!(!r.overrun());
        }
    }

    #[test]
    fn exp_golomb5_equivalence() {
        // FIRST_DC_CB is Exp-Golomb order 5: value 32 is `01` + 6 bits (000000)
        assert_eq!(cw_len(FIRST_DC_CB, 0), 6);
        assert_eq!(cw_len(FIRST_DC_CB, 31), 6);
        assert_eq!(cw_len(FIRST_DC_CB, 32), 8);
        assert_eq!(cw_len(FIRST_DC_CB, 95), 8);
        assert_eq!(cw_len(FIRST_DC_CB, 96), 10);
    }
}
