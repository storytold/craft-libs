//! Huffman entropy decoding: code tables, the bit reader and the Huffman scan decoder.

use super::scan::*;
use super::*;

/// A Huffman table in the decoding form of F.2.2.3 (`MAXCODE`, `VALPTR`, `MINCODE`).
#[derive(Clone)]
pub(super) struct Huffman {
    pub(super) maxcode: [i32; 17],
    pub(super) valptr: [i32; 17],
    pub(super) mincode: [i32; 17],
    pub(super) values: Vec<u8>,
    /// For each value of the next [`LOOKAHEAD`] bits: `length << 8 | value` of the code they
    /// begin with, or 0 when it is longer (or not a code). Built from the procedure of
    /// [`Huffman::decode`] itself, so both give the same symbol.
    pub(super) fast: Vec<u16>,
}

/// Bits [`Huffman::fast`] resolves at once; nearly every code in real tables is this short.
const LOOKAHEAD: u32 = 9;

impl Huffman {
    pub(super) fn new(counts: &[u8], values: &[u8]) -> Result<Self, Error> {
        let mut h = Huffman { maxcode: [-1; 17], valptr: [0; 17], mincode: [0; 17], values: values.to_vec(), fast: Vec::new() };
        let mut code: i32 = 0;
        let mut k: i32 = 0;
        for (l, &n) in (1..=16usize).zip(counts) {
            let n = i32::from(n);
            if n > 0 {
                h.valptr[l] = k;
                h.mincode[l] = code;
                code += n;
                k += n;
                h.maxcode[l] = code - 1;
                if code > 1 << l {
                    return Err(err("Huffman table has too many codes"));
                }
            }
            code <<= 1;
        }
        h.fast = (0..1u32 << LOOKAHEAD)
            .map(|bits| {
                let mut code: i32 = 0;
                for l in 1..=LOOKAHEAD as usize {
                    code = (code << 1) | ((bits >> (LOOKAHEAD as usize - l)) & 1) as i32;
                    if code <= h.maxcode[l] {
                        let value = usize::try_from(h.valptr[l] + code - h.mincode[l]).ok().and_then(|i| h.values.get(i));
                        return value.map_or(0, |&v| (l as u16) << 8 | u16::from(v));
                    }
                }
                0
            })
            .collect();
        Ok(h)
    }

    pub(super) fn decode(&self, bits: &mut Bits) -> Result<u8, Error> {
        if let Some(&e) = self.fast.get(bits.peek(LOOKAHEAD) as usize)
            && e != 0
        {
            bits.get(u32::from(e >> 8));
            return Ok(e as u8);
        }
        let mut code: i32 = 0;
        for l in 1..=16 {
            code = (code << 1) | bits.bit() as i32;
            if code <= self.maxcode[l] {
                let idx = self.valptr[l] + code - self.mincode[l];
                return usize::try_from(idx).ok().and_then(|i| self.values.get(i)).copied().ok_or_else(|| err("bad Huffman code"));
            }
        }
        Err(err("bad Huffman code"))
    }
}

/// Huffman bit reader (F.2.2.5).
pub(super) struct Bits<'a> {
    pub(super) d: &'a [u8],
    pub(super) pos: usize,
    pub(super) acc: u32,
    pub(super) n: u32,
    /// How many of the `n` buffered bits (always the last ones) are zero padding supplied at
    /// a marker or the end of the data, not real entropy-coded bits.
    pub(super) pad: u32,
    /// Set once a read consumed padding: the scan's data ended before the scan did.
    pub(super) exhausted: bool,
}

impl<'a> Bits<'a> {
    pub(super) fn new(d: &'a [u8]) -> Self {
        Bits { d, pos: 0, acc: 0, n: 0, pad: 0, exhausted: false }
    }

