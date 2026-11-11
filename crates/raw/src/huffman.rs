//! The table-driven Huffman decoder and MSB-first bit reader that the makers' difference codings share (Pentax PEF,
//! Nikon NEF, Panasonic RW2, Sony ARW6): a code names a difference category, the category's additional bits follow
//! as in ITU-T T.81 F.1.2.1.

use crate::{RawError, Result};

/// A canonical-by-table Huffman decoder with a 12-bit lookup.
pub(crate) struct Huffman {
    /// Indexed by the next 12 bits: (symbol, length); length 0 = invalid.
    lut: Vec<(u8, u8)>,
}

impl Huffman {
    /// From `(left-aligned 12-bit code, length)` per symbol.
    pub fn new(codes: &[(u16, u8)]) -> Result<Huffman> {
        let mut lut = vec![(0u8, 0u8); 4096];
        for (sym, &(code, len)) in codes.iter().enumerate() {
            if len == 0 {
                continue;
            }
            if len > 12 || sym > 16 {
                return Err(RawError::Unsupported(format!("PEF Huffman code of length {len}")));
            }
            let first = (code >> (12 - len) << (12 - len)) as usize;
            for e in &mut lut[first..first + (1 << (12 - len))] {
                if e.1 == 0 {
                    *e = (sym as u8, len);
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

/// Decode one difference: Huffman category then T.81 additional bits.
#[inline]
pub(crate) fn diff(bits: &mut Bits, h: &Huffman) -> Option<i32> {
    let (sym, len) = h.lut[bits.peek(12) as usize];
    if len == 0 {
        return None;
    }
    bits.skip(len as u32);
    let s = sym as u32;
    if s == 0 {
        return Some(0);
    }
    if s > 16 {
        return None;
    }
    let v = bits.get(s) as i32;
    Some(if v < 1 << (s - 1) { v - (1 << s) + 1 } else { v })
}
