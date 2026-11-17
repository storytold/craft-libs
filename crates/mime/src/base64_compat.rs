//! Compatibility decoding for legacy transports that accepted mixed Base64 alphabets and padding.
//!
//! This is deliberately permissive and must not be used for signatures, credentials or other
//! contexts requiring canonical Base64. Use [`crate::base64`] for strict RFC 4648 decoding.
use crate::base64::DecodeError;

/// Decode standard or URL-safe Base64, accepting either alphabet even in the same input.
/// Spaces, CR, LF and `=` are ignored wherever they occur; an incomplete final sextet is
/// discarded. Other bytes are errors. Output allocation is bounded by the input length.
pub fn decode(input: impl AsRef<[u8]>) -> Result<Vec<u8>, DecodeError> {
    let input = input.as_ref();
    let mut out = Vec::new();
    let capacity = (input.len() / 4) * 3 + (input.len() % 4) * 3 / 4;
    out.try_reserve(capacity).map_err(|_| DecodeError)?;
    let (mut acc, mut bits) = (0u32, 0u32);
    for c in input {
        let v = match c {
            b'A'..=b'Z' => c - b'A',
            b'a'..=b'z' => c - b'a' + 26,
            b'0'..=b'9' => c - b'0' + 52,
            b'+' | b'-' => 62,
            b'/' | b'_' => 63,
            b'=' | b'\n' | b'\r' | b' ' => continue,
            _ => return Err(DecodeError),
        };
        acc = (acc << 6) | u32::from(v);
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((acc >> bits) as u8);
            acc &= (1 << bits) - 1;
        }
    }
    Ok(out)
}
#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use super::*;
    #[test]
    fn accepts_mixed_alphabets_and_ignores_padding_anywhere() {
        assert_eq!(decode("+_8=").unwrap(), [251, 255]);
        assert_eq!(decode("Z=m 9\nv\r").unwrap(), b"foo");
        assert_eq!(decode("A").unwrap(), b"");
        assert_eq!(decode("AB").unwrap(), [0]);
        assert!(decode("Zm9v\t").is_err());
        assert!(decode("Zm9v!").is_err());
        assert!(crate::base64::STANDARD.decode("Z=m9v").is_err());
    }
    // Differential against CADCraft's pre-cutover decoder, including malformed legacy inputs.
    fn legacy(s: &str) -> Option<Vec<u8>> {
        let mut out = Vec::new();
        let (mut acc, mut bits) = (0u32, 0u32);
        for c in s.bytes() {
            let value = match c {
                b'A'..=b'Z' => c - b'A',
                b'a'..=b'z' => c - b'a' + 26,
                b'0'..=b'9' => c - b'0' + 52,
                b'+' | b'-' => 62,
                b'/' | b'_' => 63,
                b'=' | b'\n' | b'\r' | b' ' => continue,
                _ => return None,
            };
            acc = (acc << 6) | u32::from(value);
            bits += 6;
            if bits >= 8 {
                bits -= 8;
                out.push((acc >> bits) as u8);
            }
        }
        Some(out)
    }
    #[test]
    fn matches_the_legacy_decoder_on_seeded_mixed_inputs() {
        let alphabet = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/-_= \r\n!\t";
        let mut seed = 0x1234_5678u32;
        for n in 0..50_000 {
            let mut text = String::new();
            for _ in 0..n % 45 {
                seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
                text.push(char::from(alphabet[(seed as usize) % alphabet.len()]));
            }
            assert_eq!(decode(&text).ok(), legacy(&text), "{text:?}");
        }
    }

    #[test]
    fn round_trip_rfc_vectors_with_the_shared_encoder() {
        for data in [&b""[..], b"f", b"fo", b"foo", b"foobar", &[0, 255, 127]] {
            assert_eq!(decode(crate::base64::STANDARD.encode(data)).unwrap(), data);
        }
    }
}
