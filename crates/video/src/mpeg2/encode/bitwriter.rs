//! Big-endian bit writer for start-code units.

pub(crate) struct BitWriter {
    out: Vec<u8>,
    acc: u64,
    /// Bits held in `acc` (always below 8 between calls).
    n: u32,
}

impl BitWriter {
    pub fn new() -> Self {
        Self { out: Vec::new(), acc: 0, n: 0 }
    }

    /// Append the low `bits` (0..=32) bits of `v`, most significant first.
    #[inline(always)]
    pub fn put(&mut self, v: u32, bits: u32) {
        if bits == 0 {
            return;
        }
        let mask = if bits == 32 { u32::MAX } else { (1u32 << bits) - 1 };
        self.acc = (self.acc << bits) | u64::from(v & mask);
        self.n += bits;
        while self.n >= 8 {
            self.n -= 8;
            self.out.push((self.acc >> self.n) as u8);
        }
        self.acc &= (1u64 << self.n) - 1;
    }

    #[inline(always)]
    pub fn bit(&mut self, b: bool) {
        self.put(b as u32, 1);
    }

    /// Pad with zero bits to a byte boundary.
    pub fn align(&mut self) {
        if self.n > 0 {
            self.put(0, 8 - self.n);
        }
    }

    /// Byte-align and write the start code `00 00 01 code`.
    pub fn start_code(&mut self, code: u8) {
        self.align();
        self.out.extend_from_slice(&[0, 0, 1, code]);
    }

    /// Bits written so far.
    #[cfg(test)]
    pub fn bits(&self) -> u64 {
        self.out.len() as u64 * 8 + u64::from(self.n)
    }

    /// The bytes, padded to a byte boundary.
    pub fn finish(mut self) -> Vec<u8> {
        self.align();
        self.out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mpeg2::bits::Bits;

    #[test]
    fn round_trips_through_the_reader() {
        let mut w = BitWriter::new();
        w.put(0b101, 3);
        w.put(0x1234_5678, 32);
        w.bit(true);
        w.put(0x3FF, 10);
        assert_eq!(w.bits(), 46);
        let d = w.finish();
        assert_eq!(d.len(), 6);
        let mut b = Bits::new(&d);
        assert_eq!(b.read(3), 0b101);
        assert_eq!(b.read(32), 0x1234_5678);
        assert!(b.bit());
        assert_eq!(b.read(10), 0x3FF);
        assert_eq!(b.read(2), 0);
    }

    #[test]
    fn start_codes_are_byte_aligned() {
        let mut w = BitWriter::new();
        w.put(1, 1);
        w.start_code(0xB3);
        assert_eq!(w.finish(), vec![0x80, 0, 0, 1, 0xB3]);
    }
}
