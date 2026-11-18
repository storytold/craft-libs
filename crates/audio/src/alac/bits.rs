// Ported from Apple's ALAC reference implementation (ALACBitUtilities.c, macosforge/alac@c38887c,
// https://github.com/macosforge/alac). Copyright (c) 2011 Apple Inc.
// Licensed under the Apache License, Version 2.0 (LICENSE-APACHE at the repository root).
// Modified: translated to Rust and changed as the `alac` module docs describe.
// SPDX-License-Identifier: Apache-2.0

//! MSB-first bit reading and writing, from Apple's `ALACBitUtilities.c`.
//!
//! The reader never reads past its slice: [`BitReader::peek`] pads with zero bits, and every
//! read that would end past the data is [`Error::Eof`]. The writer can rewind, as the reference
//! encoder does when a compressed element turns out larger than its escape (uncompressed) form.

use super::{Error, Result};

/// Reader over one packet.
#[derive(Clone, Copy)]
pub(crate) struct BitReader<'a> {
    data: &'a [u8],
    /// Position in bits from the start of `data`.
    pos: usize,
}

impl<'a> BitReader<'a> {
    pub(crate) fn new(data: &'a [u8]) -> Self {
        Self { data, pos: 0 }
    }

    /// Bit position from the start.
    pub(crate) fn position(&self) -> usize {
        self.pos
    }

    /// Bits in the packet.
    pub(crate) fn len_bits(&self) -> usize {
        self.data.len().saturating_mul(8)
    }

    /// The next `n` (at most 32) bits at bit `pos`, right-aligned, with zeros past the end.
    pub(crate) fn peek_at(&self, pos: usize, n: u32) -> u32 {
        if n == 0 {
            return 0;
        }
        let byte = pos / 8;
        let mut window: u64 = 0;
        for i in 0..8 {
            window = (window << 8) | u64::from(self.data.get(byte.wrapping_add(i)).copied().unwrap_or(0));
        }
        // `pos % 8 + n` is at most 39, so the wanted bits lie in the 64-bit window.
        let shift = 64 - (pos % 8) as u32 - n.min(32);
        ((window >> shift) & ((1u64 << n.min(32)) - 1)) as u32
    }

    /// The next `n` bits without consuming them.
    pub(crate) fn peek(&self, n: u32) -> u32 {
        self.peek_at(self.pos, n)
    }

    /// Read `n` (at most 32) bits.
    pub(crate) fn read(&mut self, n: u32) -> Result<u32> {
        let end = self.pos.checked_add(n as usize).ok_or(Error::Eof)?;
        if end > self.len_bits() {
            return Err(Error::Eof);
        }
        let v = self.peek(n);
        self.pos = end;
        Ok(v)
    }

    /// Read `n` bits as a two's-complement value of that width.
    pub(crate) fn read_signed(&mut self, n: u32) -> Result<i32> {
        let v = self.read(n)?;
        Ok(sign_extend(v, n))
    }

    /// Skip `n` bits; skipping past the end is [`Error::Eof`].
    pub(crate) fn skip(&mut self, n: usize) -> Result<()> {
        let end = self.pos.checked_add(n).ok_or(Error::Eof)?;
        if end > self.len_bits() {
            return Err(Error::Eof);
        }
        self.pos = end;
        Ok(())
    }

    /// Move to `pos`, which the caller has checked lies within the packet.
    pub(crate) fn set_position(&mut self, pos: usize) -> Result<()> {
        if pos > self.len_bits() {
            return Err(Error::Eof);
        }
        self.pos = pos;
        Ok(())
    }

    /// Skip to the next byte boundary (`BitBufferByteAlign` without zeros).
    pub(crate) fn byte_align(&mut self) {
        // The packet is whole bytes, so the boundary is never past its end.
        self.pos = self.pos.div_ceil(8).saturating_mul(8);
    }
}

