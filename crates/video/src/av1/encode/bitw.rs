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
