//! The `format` contribution protocol (`ABI.md` section 6).
//!
//! A `format` contribution reads and writes a file type the host app does not know. The host calls
//! `craft_invoke` with a JSON request and the file or the pixels in `blob`.
//!
//! | Op | Request JSON | `blob` | Response JSON |
//! |---|---|---|---|
//! | read | `{"op":"read","name":"a.xyz","ext":"xyz"}` | the file bytes | `{"doc":{"kind":"image","width":W,"height":H,"rgba8":"<base64>"}}` |
//! | write | `{"op":"write","name":"a.xyz","ext":"xyz","doc":{"kind":"image","width":W,"height":H}}` | `W*H*4` bytes, RGBA8 | `{"bytes":"<base64>"}` |
//!
//! Pixels are 8 bit RGBA, straight alpha, rows tightly packed, top row first. A response is JSON, so
//! bytes it returns are base64 (RFC 4648, with padding). The request side has a blob and needs no
//! encoding. A plugin that cannot do an op answers `{"error": "..."}`.
//!
//! Host and SDK share this module, so the two sides cannot drift.

use std::fmt;

use serde_json::{Value, json};

/// The op names of a request.
pub const OP_READ: &str = "read";
pub const OP_WRITE: &str = "write";

/// A decoded image document. Straight alpha RGBA8, tightly packed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ImageDoc {
    pub width: u32,
    pub height: u32,
    pub rgba8: Vec<u8>,
}

/// Why a document or a response did not fit the protocol.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FormatDocError {
    /// Width or height is 0.
    Empty,
    /// `width * height * 4` does not equal the byte count (or overflows).
    SizeMismatch { width: u32, height: u32, bytes: usize },
    /// The JSON lacks a field or has the wrong type.
    Field(&'static str),
    /// `rgba8` or `bytes` is not valid base64.
    Base64(String),
}

impl fmt::Display for FormatDocError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            FormatDocError::Empty => f.write_str("the image has no pixels"),
            FormatDocError::SizeMismatch { width, height, bytes } => {
                write!(f, "{width}x{height} RGBA8 needs {} bytes, got {bytes}", u64::from(*width) * u64::from(*height) * 4)
            }
            FormatDocError::Field(name) => write!(f, "missing or wrong type: {name}"),
            FormatDocError::Base64(m) => write!(f, "bad base64: {m}"),
        }
    }
}

impl std::error::Error for FormatDocError {}

impl ImageDoc {
    /// Check that `rgba8` holds exactly `width * height` pixels.
    pub fn new(width: u32, height: u32, rgba8: Vec<u8>) -> Result<ImageDoc, FormatDocError> {
        if width == 0 || height == 0 {
            return Err(FormatDocError::Empty);
        }
        let want = (width as usize).checked_mul(height as usize).and_then(|n| n.checked_mul(4));
        if want != Some(rgba8.len()) {
            return Err(FormatDocError::SizeMismatch { width, height, bytes: rgba8.len() });
        }
        Ok(ImageDoc { width, height, rgba8 })
    }

    /// The `doc` object of a response, pixels as base64.
    pub fn to_json(&self) -> Value {
        json!({ "kind": "image", "width": self.width, "height": self.height, "rgba8": base64::encode(&self.rgba8) })
    }

    /// Parse the `doc` object of a response.
    pub fn from_json(doc: &Value) -> Result<ImageDoc, FormatDocError> {
        if doc.get("kind").and_then(Value::as_str) != Some("image") {
            return Err(FormatDocError::Field("doc.kind"));
        }
        let dim = |name: &'static str| doc.get(name).and_then(Value::as_u64).and_then(|v| u32::try_from(v).ok()).ok_or(FormatDocError::Field(name));
        let (width, height) = (dim("width")?, dim("height")?);
        let text = doc.get("rgba8").and_then(Value::as_str).ok_or(FormatDocError::Field("doc.rgba8"))?;
        ImageDoc::new(width, height, base64::decode(text)?)
    }
}

/// The request for `op: read`. The file bytes go in the blob.
pub fn read_request(name: &str, ext: &str) -> Value {
    json!({ "op": OP_READ, "name": name, "ext": ext })
}

/// The request for `op: write`. The pixels go in the blob, see [`ImageDoc::rgba8`].
pub fn write_request(name: &str, ext: &str, width: u32, height: u32) -> Value {
    json!({ "op": OP_WRITE, "name": name, "ext": ext, "doc": { "kind": "image", "width": width, "height": height } })
}

/// `{"doc": ...}`, the answer to a read.
pub fn read_response(doc: &ImageDoc) -> Value {
    json!({ "doc": doc.to_json() })
}

/// `{"bytes": "<base64>"}`, the answer to a write.
pub fn write_response(bytes: &[u8]) -> Value {
    json!({ "bytes": base64::encode(bytes) })
}

/// Parse the answer to a read.
pub fn parse_read_response(value: &Value) -> Result<ImageDoc, FormatDocError> {
    ImageDoc::from_json(value.get("doc").ok_or(FormatDocError::Field("doc"))?)
}

/// Parse the answer to a write.
pub fn parse_write_response(value: &Value) -> Result<Vec<u8>, FormatDocError> {
    base64::decode(value.get("bytes").and_then(Value::as_str).ok_or(FormatDocError::Field("bytes"))?)
}