/// The low `n` bits of `v` as a signed value (`n` of 0 gives 0, 32 or more the value itself).
pub(crate) fn sign_extend(v: u32, n: u32) -> i32 {
    match n {
        0 => 0,
        1..=31 => {
            let s = 32 - n;
            ((v << s) as i32) >> s
        }
        _ => v as i32,
    }
}

/// Where bits go: a [`BitWriter`], or a [`BitCounter`] for the encoder's trial runs.
pub(crate) trait BitSink {
    /// Write the low `n` (at most 32) bits of `value`.
    fn put(&mut self, value: u32, n: u32);
    /// Bits written so far.
    fn bit_len(&self) -> usize;
}

/// Growable MSB-first writer.
#[derive(Default)]
pub(crate) struct BitWriter {
    buf: Vec<u8>,
    bits: usize,
}

impl BitWriter {
    pub(crate) fn new() -> Self {
        Self::default()
    }

    /// Drop everything after bit `pos` (no-op when `pos` is past the end).
    pub(crate) fn truncate(&mut self, pos: usize) {
        if pos >= self.bits {
            return;
        }
        self.bits = pos;
        self.buf.truncate(pos.div_ceil(8));
        if let Some(last) = self.buf.last_mut()
            && !pos.is_multiple_of(8)
        {
            *last &= 0xFFu8 << (8 - pos % 8);
        }
    }

    /// Pad with zero bits to a byte boundary.
    pub(crate) fn byte_align(&mut self) {
        let pad = (8 - self.bits % 8) % 8;
        self.put(0, pad as u32);
    }

    pub(crate) fn into_bytes(mut self) -> Vec<u8> {
        self.byte_align();
        self.buf
    }
}

impl BitSink for BitWriter {
    fn put(&mut self, value: u32, n: u32) {
        let n = n.min(32);
        for i in (0..n).rev() {
            let bit = ((value >> i) & 1) as u8;
            if self.bits.is_multiple_of(8) {
                self.buf.push(0);
            }
            if let Some(last) = self.buf.last_mut() {
                *last |= bit << (7 - self.bits % 8);
            }
            self.bits += 1;
        }
    }

    fn bit_len(&self) -> usize {
        self.bits
    }
}

/// Counts bits without storing them.
#[derive(Default)]
pub(crate) struct BitCounter(pub(crate) usize);

impl BitSink for BitCounter {
    fn put(&mut self, _value: u32, n: u32) {
        self.0 += n.min(32) as usize;
    }

    fn bit_len(&self) -> usize {
        self.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn writer_and_reader_agree() {
        let mut w = BitWriter::new();
        w.put(0b101, 3);
        w.put(0xABCD, 16);
        w.put(0xDEAD_BEEF, 32);
        w.put(1, 1);
        assert_eq!(w.bit_len(), 52);
        let bytes = w.into_bytes();
        let mut r = BitReader::new(&bytes);
        assert_eq!(r.read(3), Ok(0b101));
        assert_eq!(r.read(16), Ok(0xABCD));
        assert_eq!(r.read(32), Ok(0xDEAD_BEEF));
        assert_eq!(r.read(1), Ok(1));
        assert_eq!(r.read(4), Ok(0));
        assert_eq!(r.read(1), Err(Error::Eof));
        assert_eq!(r.peek(32), 0);
    }

    #[test]
    fn truncate_clears_the_dropped_bits() {
        let mut w = BitWriter::new();
        w.put(0xFFFF, 16);
        w.truncate(5);
        w.put(0, 3);
        assert_eq!(w.into_bytes(), vec![0xF8]);
    }

    #[test]
    fn sign_extension() {
        assert_eq!(sign_extend(0xF, 4), -1);
        assert_eq!(sign_extend(0x7, 4), 7);
        assert_eq!(sign_extend(0x8000_0000, 32), i32::MIN);
        assert_eq!(sign_extend(5, 0), 0);
    }
}
