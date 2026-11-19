//! Base64 (RFC 4648 sections 4 and 5), written from the RFC.
//!
//! Four ready-made [`Config`]s: [`STANDARD`] (the `+/` alphabet, padded with `=`),
//! [`STANDARD_NO_PAD`], [`URL_SAFE`] (the `-_` alphabet, padded) and [`URL_SAFE_NO_PAD`].
//!
//! Encoding is the RFC's. **Decoding is strict**: nothing but the config's alphabet (no white
//! space, no line breaks), padding exactly as the config says (a padded config requires it and
//! counts it; an unpadded one refuses `=`), a length that can be, and the unused bits of the last
//! symbol zero (so every value has one spelling). A caller that reads mail, where line breaks and
//! missing padding are everyday, cleans its input first, as it always has; the strictness is what
//! the signature, key and credential checks want. No input panics, and the output is never longer
//! than three quarters of the input.

use std::fmt;

/// Which alphabet and whether padded.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Config {
    url_safe: bool,
    padded: bool,
}

/// RFC 4648 section 4: `A-Za-z0-9+/`, padded with `=`.
pub const STANDARD: Config = Config { url_safe: false, padded: true };
/// Section 4 without padding.
pub const STANDARD_NO_PAD: Config = Config { url_safe: false, padded: false };
/// Section 5: `A-Za-z0-9-_`, padded with `=`.
pub const URL_SAFE: Config = Config { url_safe: true, padded: true };
/// Section 5 without padding (OAuth's PKCE and JSON Web Tokens).
pub const URL_SAFE_NO_PAD: Config = Config { url_safe: true, padded: false };

/// Why a text is not base64 of the config's kind. Says nothing of the text itself.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DecodeError;

impl fmt::Display for DecodeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("not valid base64")
    }
}

impl std::error::Error for DecodeError {}

impl Config {
    /// The symbol for the six bits `v` (only the low six are looked at).
    fn symbol(self, v: u32) -> char {
        let v = (v & 63) as u8;
        let c = match v {
            0..=25 => b'A'.saturating_add(v),
            26..=51 => b'a'.saturating_add(v.saturating_sub(26)),
            52..=61 => b'0'.saturating_add(v.saturating_sub(52)),
            62 if self.url_safe => b'-',
            62 => b'+',
            _ if self.url_safe => b'_',
            _ => b'/',
        };
        char::from(c)
    }

    /// The six bits a symbol stands for, if it is one.
    fn value(self, c: u8) -> Option<u32> {
        let v = match c {
            b'A'..=b'Z' => c.saturating_sub(b'A'),
            b'a'..=b'z' => c.saturating_sub(b'a').saturating_add(26),
            b'0'..=b'9' => c.saturating_sub(b'0').saturating_add(52),
            b'+' if !self.url_safe => 62,
            b'/' if !self.url_safe => 63,
            b'-' if self.url_safe => 62,
            b'_' if self.url_safe => 63,
            _ => return None,
        };
        Some(u32::from(v))
    }

    /// The text for `data`.
    pub fn encode(self, data: impl AsRef<[u8]>) -> String {
        let data = data.as_ref();
        let mut out = String::with_capacity(data.len().div_ceil(3).saturating_mul(4));
        let (groups, remainder) = data.as_chunks::<3>();
        for [a, b, c] in groups {
            let n = u32::from(*a) << 16 | u32::from(*b) << 8 | u32::from(*c);
            for shift in [18, 12, 6, 0] {
                out.push(self.symbol(n >> shift));
            }
        }
        match *remainder {
            [a] => {
                let n = u32::from(a) << 16;
                out.push(self.symbol(n >> 18));
                out.push(self.symbol(n >> 12));
                if self.padded {
                    out.push_str("==");
                }
            }
            [a, b] => {
                let n = u32::from(a) << 16 | u32::from(b) << 8;
                out.push(self.symbol(n >> 18));
                out.push(self.symbol(n >> 12));
                out.push(self.symbol(n >> 6));
                if self.padded {
                    out.push('=');
                }
            }
            _ => {}
        }
        out
    }

    /// The bytes of `text`, which must be exactly base64 of this config's kind (see the module).
    pub fn decode(self, text: impl AsRef<[u8]>) -> Result<Vec<u8>, DecodeError> {
        let text = text.as_ref();
        let (symbols, pad) = if self.padded {
            if text.len() % 4 != 0 {
                return Err(DecodeError);
            }
            let pad = text.iter().rev().take(2).take_while(|c| **c == b'=').count();
            (text.get(..text.len().saturating_sub(pad)).unwrap_or_default(), pad)
        } else {
            (text, 0)
        };
        // A last group of one symbol holds under eight bits; with padding, `=` fills it to four.
        let rest = symbols.len() % 4;
        if rest == 1 || (self.padded && pad != (4 - rest) % 4) {
            return Err(DecodeError);
        }
        let mut out = Vec::with_capacity(symbols.len().saturating_mul(3) / 4);
        let (mut acc, mut bits) = (0u32, 0u32);
        for c in symbols {
            acc = acc << 6 | self.value(*c).ok_or(DecodeError)?;
            bits = bits.saturating_add(6);
            if bits >= 8 {
                bits -= 8;
                out.push((acc >> bits) as u8);
                acc &= (1 << bits) - 1;
            }
        }
        // What is left over must be zero: one spelling for one value.
        if acc != 0 {
            return Err(DecodeError);
        }
        Ok(out)
    }
}

#[cfg(test)]
#[path = "base64_tests.rs"]
mod base64_tests;
