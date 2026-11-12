//! `leb128()` and OBU framing helpers (spec 4.10, 5.3).

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
    use craft_media::bitstream::BitWriter;

    #[test]
    fn bits_and_leb128() {
        let mut w = BitWriter::new();
        w.write_bits(0b101, 3);
        w.write_bits(0b110011, 6);
        w.rbsp_trailing();
        assert_eq!(w.finish(), vec![0b1011_1001, 0b1100_0000]);
        let mut v = Vec::new();
        leb128(300, &mut v);
        assert_eq!(v, vec![0xAC, 0x02]);
    }
}
