//! Standard base64 (RFC 4648, `+/` alphabet, `=` padding), used to keep opaque plugin state
//! blobs in the JSON session file. Decoding treats its input as hostile: it rejects bad
//! characters and bad padding, ignores ASCII whitespace, and caps the output size.

/// Largest blob [`decode`] will produce (64 MiB): bigger session fields are rejected.
pub const MAX_DECODED_BYTES: usize = 64 << 20;

const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

/// Encodes bytes as padded standard base64.
pub fn encode(data: &[u8]) -> String {
    let mut out = String::with_capacity(data.len().div_ceil(3).saturating_mul(4));
    for chunk in data.chunks(3) {
        let b = [chunk.first().copied().unwrap_or(0), chunk.get(1).copied().unwrap_or(0), chunk.get(2).copied().unwrap_or(0)];
        let n = (u32::from(b[0]) << 16) | (u32::from(b[1]) << 8) | u32::from(b[2]);
        let sym = |shift: u32| char::from(ALPHABET.get(((n >> shift) & 63) as usize).copied().unwrap_or(b'A'));
        out.push(sym(18));
        out.push(sym(12));
        out.push(if chunk.len() > 1 { sym(6) } else { '=' });
        out.push(if chunk.len() > 2 { sym(0) } else { '=' });
    }
    out
}

fn value(c: u8) -> Option<u32> {
    let v = match c {
        b'A'..=b'Z' => c - b'A',
        b'a'..=b'z' => c - b'a' + 26,
        b'0'..=b'9' => c - b'0' + 52,
        b'+' => 62,
        b'/' => 63,
        _ => return None,
    };
    Some(u32::from(v))
}

/// Decodes padded (or unpadded) standard base64. `None` for malformed input or a result larger
/// than [`MAX_DECODED_BYTES`].
pub fn decode(text: &str) -> Option<Vec<u8>> {
    decode_capped(text, MAX_DECODED_BYTES)
}

/// [`decode`] with the output capped at `max` bytes.
fn decode_capped(text: &str, max: usize) -> Option<Vec<u8>> {
    if text.bytes().filter(|c| !c.is_ascii_whitespace()).count() > max.div_ceil(3).saturating_mul(4) {
        return None;
    }
    let syms: Vec<u8> = text.bytes().filter(|c| !c.is_ascii_whitespace()).collect();
    let body = syms.strip_suffix(b"==").or_else(|| syms.strip_suffix(b"=")).unwrap_or(&syms);
    if body.contains(&b'=') || (syms.len() != body.len() && !syms.len().is_multiple_of(4)) || body.len() % 4 == 1 {
        return None;
    }
    let decoded_len = body.len() / 4 * 3 + body.len() % 4 * 3 / 4;
    if decoded_len > max {
        return None;
    }
    let mut out = Vec::with_capacity(decoded_len);
    for chunk in body.chunks(4) {
        let mut n = 0u32;
        for (i, &c) in chunk.iter().enumerate() {
            n |= value(c)? << (18 - 6 * i as u32);
        }
        let bytes = [(n >> 16) as u8, (n >> 8) as u8, n as u8];
        out.extend_from_slice(bytes.get(..chunk.len() - 1)?);
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rfc4648_vectors_round_trip() {
        for (plain, enc) in
            [("", ""), ("f", "Zg=="), ("fo", "Zm8="), ("foo", "Zm9v"), ("foob", "Zm9vYg=="), ("fooba", "Zm9vYmE="), ("foobar", "Zm9vYmFy")]
        {
            assert_eq!(encode(plain.as_bytes()), enc);
            assert_eq!(decode(enc).unwrap(), plain.as_bytes());
        }
        let all: Vec<u8> = (0..=255).collect();
        assert_eq!(decode(&encode(&all)).unwrap(), all);
        assert_eq!(decode("Zm9v\nYmE=").unwrap(), b"fooba", "whitespace is ignored");
        assert_eq!(decode("Zm9vYmE").unwrap(), b"fooba", "padding is optional");
    }

    #[test]
    fn hostile_input_is_rejected() {
        for bad in ["Z", "Zm9=v", "Zm9v!", "====", "Zg=", "é", "Zg===", "Z==="] {
            assert!(decode(bad).is_none(), "{bad}");
        }
    }

    #[test]
    fn size_cap_counts_the_partial_quartet() {
        fn encoded_zero_bytes(len: usize) -> String {
            let groups = len / 3;
            let mut encoded = "A".repeat(groups * 4);
            match len % 3 {
                1 => encoded.push_str("AA=="),
                2 => encoded.push_str("AAA="),
                _ => {}
            }
            encoded
        }

        // The boundary at a cap of every remainder mod 3; nightly also at the real 64 MiB cap.
        let full = craft_testkit::hostile::divisor() == 1;
        let caps = [97, 98, 99].into_iter().chain(full.then_some(MAX_DECODED_BYTES));
        for max in caps {
            let exact = encoded_zero_bytes(max);
            assert_eq!(decode_capped(&exact, max).map(|bytes| bytes.len()), Some(max), "padded boundary at {max}");
            let too_large = encoded_zero_bytes(max + 1);
            assert!(decode_capped(&too_large, max).is_none(), "padded over boundary at {max}");
            assert_eq!(decode_capped(exact.trim_end_matches('='), max).map(|bytes| bytes.len()), Some(max), "unpadded boundary at {max}");
            let spaced = format!("{}\n {}", &too_large[..8], &too_large[8..]);
            assert!(decode_capped(&spaced, max).is_none(), "whitespace over boundary at {max}");
        }
    }
}