    pub(super) fn fill(&mut self) {
        while self.n <= 24 {
            let v = match next_byte(self.d, &mut self.pos) {
                Some(v) => v,
                None => {
                    self.pad += 8;
                    0
                }
            };
            self.acc |= u32::from(v) << (24 - self.n);
            self.n += 8;
        }
    }

    pub(super) fn bit(&mut self) -> u32 {
        self.get(1)
    }

    /// Whether whole bytes of entropy-coded data are left before the next marker: more than the
    /// padding (under 8 bits, F.1.2.3) that ends a scan or a restart interval.
    pub(super) fn data_left(&self) -> bool {
        if self.n.saturating_sub(self.pad) >= 8 {
            return true;
        }
        let mut p = self.pos;
        while self.d.get(p) == Some(&0xFF) {
            p += 1;
        }
        // A data byte, or a stuffed `FF 00` (fill bytes before a marker are not data).
        self.d.get(p).is_some_and(|&v| p == self.pos || v == 0x00)
    }

    /// The next `n` (≤ 16) bits, left in place.
    pub(super) fn peek(&mut self, n: u32) -> u32 {
        if self.n < n {
            self.fill();
        }
        self.acc >> (32 - n)
    }

    /// The next `n` (≤ 16) bits.
    pub(super) fn get(&mut self, n: u32) -> u32 {
        if n == 0 {
            return 0;
        }
        if self.n < n {
            self.fill();
        }
        if n > self.n.saturating_sub(self.pad) {
            self.exhausted = true;
        }
        let v = self.acc >> (32 - n);
        self.acc <<= n;
        self.n -= n;
        self.pad = self.pad.min(self.n);
        v
    }

    /// Past the next restart marker, with nothing buffered: a fresh start, including after an
    /// interval whose data ran out (the band decoder resumes there; [`decode`] stops at it).
    pub(super) fn restart(&mut self) {
        self.pos = skip_restart(self.d, self.pos).0;
        self.acc = 0;
        self.n = 0;
        self.pad = 0;
        self.exhausted = false;
    }
}

/// `EXTEND` of F.2.2.1: the signed value of the `t`-bit magnitude `v`.
pub(super) fn extend(v: u32, t: u32) -> i32 {
    let v = v as i32;
    if t == 0 {
        0
    } else if v < 1 << (t - 1) {
        v - (1 << t) + 1
    } else {
        v
    }
}

/// Huffman sequential decoding (F.2.2).
pub(super) struct HuffmanScan<'a> {
    pub(super) bits: Bits<'a>,
    pub(super) tables: Vec<(&'a Huffman, &'a Huffman)>,
    pub(super) pred: [i32; 4],
}

impl ScanDecoder for HuffmanScan<'_> {
    fn restart(&mut self) {
        self.bits.restart();
        self.pred = [0; 4];
    }

    fn block(&mut self, sc: usize, blk: &mut [i32; 64]) -> Result<(), Error> {
        let &(dc, ac) = self.tables.get(sc).ok_or_else(|| err("bad scan component"))?;
        let pred = self.pred.get_mut(sc).ok_or_else(|| err("bad scan component"))?;
        let t = u32::from(dc.decode(&mut self.bits)?);
        if t > 15 {
            return Err(err("bad DC difference category"));
        }
        let diff = extend(self.bits.get(t), t);
        *pred = pred.wrapping_add(diff);
        blk[0] = *pred;
        let mut k = 1;
        while k < 64 {
            let rs = ac.decode(&mut self.bits)?;
            let (r, s) = (usize::from(rs >> 4), u32::from(rs & 15));
            if s == 0 {
                if r != 15 {
                    break;
                }
                k += 16;
                continue;
            }
            k += r;
            let slot = blk.get_mut(k).ok_or_else(|| err("AC coefficient past the end of the block"))?;
            *slot = extend(self.bits.get(s), s);
            k += 1;
        }
        if self.bits.exhausted {
            return Err(err("entropy-coded data ends before the scan does"));
        }
        Ok(())
    }
}
