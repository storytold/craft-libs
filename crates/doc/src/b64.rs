//! Standard base64 (RFC 4648 §4, with `=` padding), as VectorCraft writes members in the JSON.
//! ASCII whitespace is skipped, as VectorCraft's reader skips it.

/// Sextet value of each byte; 64 for bytes outside the alphabet.
const VALUE: [u8; 256] = {
    let mut t = [64u8; 256];
    let alphabet = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut i = 0;
    while i < 64 {
        t[alphabet[i] as usize] = i as u8;
        i += 1;
    }
    t
};

/// The bytes `s` decodes to when it is valid: 3 for every 4 non-whitespace characters, less
/// one for each trailing `=` (at most 2).
pub(crate) fn decoded_len(s: &str) -> u64 {
    let clean = s.bytes().filter(|b| !b.is_ascii_whitespace());
    let n = clean.clone().count() as u64;
    let pad = clean.rev().take(2).take_while(|&b| b == b'=').count() as u64;
    (n / 4 * 3).saturating_sub(pad)
}

/// Decode `s`; `None` when it is not valid padded base64.
pub(crate) fn decode(s: &str) -> Option<Vec<u8>> {
    let clean = s.bytes().filter(|b| !b.is_ascii_whitespace());
    let n = clean.clone().count();
    if !n.is_multiple_of(4) {
        return None;
    }
    let mut out = Vec::with_capacity(n / 4 * 3);
    let (mut group, mut len, mut pad, mut ended) = (0u32, 0usize, 0usize, false);
    for b in clean {
        if ended {
            // Padding ends the data.
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
    fn decodes_the_rfc_vectors() {
        for (enc, dec) in
            [("", ""), ("Zg==", "f"), ("Zm8=", "fo"), ("Zm9v", "foo"), ("Zm9vYg==", "foob"), ("Zm9vYmE=", "fooba"), ("Zm9vYmFy", "foobar")]
        {
            assert_eq!(decode(enc).as_deref(), Some(dec.as_bytes()), "{enc}");
            assert_eq!(decoded_len(enc), dec.len() as u64, "{enc}");
        }
        assert_eq!(decode("Zm9v\nYmFy").as_deref(), Some(&b"foobar"[..]));
    }

    #[test]
    fn rejects_bad_input() {
        for s in ["====", "A===", "AA=A", "=AAA", "QQ==QUJD", "QUI=QUJD", "QUJ", "QU*D", "QUJDé"] {
            assert_eq!(decode(s), None, "{s}");
        }
    }
}
