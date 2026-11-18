//! Standard base64 (RFC 4648 §4, with `=` padding), as VectorCraft writes members in the JSON.
//! The decoder skips ASCII whitespace.

const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

/// Sextet value of each byte; 64 for bytes outside the alphabet.
const VALUE: [u8; 256] = {
    let mut t = [64u8; 256];
    let mut i = 0;
    while i < 64 {
        t[ALPHABET[i] as usize] = i as u8;
        i += 1;
    }
    t
};

fn symbol(sextet: u32) -> char {
    ALPHABET.get((sextet & 63) as usize).map_or('A', |&b| char::from(b))
}

/// `data` as base64, padded with `=`.
pub fn encode(data: &[u8]) -> String {
    let mut out = String::with_capacity(data.len().div_ceil(3) * 4);
    for c in data.chunks(3) {
        let byte = |i: usize| u32::from(c.get(i).copied().unwrap_or(0));
        let n = byte(0) << 16 | byte(1) << 8 | byte(2);
        out.push(symbol(n >> 18));
        out.push(symbol(n >> 12));
        out.push(if c.len() > 1 { symbol(n >> 6) } else { '=' });
        out.push(if c.len() > 2 { symbol(n) } else { '=' });
    }
    out
}

/// The number of bytes `s` decodes to when it is valid: 3 for every 4 characters other than
/// whitespace, less one for each trailing `=` (at most 2).
pub fn decoded_len(s: &str) -> u64 {
    let clean = s.bytes().filter(|b| !b.is_ascii_whitespace());
    let n = clean.clone().count() as u64;
    let pad = clean.rev().take(2).take_while(|&b| b == b'=').count() as u64;
    (n / 4 * 3).saturating_sub(pad)
}

pub(crate) const CHECK_CHUNK: usize = 64 * 1024;

/// [`decoded_len`] with a fallible checkpoint before each bounded portion of input.
pub fn decoded_len_with_check<E>(s: &str, mut check: impl FnMut() -> Result<(), E>) -> Result<u64, E> {
    let (n, pad) = measured_with_check(s, &mut check)?;
    Ok((n / 4 * 3).saturating_sub(pad))
}

fn measured_with_check<E>(s: &str, mut check: impl FnMut() -> Result<(), E>) -> Result<(u64, u64), E> {
    check()?;
    let (mut n, mut pad) = (0u64, 0u64);
    for chunk in s.as_bytes().chunks(CHECK_CHUNK) {
        check()?;
        for &b in chunk {
            if !b.is_ascii_whitespace() {
                n += 1;
                pad = if b == b'=' { (pad + 1).min(2) } else { 0 };
            }
        }
    }
    Ok((n, pad))
}

/// [`decode`] with fallible checkpoints during length calculation and decoding. Whitespace,
/// padding and non-canonical groups follow [`decode`]; `Ok(None)` means invalid base64.
pub fn decode_with_check<E>(s: &str, mut check: impl FnMut() -> Result<(), E>) -> Result<Option<Vec<u8>>, E> {
    let (n, pad) = measured_with_check(s, &mut check)?;
    if !n.is_multiple_of(4) {
        return Ok(None);
    }
    let Ok(len) = usize::try_from((n / 4 * 3).saturating_sub(pad)) else { return Ok(None) };
    decode_chunks(s, len, &mut check)
}

fn decode_chunks<E>(s: &str, capacity: usize, check: &mut impl FnMut() -> Result<(), E>) -> Result<Option<Vec<u8>>, E> {
    let mut out = Vec::with_capacity(capacity);
    let mut clean = Vec::with_capacity(CHECK_CHUNK);
    let mut ended = false;
    for chunk in s.as_bytes().chunks(CHECK_CHUNK) {
        check()?;
        for &b in chunk {
            if b.is_ascii_whitespace() {
                continue;
            }
            if ended {
                return Ok(None);
            }
            clean.push(b);
            if clean.len() == CHECK_CHUNK {
                let Ok(text) = std::str::from_utf8(&clean) else { return Ok(None) };
                let Some(decoded) = decode(text) else { return Ok(None) };
                out.extend_from_slice(&decoded);
                ended = clean.contains(&b'=');
                clean.clear();
            }
        }
    }
    let Ok(text) = std::str::from_utf8(&clean) else { return Ok(None) };
    let Some(decoded) = decode(text) else { return Ok(None) };
    out.extend_from_slice(&decoded);
    Ok(Some(out))
}

