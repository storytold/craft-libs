//! Percent-encoding and decoding (RFC 3986 §2.1).
//!
//! [`encode_component`] produces text that is safe in a path segment and in a query value: every octet
//! outside the unreserved set of RFC 3986 §2.3 is written as `%XX`. This is also the encoding RFC 6570
//! §3.2.1 asks of a simple string expansion, which is what JMAP's address templates use.
//!
//! [`decode`] reverses it and refuses to hand back anything that is not UTF-8; [`decode_bytes`] gives the raw
//! octets for callers that want them. A `+` is left as it is: form encoding is not part of RFC 3986.

use std::fmt;

/// Why percent-decoding failed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DecodeError {
    /// A `%` was not followed by two hexadecimal digits.
    MalformedEscape,
    /// The decoded octets are not UTF-8.
    InvalidUtf8,
}

impl fmt::Display for DecodeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MalformedEscape => f.write_str("a % sign is not followed by two hexadecimal digits; a literal % is written %25"),
            Self::InvalidUtf8 => f.write_str("the percent-encoded octets do not form UTF-8 text"),
        }
    }
}

impl std::error::Error for DecodeError {}

/// Whether `b` is in the unreserved set of RFC 3986 §2.3.
pub(crate) fn is_unreserved(b: u8) -> bool {
    b.is_ascii_alphanumeric() || matches!(b, b'-' | b'.' | b'_' | b'~')
}

/// Whether `b` is one of the sub-delimiters of RFC 3986 §2.2.
pub(crate) fn is_sub_delim(b: u8) -> bool {
    matches!(b, b'!' | b'$' | b'&' | b'\'' | b'(' | b')' | b'*' | b'+' | b',' | b';' | b'=')
}

const HEX_UPPER: &[u8; 16] = b"0123456789ABCDEF";

fn hex_char(nibble: u8) -> char {
    char::from(HEX_UPPER.get(usize::from(nibble & 0x0F)).copied().unwrap_or(b'0'))
}

/// The value of one hexadecimal digit.
pub(crate) fn hex_value(b: u8) -> Option<u8> {
    match b {
        b'0'..=b'9' => Some(b.saturating_sub(b'0')),
        b'a'..=b'f' => Some(b.saturating_sub(b'a').saturating_add(10)),
        b'A'..=b'F' => Some(b.saturating_sub(b'A').saturating_add(10)),
        _ => None,
    }
}

fn push_escaped(out: &mut String, b: u8) {
    out.push('%');
    out.push(hex_char(b >> 4));
    out.push(hex_char(b));
}

/// Percent-encodes `value` for a path segment or a query value.
///
/// Only the unreserved characters of RFC 3986 §2.3 (`A-Z a-z 0-9 - . _ ~`) are kept as they are; every other
/// octet of the UTF-8 form, `/`, `?`, `&`, `=`, `+`, `%` and `#` included, becomes `%XX` with upper-case hex.
pub fn encode_component(value: &str) -> String {
    let mut out = String::with_capacity(value.len().saturating_mul(3));
    for &b in value.as_bytes() {
        if is_unreserved(b) {
            out.push(char::from(b));
        } else {
            push_escaped(&mut out, b);
        }
    }
    out
}

/// Decodes every `%XX` triplet of `input` into the octet it stands for.
pub fn decode_bytes(input: &str) -> Result<Vec<u8>, DecodeError> {
    let mut out = Vec::with_capacity(input.len());
    let mut bytes = input.bytes();
    while let Some(b) = bytes.next() {
        if b == b'%' {
            let high = bytes.next().and_then(hex_value).ok_or(DecodeError::MalformedEscape)?;
            let low = bytes.next().and_then(hex_value).ok_or(DecodeError::MalformedEscape)?;
            out.push(high.saturating_mul(16).saturating_add(low));
        } else {
            out.push(b);
        }
    }
    Ok(out)
}

/// Decodes percent-encoding and returns the text, refusing a result that is not UTF-8.
///
/// Nothing is ever replaced by U+FFFD: a value that does not decode to UTF-8 is an error.
pub fn decode(input: &str) -> Result<String, DecodeError> {
    let bytes = decode_bytes(input)?;
    String::from_utf8(bytes).map_err(|_| DecodeError::InvalidUtf8)
}

/// Rewrites the escapes of an already character-checked component in their normal form (RFC 3986 §6.2.2.1 and
/// §6.2.2.2): hex digits in upper case, and an escaped unreserved character written as itself.
///
/// Returns `None` when a `%` is not followed by two hex digits.
pub(crate) fn normalise_escapes(input: &str) -> Option<String> {
    let mut out = String::with_capacity(input.len());
    let mut bytes = input.bytes();
    while let Some(b) = bytes.next() {
        if b == b'%' {
            let high = bytes.next().and_then(hex_value)?;
            let low = bytes.next().and_then(hex_value)?;
            let octet = high.saturating_mul(16).saturating_add(low);
            if is_unreserved(octet) {
                out.push(char::from(octet));
            } else {
                push_escaped(&mut out, octet);
            }
        } else {
            out.push(char::from(b));
        }
    }
    Some(out)
}

#[cfg(test)]
mod tests;
