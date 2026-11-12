//! Standard base64 (RFC 4648 §4) and `data:` URIs, for embedded image assets.

const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

pub fn encode(data: &[u8]) -> String {
    let mut out = String::with_capacity(data.len().div_ceil(3) * 4);
    for chunk in data.chunks(3) {
        let b = [chunk[0], *chunk.get(1).unwrap_or(&0), *chunk.get(2).unwrap_or(&0)];
        let n = (b[0] as u32) << 16 | (b[1] as u32) << 8 | b[2] as u32;
        for i in 0..4 {
            if i <= chunk.len() {
                out.push(ALPHABET[(n >> (18 - 6 * i) & 63) as usize] as char);
            } else {
                out.push('=');
            }
        }
    }
    out
}

pub fn decode(s: &str) -> Option<Vec<u8>> {
    let val = |c: u8| -> Option<u32> {
        Some(match c {
            b'A'..=b'Z' => c - b'A',
            b'a'..=b'z' => c - b'a' + 26,
            b'0'..=b'9' => c - b'0' + 52,
            b'+' | b'-' => 62,
            b'/' | b'_' => 63,
            _ => return None,
        } as u32)
    };
    let clean: Vec<u8> = s.bytes().filter(|c| !c.is_ascii_whitespace() && *c != b'=').collect();
    let mut out = Vec::with_capacity(clean.len() * 3 / 4);
    for chunk in clean.chunks(4) {
        let mut n = 0u32;
        for (i, &c) in chunk.iter().enumerate() {
            n |= val(c)? << (18 - 6 * i);
        }
        let bytes = [(n >> 16) as u8, (n >> 8) as u8, n as u8];
        match chunk.len() {
            4 => out.extend_from_slice(&bytes),
            3 => out.extend_from_slice(&bytes[..2]),
            2 => out.push(bytes[0]),
            _ => return None,
        }
    }
    Some(out)
}

/// A `data:<mime>;base64,<payload>` URI of `bytes`.
pub fn to_data_uri(mime: &str, bytes: &[u8]) -> String {
    format!("data:{mime};base64,{}", encode(bytes))
}

/// Split a `data:<mime>;base64,<payload>` URI into (mime, bytes).
pub fn data_uri(s: &str) -> Option<(String, Vec<u8>)> {
    let rest = s.strip_prefix("data:")?;
    let (head, payload) = rest.split_once(',')?;
    let mime = head.strip_suffix(";base64")?;
    Some((mime.to_string(), decode(payload)?))
}

#[cfg(test)]
mod tests {
    #[test]
    fn roundtrip() {
        for s in ["", "f", "fo", "foo", "foob", "fooba", "foobar"] {
            let e = super::encode(s.as_bytes());
            assert_eq!(super::decode(&e).unwrap(), s.as_bytes());
        }
        assert_eq!(super::encode(b"foobar"), "Zm9vYmFy");
        assert_eq!(super::encode(b"fo"), "Zm8=");
    }
}