/// The lower case extension of a file name, without the dot. `None` for no extension.
pub fn extension_of(name: &str) -> Option<String> {
    let file = name.rsplit(['/', '\\']).next().unwrap_or(name);
    let (stem, ext) = file.rsplit_once('.')?;
    (!stem.is_empty() && !ext.is_empty()).then(|| ext.to_ascii_lowercase())
}

/// Base64 (RFC 4648 section 4) with padding.
pub mod base64 {
    use super::FormatDocError;

    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

    pub fn encode(bytes: &[u8]) -> String {
        let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
        for chunk in bytes.chunks(3) {
            let n = chunk.iter().enumerate().fold(0u32, |acc, (i, b)| acc | u32::from(*b) << (16 - 8 * i));
            for i in 0..4 {
                if i <= chunk.len() {
                    out.push(char::from(ALPHABET[(n >> (18 - 6 * i) & 63) as usize]));
                } else {
                    out.push('=');
                }
            }
        }
        out
    }

    fn value(c: u8) -> Option<u32> {
        match c {
            b'A'..=b'Z' => Some(u32::from(c - b'A')),
            b'a'..=b'z' => Some(u32::from(c - b'a') + 26),
            b'0'..=b'9' => Some(u32::from(c - b'0') + 52),
            b'+' => Some(62),
            b'/' => Some(63),
            _ => None,
        }
    }

    pub fn decode(text: &str) -> Result<Vec<u8>, FormatDocError> {
        let bad = |m: &str| FormatDocError::Base64(m.to_owned());
        let bytes = text.as_bytes();
        if !bytes.len().is_multiple_of(4) {
            return Err(bad("length is not a multiple of 4"));
        }
        let mut out = Vec::with_capacity(bytes.len() / 4 * 3);
        for (i, quad) in bytes.chunks(4).enumerate() {
            let last = (i + 1) * 4 == bytes.len();
            let pad = quad.iter().rev().take_while(|c| **c == b'=').count();
            if pad > 2 || (pad > 0 && !last) {
                return Err(bad("misplaced padding"));
            }
            let mut n = 0u32;
            for (j, c) in quad[..4 - pad].iter().enumerate() {
                n |= value(*c).ok_or_else(|| bad("character outside the alphabet"))? << (18 - 6 * j);
            }
            out.extend_from_slice(&n.to_be_bytes()[1..4 - pad]);
        }
        Ok(out)
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;

    #[test]
    fn base64_matches_the_rfc_vectors() {
        for (plain, coded) in
            [("", ""), ("f", "Zg=="), ("fo", "Zm8="), ("foo", "Zm9v"), ("foob", "Zm9vYg=="), ("fooba", "Zm9vYmE="), ("foobar", "Zm9vYmFy")]
        {
            assert_eq!(base64::encode(plain.as_bytes()), coded);
            assert_eq!(base64::decode(coded).unwrap(), plain.as_bytes());
        }
    }

    #[test]
    fn base64_round_trips_every_byte_value() {
        let all: Vec<u8> = (0..=255).collect();
        assert_eq!(base64::decode(&base64::encode(&all)).unwrap(), all);
    }

    #[test]
    fn base64_rejects_bad_input() {
        for bad in ["Zg=", "Z", "Zm9v!A==", "Zg==Zm9v", "====", "Z==="] {
            assert!(matches!(base64::decode(bad), Err(FormatDocError::Base64(_))), "{bad}");
        }
    }

    #[test]
    fn image_doc_checks_its_size() {
        assert_eq!(ImageDoc::new(0, 2, vec![]), Err(FormatDocError::Empty));
        assert_eq!(ImageDoc::new(2, 1, vec![0; 7]), Err(FormatDocError::SizeMismatch { width: 2, height: 1, bytes: 7 }));
        assert!(ImageDoc::new(u32::MAX, u32::MAX, vec![0; 4]).is_err(), "the product must not overflow into a match");
        assert_eq!(ImageDoc::new(2, 1, vec![1; 8]).unwrap().rgba8.len(), 8);
    }

    #[test]
    fn a_response_round_trips() {
        let doc = ImageDoc::new(2, 1, vec![1, 2, 3, 4, 5, 6, 7, 8]).unwrap();
        assert_eq!(parse_read_response(&read_response(&doc)).unwrap(), doc);
        assert_eq!(parse_write_response(&write_response(b"hello")).unwrap(), b"hello");
    }

    #[test]
    fn a_malformed_response_names_the_field() {
        assert_eq!(parse_read_response(&json!({})), Err(FormatDocError::Field("doc")));
        assert_eq!(parse_read_response(&json!({"doc": {"kind": "audio"}})), Err(FormatDocError::Field("doc.kind")));
        assert_eq!(parse_read_response(&json!({"doc": {"kind": "image", "width": 1}})), Err(FormatDocError::Field("height")));
        assert_eq!(parse_write_response(&json!({"bytes": 5})), Err(FormatDocError::Field("bytes")));
    }

    #[test]
    fn extension_is_lower_case_and_needs_a_stem() {
        assert_eq!(extension_of("/a/b/Photo.XYZ").as_deref(), Some("xyz"));
        assert_eq!(extension_of("archive.tar.gz").as_deref(), Some("gz"));
        for none in ["noext", ".hidden", "trailing.", "dir.d/file"] {
            assert_eq!(extension_of(none), None, "{none}");
        }
    }
}
