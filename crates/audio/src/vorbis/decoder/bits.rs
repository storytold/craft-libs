//! The Vorbis bit reader (Vorbis I §2): bits are taken least significant first from each byte.

/// Reading past the end of the packet ("end-of-packet", §2.1.8).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct Eop;

pub(super) struct BitReader<'a> {
    data: &'a [u8],
    /// The position of the next bit.
    pos: usize,
}

impl<'a> BitReader<'a> {
    pub(super) fn new(data: &'a [u8]) -> Self {
        Self { data, pos: 0 }
    }

    /// The bits not yet read.
    pub(super) fn remaining(&self) -> usize {
        (self.data.len() * 8).saturating_sub(self.pos)
    }

    /// Read `n` (at most 32) bits as an unsigned integer. A read that does not fit takes nothing.
    pub(super) fn read(&mut self, n: u32) -> Result<u32, Eop> {
        if n == 0 {
            return Ok(0);
        }
        if (n as usize) > self.remaining() {
            self.pos = self.data.len() * 8;
            return Err(Eop);
        }
        let mut value = 0u64;
        let mut got = 0u32;
        while got < n {
            let byte = u64::from(self.data[self.pos / 8]);
            let shift = (self.pos % 8) as u32;
            let take = (8 - shift).min(n - got);
            value |= ((byte >> shift) & ((1 << take) - 1)) << got;
            got += take;
            self.pos += take as usize;
        }
        Ok(value as u32)
    }

    pub(super) fn bit(&mut self) -> Result<bool, Eop> {
        Ok(self.read(1)? == 1)
    }
}

/// `ilog(x)` (§9.2.1): the position of the highest set bit, 0 for values below 1.
pub(super) fn ilog(x: i64) -> u32 {
    if x <= 0 { 0 } else { 64 - (x as u64).leading_zeros() }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_least_significant_bit_first() {
        // Fields come out of each byte from its least significant bit.
        let mut r = BitReader::new(&[0xFC, 0x08]);
        assert_eq!(r.read(4), Ok(0xC));
        assert_eq!(r.read(4), Ok(0xF));
        assert_eq!(r.read(1), Ok(0));
        assert_eq!(r.read(3), Ok(4));
        assert_eq!(r.read(4), Ok(0));
        assert_eq!(r.read(1), Err(Eop));
    }

    #[test]
    fn wide_reads_span_bytes_and_zero_bits_are_free() {
        let mut r = BitReader::new(&[0x01, 0x02, 0x03, 0x04, 0x05]);
        assert_eq!(r.read(0), Ok(0));
        assert_eq!(r.read(32), Ok(0x0403_0201));
        assert_eq!(r.read(8), Ok(5));
        assert_eq!(r.read(0), Ok(0));
        assert_eq!(r.read(1), Err(Eop));
        assert_eq!(r.remaining(), 0);
    }

    #[test]
    fn a_failed_read_consumes_the_rest() {
        let mut r = BitReader::new(&[0xFF]);
        assert_eq!(r.read(3), Ok(7));
        assert_eq!(r.read(6), Err(Eop));
        assert_eq!(r.read(1), Err(Eop));
    }

    #[test]
    fn ilog_matches_the_specification_examples() {
        for (x, want) in [(0, 0), (1, 1), (2, 2), (3, 2), (4, 3), (7, 3), (-5, 0), (255, 8), (256, 9)] {
            assert_eq!(ilog(x), want, "ilog({x})");
        }
    }
}
