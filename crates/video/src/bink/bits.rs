//! Bit reader of the Bink video decoder: bits are taken from the least significant end of each
//! byte, and a multi-bit value has its first bit lowest.

use super::{Error, Result};

pub(super) struct Bits<'a> {
    data: &'a [u8],
    /// Position in bits from the start of `data`.
    pos: usize,
}

impl<'a> Bits<'a> {
    pub(super) fn new(data: &'a [u8]) -> Bits<'a> {
        Bits { data, pos: 0 }
    }

    fn total(&self) -> usize {
        self.data.len().saturating_mul(8)
    }

    /// The bit at `at`, or 0 past the end of the data.
    fn bit_at(&self, at: usize) -> u32 {
        self.data.get(at / 8).map_or(0, |b| u32::from(b >> (at % 8)) & 1)
    }

    /// The next `n` bits (at most 24) without consuming them; bits past the end read as 0.
    pub(super) fn peek(&self, n: u32) -> u32 {
        (0..n.min(24)).fold(0, |v, k| v | self.bit_at(self.pos.saturating_add(k as usize)) << k)
    }

    /// Consumes `n` bits; an error if the data ends before that.
    pub(super) fn skip(&mut self, n: u32) -> Result<()> {
        let end = self.pos.saturating_add(n as usize);
        if end > self.total() {
            return Err(Error::Invalid("the packet ends inside a plane".into()));
        }
        self.pos = end;
        Ok(())
    }

    /// Reads `n` bits (at most 24).
    pub(super) fn get(&mut self, n: u32) -> Result<u32> {
        let v = self.peek(n);
        self.skip(n.min(24))?;
        Ok(v)
    }

    pub(super) fn bit(&mut self) -> Result<bool> {
        Ok(self.get(1)? != 0)
    }

    /// Moves to the next 32-bit boundary, where the next plane starts. The end of the data counts
    /// as a boundary, so a final plane need not be padded.
    pub(super) fn align32(&mut self) {
        self.pos = self.pos.saturating_add(31) / 32 * 32;
        self.pos = self.pos.min(self.total());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_least_significant_bits_first() {
        let mut b = Bits::new(&[0b1010_0110, 0xff, 0x01]);
        assert_eq!(b.get(1).unwrap(), 0);
        assert_eq!(b.get(3).unwrap(), 0b011);
        assert_eq!(b.peek(4), 0b1010);
        assert_eq!(b.get(8).unwrap(), 0b1111_1010);
        assert!(b.bit().unwrap());
        assert_eq!(b.get(11).unwrap(), 0b1111);
        assert!(b.get(1).is_err());
    }

    #[test]
    fn bits_past_the_end_peek_as_zero_and_do_not_read() {
        let mut b = Bits::new(&[0xff]);
        assert_eq!(b.peek(12), 0xff);
        assert!(b.get(9).is_err());
        assert_eq!(b.get(8).unwrap(), 0xff);
        assert!(b.skip(1).is_err());
        assert_eq!(Bits::new(&[]).peek(7), 0);
    }

    #[test]
    fn aligns_to_32_bits_or_the_end() {
        let data = [0u8; 9];
        let mut b = Bits::new(&data);
        b.skip(1).unwrap();
        b.align32();
        assert_eq!(b.pos, 32);
        b.align32();
        assert_eq!(b.pos, 32);
        b.skip(8).unwrap();
        b.align32();
        assert_eq!(b.pos, 64);
        b.skip(1).unwrap();
        b.align32();
        assert_eq!(b.pos, 72);
    }
}
