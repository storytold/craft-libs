//! The table-driven Huffman decoder and MSB-first bit reader that the makers' difference codings share (Pentax PEF,
//! Nikon NEF, Panasonic RW2, Sony ARW6): a code names a difference category, the category's additional bits follow
//! as in ITU-T T.81 F.1.2.1.

use crate::{RawError, Result};

/// A canonical-by-table Huffman decoder with a 12-bit lookup that also holds the difference when code and
/// additional bits fit in 12 bits.
pub(crate) struct Huffman {
    /// Indexed by the next 12 bits. Per entry: bits 0-7 the symbol, bits 8-11 the code length (0 = invalid), bits
    /// 12-15 the length of code plus additional bits when both fit in 12 bits (0 = they do not), bits 16-31 the
    /// signed difference in that case.
    lut: Box<[u32; 4096]>,
}

impl Huffman {
    /// From `(left-aligned 12-bit code, length)` per symbol.
    pub fn new(codes: &[(u16, u8)]) -> Result<Huffman> {
        let mut lut = Box::new([0u32; 4096]);
        for (sym, &(code, len)) in codes.iter().enumerate() {
            if len == 0 {
                continue;
            }
            if len > 12 || sym > 16 {
                return Err(RawError::Unsupported(format!("PEF Huffman code of length {len}")));
            }
            let (len, s) = (len as u32, sym as u32);
            let first = (code >> (12 - len) << (12 - len)) as usize;
            for (i, e) in lut[first..first + (1 << (12 - len))].iter_mut().enumerate() {
                if (*e >> 8) & 15 == 0 {
                    let mut entry = (len << 8) | s;
                    if s == 0 {
                        entry |= len << 12;
                    } else if len + s <= 12 {
                        let v = (((first + i) >> (12 - len - s)) & ((1 << s) - 1)) as i32;
                        let d = if v < 1 << (s - 1) { v - (1 << s) + 1 } else { v };
                        entry |= ((len + s) << 12) | ((d as i16 as u16 as u32) << 16);
                    }
                    *e = entry;
                }
            }
        }
        Ok(Huffman { lut })
    }
}

/// MSB-first bit reader that yields zeros past the end.
pub(crate) struct Bits<'a> {
    src: &'a [u8],
    pos: usize,
    acc: u64,
    n: u32,
}

impl<'a> Bits<'a> {
    pub fn new(src: &'a [u8]) -> Self {
        Bits { src, pos: 0, acc: 0, n: 0 }
    }
    #[inline]
    fn fill(&mut self) {
        while self.n <= 56 {
            let b = self.src.get(self.pos).copied().unwrap_or(0);
            self.pos += 1;
            self.acc |= (b as u64) << (56 - self.n);
            self.n += 8;
        }
    }
    #[inline]
    pub fn peek(&mut self, k: u32) -> u32 {
        if self.n < k {
            self.fill();
        }
        (self.acc >> (64 - k)) as u32
    }
    #[inline]
    pub fn skip(&mut self, k: u32) {
        self.acc <<= k;
        self.n -= k;
    }
    #[inline]
    pub fn get(&mut self, k: u32) -> u32 {
        if k == 0 {
            return 0;
        }
        let v = self.peek(k);
        self.skip(k);
        v
    }
    /// Bits handed out so far (may exceed the source length: zeros are read past its end).
    pub fn consumed_bits(&self) -> usize {
        self.pos * 8 - self.n as usize
    }
    /// Whether more bits were handed out than the source holds (the zeros read past its end are lookahead padding,
    /// not data; using them would fabricate samples).
    pub fn overrun(&self) -> bool {
        self.consumed_bits() > self.src.len() * 8
    }
}

