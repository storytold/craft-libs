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

/// The bytes `s` encodes; `None` when it is not valid padded base64. Padding is allowed only at
/// the end.
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
    fn misplaced_padding_and_foreign_characters_are_refused() {
        for s in ["====", "A===", "AA=A", "=AAA", "QQ==QUJD", "QUI=QUJD", "QUJ", "QU*D", "QUJDé"] {
            assert_eq!(decode(s), None, "{s}");
        }
    }
}
