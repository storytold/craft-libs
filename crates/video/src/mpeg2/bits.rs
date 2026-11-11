//! Big-endian bit reader over one start-code unit. Reads past the end return zero bits; callers
//! check [`Bits::overrun`] to detect truncated data.

#[derive(Clone)]
pub(crate) struct Bits<'a> {
    data: &'a [u8],
    pos: usize,
}

impl<'a> Bits<'a> {
    pub fn new(data: &'a [u8]) -> Self {
        Self { data, pos: 0 }
    }

    /// The next 32 bits, MSB first (zeros past the end).
    #[inline(always)]
    pub fn peek32(&self) -> u32 {
        let byte = self.pos >> 3;
        let w = if let Some(&b) = self.data.get(byte..).and_then(|d| d.first_chunk::<8>()) {
            u64::from_be_bytes(b)
        } else {
            let mut b = [0u8; 8];
            if byte < self.data.len() {
                let n = self.data.len() - byte;
                b[..n].copy_from_slice(&self.data[byte..]);
            }
            u64::from_be_bytes(b)
        };
        ((w << (self.pos & 7)) >> 32) as u32
    }

    /// The next `n` (1..=32) bits without consuming them.
    #[inline(always)]
    pub fn peek(&self, n: u32) -> u32 {
        debug_assert!((1..=32).contains(&n));
        ((self.peek32() as u64) >> (32 - n)) as u32
    }

    #[inline(always)]
    pub fn skip(&mut self, n: u32) {
        self.pos += n as usize;
    }

    #[inline(always)]
    pub fn read(&mut self, n: u32) -> u32 {
        if n == 0 {
            return 0;
        }
        let v = self.peek(n);
        self.pos += n as usize;
        v
    }

    #[inline(always)]
    pub fn bit(&mut self) -> bool {
        self.read(1) == 1
    }

    /// A `n`-bit two's complement value.
    #[inline(always)]
    pub fn read_signed(&mut self, n: u32) -> i32 {
        let v = self.read(n) as i32;
        (v << (32 - n)) >> (32 - n)
    }

    pub fn bits_left(&self) -> isize {
        (self.data.len() * 8) as isize - self.pos as isize
    }

    /// More than a few bytes have been read past the end: the data is truncated or corrupt.
    pub fn overrun(&self) -> bool {
        self.bits_left() < 0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_across_bytes_and_past_the_end() {
        let d = [0b1010_1100, 0xFF, 0x01];
        let mut b = Bits::new(&d);
        assert_eq!(b.read(3), 0b101);
        assert_eq!(b.read(7), 0b011_0011);
        assert_eq!(b.peek(4), 0xF);
        assert_eq!(b.read_signed(4), -1);
        assert_eq!(b.read(10), 0b11_0000_0001);
        assert!(!b.overrun());
        assert_eq!(b.read(8), 0);
        assert!(b.overrun());
    }
}