/// Decode one difference: Huffman category then T.81 additional bits, in one lookup when both fit in 12 bits.
#[inline]
pub(crate) fn diff(bits: &mut Bits, h: &Huffman) -> Option<i32> {
    let e = h.lut[(bits.peek(12) & 0xfff) as usize];
    let len = (e >> 8) & 15;
    if len == 0 {
        return None;
    }
    let total = (e >> 12) & 15;
    if total > 0 {
        bits.skip(total);
        return Some((e as i32) >> 16);
    }
    bits.skip(len);
    let s = e & 0xff;
    if s > 16 {
        return None;
    }
    let v = bits.get(s) as i32;
    Some(if v < 1 << (s - 1) { v - (1 << s) + 1 } else { v })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Canonical left-aligned 12-bit codes for symbols 0..lens.len() (shorter codes first, ties by symbol).
    fn canonical(lens: &[u8]) -> Vec<(u16, u8)> {
        let mut order: Vec<usize> = (0..lens.len()).collect();
        order.sort_by_key(|&s| (lens[s], s));
        let mut codes = vec![(0u16, 0u8); lens.len()];
        let (mut code, mut prev) = (0u32, 0u8);
        for s in order {
            code <<= lens[s] - prev;
            prev = lens[s];
            codes[s] = ((code << (12 - lens[s])) as u16, lens[s]);
            code += 1;
        }
        codes
    }

    fn bit(src: &[u8], i: usize) -> u32 {
        src.get(i / 8).map_or(0, |b| ((b >> (7 - i % 8)) & 1) as u32)
    }

    /// Reference: match the code bit by bit, then read the category's additional bits (T.81 F.1.2.1).
    fn reference(src: &[u8], pos: &mut usize, codes: &[(u16, u8)]) -> Option<i32> {
        let window: u32 = (0..12).fold(0, |a, k| (a << 1) | bit(src, *pos + k));
        let (sym, len) = codes
            .iter()
            .enumerate()
            .find_map(|(s, &(c, l))| (l > 0 && (c as u32 >> (12 - l)) == window >> (12 - l)).then_some((s as u32, l as usize)))?;
        *pos += len;
        if sym == 0 {
            return Some(0);
        }
        let v = (0..sym as usize).fold(0u32, |a, k| (a << 1) | bit(src, *pos + k)) as i32;
        *pos += sym as usize;
        Some(if v < 1 << (sym - 1) { v - (1 << sym) + 1 } else { v })
    }

    /// `diff` against the reference on random streams (invalid codes are stepped over) and on every (code, extra
    /// bits) pair of every symbol.
    fn check_table(lens: &[u8]) {
        let codes = canonical(lens);
        let h = Huffman::new(&codes).unwrap();
        for (sym, &(code, len)) in codes.iter().enumerate() {
            for v in 0..1u64 << sym {
                let (len, n) = (len as u32, sym as u32);
                let word = (((code >> (12 - len)) as u64) << n | v) << (64 - len - n);
                let src = word.to_be_bytes();
                let mut pos = 0;
                let want = reference(&src, &mut pos, &codes);
                let mut bits = Bits::new(&src);
                assert_eq!(diff(&mut bits, &h), want, "symbol {sym} bits {v:#b}");
                assert_eq!(bits.consumed_bits(), pos);
            }
        }
        let mut s = 0x1234_5678_9abc_def0u64;
        let (mut invalid, mut long) = (0, 0);
        for _ in 0..40 {
            let src: Vec<u8> = (0..400)
                .map(|_| {
                    s = s.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
                    (s >> 40) as u8
                })
                .collect();
            let mut bits = Bits::new(&src);
            let mut pos = 0;
            while pos < src.len() * 8 - 64 {
                let before = pos;
                let want = reference(&src, &mut pos, &codes);
                assert_eq!(diff(&mut bits, &h), want);
                if want.is_none() {
                    // step over the invalid code
                    invalid += 1;
                    pos += 1;
                    bits.skip(1);
                } else {
                    long += (pos - before > 12) as u32;
                }
                assert_eq!(bits.consumed_bits(), pos);
            }
        }
        assert!(long > 0, "{invalid} invalid codes, no symbol longer than the lookup");
    }

    #[test]
    fn diff_matches_bitwise_reference_for_every_category() {
        // categories 0..=16, code lengths up to 12 so that short codes combine with their bits and long ones do not
        check_table(&[2, 2, 3, 3, 4, 4, 5, 6, 7, 8, 9, 10, 11, 12, 12, 12, 12]);
    }

    #[test]
    fn diff_matches_reference_when_code_and_bits_fill_the_lookup_exactly() {
        // category 8 with a 4-bit code and category 7 with a 5-bit code: len + category == 12
        check_table(&[2, 3, 3, 5, 5, 5, 6, 5, 4, 7, 8, 9, 10, 11, 12, 12, 12]);
    }
}