/// The bytes `s` encodes; `None` when it is not valid padded base64. Padding is allowed only at
/// the end. The unused bits before the padding are not checked, so a non-canonical group decodes
/// like its canonical form (`Zh==` gives the bytes of `Zg==`, and `Zm9=` those of `Zm8=`), as
/// VectorCraft's own decoder reads them.
pub fn decode(s: &str) -> Option<Vec<u8>> {
    let clean = s.bytes().filter(|b| !b.is_ascii_whitespace());
    let n = clean.clone().count();
    if !n.is_multiple_of(4) {
        return None;
    }
    let mut out = Vec::with_capacity(n / 4 * 3);
    let (mut group, mut len, mut pad, mut ended) = (0u32, 0usize, 0usize, false);
    for b in clean {
        if ended {
            return None;
        }
        group <<= 6;
        if b == b'=' {
            pad += 1;
        } else {
            let v = *VALUE.get(usize::from(b))?;
            if v == 64 || pad > 0 {
                return None;
            }
            group |= u32::from(v);
        }
        len += 1;
        if len == 4 {
            if pad > 2 {
                return None;
            }
            let [_, a, b, c] = group.to_be_bytes();
            out.push(a);
            if pad < 2 {
                out.push(b);
            }
            if pad < 1 {
                out.push(c);
            }
            ended = pad > 0;
            (group, len) = (0, 0);
        }
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_rfc_vectors_encode_and_decode() {
        for (dec, enc) in
            [("", ""), ("f", "Zg=="), ("fo", "Zm8="), ("foo", "Zm9v"), ("foob", "Zm9vYg=="), ("fooba", "Zm9vYmE="), ("foobar", "Zm9vYmFy")]
        {
            assert_eq!(encode(dec.as_bytes()), enc);
            assert_eq!(decode(enc).as_deref(), Some(dec.as_bytes()), "{enc}");
            assert_eq!(decoded_len(enc), dec.len() as u64, "{enc}");
            assert_eq!(decode_with_check(enc, || Ok::<_, ()>(())).unwrap(), decode(enc));
            assert_eq!(decoded_len_with_check(enc, || Ok::<_, ()>(())).unwrap(), decoded_len(enc));
        }
        assert_eq!(decode("Zm9v\nYmFy").as_deref(), Some(&b"foobar"[..]));
    }

    #[test]
    fn every_byte_value_round_trips() {
        let all: Vec<u8> = (0..=255).collect();
        for n in 0..all.len() {
            assert_eq!(decode(&encode(&all[..n])).as_deref(), Some(&all[..n]));
        }
    }

    #[test]
    fn nonzero_bits_before_the_padding_are_ignored() {
        for (loose, canonical) in [("Zh==", "Zg=="), ("Zm9=", "Zm8=")] {
            assert_eq!(decode(loose), decode(canonical), "{loose}");
            assert_eq!(decoded_len(loose), decoded_len(canonical), "{loose}");
        }
        assert_eq!(decode("Zh==").as_deref(), Some(&b"f"[..]));
        assert_eq!(decode("Zm9=").as_deref(), Some(&b"fo"[..]));
    }

    #[test]
    fn misplaced_padding_and_foreign_characters_are_refused() {
        for s in ["====", "A===", "AA=A", "=AAA", "QQ==QUJD", "QUI=QUJD", "QUJ", "QU*D", "QUJDé"] {
            assert_eq!(decode(s), None, "{s}");
            assert_eq!(decode_with_check(s, || Ok::<_, ()>(())).unwrap(), None, "{s}");
        }
    }

    #[test]
    fn checked_blocks_preserve_padding_and_whitespace() {
        for pad in ["QUJD", "Zh==", "Zm9=", "A===", "QUJ"] {
            let data = format!("{}\n{pad}\t", "QUJD".repeat(CHECK_CHUNK / 4 - 1));
            for data in [data.clone(), format!("{data}QUJD")] {
                assert_eq!(decode_with_check(&data, || Ok::<_, ()>(())).unwrap(), decode(&data));
                assert_eq!(decoded_len_with_check(&data, || Ok::<_, ()>(())).unwrap(), decoded_len(&data));
            }
        }
    }

    #[test]
    fn decoding_can_stop_after_its_first_block() {
        let data = "QUJD".repeat(CHECK_CHUNK);
        let mut polls = 0;
        let mut check = || {
            polls += 1;
            if polls == 2 { Err("stopped") } else { Ok(()) }
        };
        assert_eq!(decode_chunks(&data, 0, &mut check), Err("stopped"));
        assert_eq!(polls, 2);
    }

    #[test]
    fn checked_image_reads_keep_bounds_and_blob_ranges() {
        let mut image = crate::Image::inline("application/pdf", b"abcdef");
        for max in [0, 5, 6, u64::MAX] {
            assert_eq!(image.bytes_at_most_with_check(b"", max, || Ok::<_, ()>(())).unwrap(), image.bytes_at_most(b"", max));
        }
        image.blob = Some([2, 3]);
        for tail in [&b"123456"[..], b"12"] {
            for max in [0, 2, 3, u64::MAX] {
                assert_eq!(image.bytes_at_most_with_check(tail, max, || Ok::<_, ()>(())).unwrap(), image.bytes_at_most(tail, max));
            }
        }
        assert_eq!(image.bytes_at_most_with_check(b"123456", 3, || Err(17)), Err(17));
    }
}
