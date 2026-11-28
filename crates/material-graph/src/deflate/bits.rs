//! LSB-first bit writer, the packing order DEFLATE uses.

pub(super) struct BitWriter {
    out: Vec<u8>,
    acc: u64,
    n: u32,
}

impl BitWriter {
    pub fn with_capacity(cap: usize) -> Self {
        BitWriter { out: Vec::with_capacity(cap), acc: 0, n: 0 }
    }

    /// Append the low `count` bits of `bits` (count <= 32).
    #[inline]
    pub fn put(&mut self, bits: u32, count: u32) {
        debug_assert!(count <= 32 && (count == 32 || bits >> count == 0));
        self.acc |= (bits as u64) << self.n;
        self.n += count;
        if self.n >= 32 {
            self.out.extend_from_slice(&(self.acc as u32).to_le_bytes());
            self.acc >>= 32;
            self.n -= 32;
        }
    }

    /// Pad with zero bits up to the next byte boundary.
    pub fn align(&mut self) {
        while self.n > 0 {
            self.out.push(self.acc as u8);
            self.acc >>= 8;
            self.n = self.n.saturating_sub(8);
        }
        self.acc = 0;
    }

    /// Append whole bytes; the writer must be byte-aligned.
    pub fn put_bytes(&mut self, bytes: &[u8]) {
        debug_assert_eq!(self.n, 0);
        self.out.extend_from_slice(bytes);
    }

    pub fn finish(mut self) -> Vec<u8> {
        self.align();
        self.out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn packs_lsb_first() {
        let mut w = BitWriter::with_capacity(0);
        w.put(1, 1);
        w.put(0b10, 2);
        w.put(0x1ff, 9);
        w.put(0xdead_beef, 32);
        w.align();
        w.put_bytes(&[7]);
        w.put(1, 3);
        let out = w.finish();
        let mut expect: u64 = 1 | (0b10 << 1) | (0x1ff << 3) | (0xdead_beef << 12);
        let mut bytes = Vec::new();
        for _ in 0..6 {
            bytes.push(expect as u8);
            expect >>= 8;
        }
        bytes.extend_from_slice(&[7, 1]);
        assert_eq!(out, bytes);
    }
}
