//! MSB-first bit writer for the OBU headers (spec 4.10 `f(n)` descriptors) and `leb128()`.

#[derive(Default)]
pub(crate) struct BitWriter {
    pub buf: Vec<u8>,
    nbits: usize,
}

impl BitWriter {
    pub fn new() -> Self {
        BitWriter::default()
    }

    /// `f(n)`: writes the low `n` bits of `v`, most significant first.
    pub fn f(&mut self, n: u32, v: u32) {
        for i in (0..n).rev() {
            self.bit((v >> i) & 1 != 0);
        }
    }

    pub fn bit(&mut self, b: bool) {
        if self.nbits.is_multiple_of(8) {
            self.buf.push(0);
        }
        if b {
            let last = self.buf.len() - 1;
            self.buf[last] |= 0x80 >> (self.nbits % 8);
        }
        self.nbits += 1;
    }

    /// `byte_alignment()`: zero bits up to the next byte boundary.
    pub fn byte_align(&mut self) {
        while !self.nbits.is_multiple_of(8) {
            self.bit(false);
        }
    }

    /// `trailing_bits()`: a one bit followed by zero bits up to the byte boundary.
    pub fn trailing_bits(&mut self) {
        self.bit(true);
        self.byte_align();
    }

    pub fn into_bytes(self) -> Vec<u8> {
        self.buf
    }
}

/// `leb128()` encoding of an OBU size.
pub(crate) fn leb128(mut v: usize, out: &mut Vec<u8>) {
    loop {
        let mut b = (v & 0x7f) as u8;
        v >>= 7;
        if v != 0 {
            b |= 0x80;
        }
        out.push(b);
        if v == 0 {
            break;
        }
    }
}

/// An OBU with `obu_has_size_field` = 1 and no extension header.
pub(crate) fn obu(obu_type: u8, payload: &[u8], out: &mut Vec<u8>) {
    out.push((obu_type << 3) | 0x02);
    leb128(payload.len(), out);
    out.extend_from_slice(payload);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bits_and_leb128() {
        let mut w = BitWriter::new();
        w.f(3, 0b101);
        w.f(6, 0b110011);
        w.trailing_bits();
        assert_eq!(w.into_bytes(), vec![0b1011_1001, 0b1100_0000]);
        let mut v = Vec::new();
        leb128(300, &mut v);
        assert_eq!(v, vec![0xAC, 0x02]);
    }
}
