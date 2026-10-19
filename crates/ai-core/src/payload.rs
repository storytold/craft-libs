//! Payloads: the data that goes to and comes back from a provider.
//!
//! Small things travel inline. Large media (video, long audio, big PDFs) travels as a
//! [`PayloadHandle`], a reference the adapter streams from, so the library never forces a
//! gigabyte into memory. Inline data is serialized as base64 and capped by [`Limits`].

use serde::{Deserialize, Serialize};

use crate::error::AiError;
use crate::ids::{IdError, validated_string};

/// Default cap on inline binary data. Larger data should be a handle.
pub const DEFAULT_MAX_INLINE_BYTES: u64 = 32 * 1024 * 1024;
/// Default cap on inline text.
pub const DEFAULT_MAX_TEXT_BYTES: usize = 4 * 1024 * 1024;
const MAX_URI_LEN: usize = 4096;

/// Size limits applied to payloads in a request.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Limits {
    pub max_inline_bytes: u64,
    pub max_text_bytes: usize,
}

impl Default for Limits {
    fn default() -> Self {
        Self { max_inline_bytes: DEFAULT_MAX_INLINE_BYTES, max_text_bytes: DEFAULT_MAX_TEXT_BYTES }
    }
}

/// Lower-case `type/subtype`, such as `image/png`.
pub fn validate_media_type(s: &str) -> Result<(), IdError> {
    if s.is_empty() {
        return Err(IdError::Empty);
    }
    if s.len() > 127 {
        return Err(IdError::TooLong { len: s.len(), max: 127 });
    }
    if let Some(c) = s.chars().find(|c| !(c.is_ascii_lowercase() || c.is_ascii_digit() || matches!(c, '/' | '-' | '.' | '+' | '_'))) {
        return Err(IdError::BadChar(c));
    }
    match s.split_once('/') {
        Some((a, b)) if !a.is_empty() && !b.is_empty() && !b.contains('/') => Ok(()),
        _ => Err(IdError::Malformed("expected type/subtype")),
    }
}

validated_string!(
    /// A validated media type such as `image/png`.
    MediaType,
    validate_media_type
);

impl MediaType {
    pub fn kind(&self) -> PayloadKind {
        let s = self.as_str();
        if s.starts_with("image/") {
            PayloadKind::Image
        } else if s.starts_with("video/") {
            PayloadKind::Video
        } else if s.starts_with("audio/") {
            PayloadKind::Audio
        } else if s.starts_with("text/") || s == "application/json" {
            PayloadKind::Text
        } else if s == "application/pdf" {
            PayloadKind::Document
        } else {
            PayloadKind::Other
        }
    }
}

/// A coarse description of what a payload is. Used in consent text and logs, which must say what
/// leaves the machine without printing it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PayloadKind {
    Text,
    Image,
    Video,
    Audio,
    Document,
    Other,
}

/// A reference to data that stays where it is until a provider adapter streams it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PayloadHandle {
    pub uri: String,
    pub media_type: MediaType,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub size_bytes: Option<u64>,
}

/// Data in a request or a result.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Payload {
    Text {
        text: String,
    },
    Inline {
        media_type: MediaType,
        #[serde(with = "base64_bytes")]
        data: Vec<u8>,
    },
    Handle(PayloadHandle),
}

impl Payload {
    pub fn text(text: impl Into<String>) -> Self {
        Self::Text { text: text.into() }
    }

    pub fn inline(media_type: &str, data: Vec<u8>) -> Result<Self, AiError> {
        let media_type = MediaType::new(media_type).map_err(|e| AiError::invalid(format!("media type: {e}")))?;
        Ok(Self::Inline { media_type, data })
    }

    pub fn handle(uri: &str, media_type: &str, size_bytes: Option<u64>) -> Result<Self, AiError> {
        if uri.is_empty() || uri.len() > MAX_URI_LEN || uri.chars().any(char::is_control) {
            return Err(AiError::invalid("payload uri is empty, too long, or has control characters"));
        }
        let media_type = MediaType::new(media_type).map_err(|e| AiError::invalid(format!("media type: {e}")))?;
        Ok(Self::Handle(PayloadHandle { uri: uri.to_owned(), media_type, size_bytes }))
    }

    pub fn kind(&self) -> PayloadKind {
        match self {
            Self::Text { .. } => PayloadKind::Text,
            Self::Inline { media_type, .. } => media_type.kind(),
            Self::Handle(h) => h.media_type.kind(),
        }
    }

    /// Size in bytes when known without reading anything.
    pub fn size_hint(&self) -> Option<u64> {
        match self {
            Self::Text { text } => u64::try_from(text.len()).ok(),
            Self::Inline { data, .. } => u64::try_from(data.len()).ok(),
            Self::Handle(h) => h.size_bytes,
        }
    }

    /// Refuses payloads over the limits, telling the caller to use a handle for big data.
    pub fn check(&self, limits: &Limits) -> Result<(), AiError> {
        match self {
            Self::Text { text } if text.len() > limits.max_text_bytes => {
                Err(AiError::invalid(format!("text is {} bytes, the limit is {}", text.len(), limits.max_text_bytes)))
            }
            Self::Inline { data, .. } if u64::try_from(data.len()).unwrap_or(u64::MAX) > limits.max_inline_bytes => Err(AiError::invalid(format!(
                "inline data is {} bytes, the limit is {}; pass a handle for large media",
                data.len(),
                limits.max_inline_bytes
            ))),
            _ => Ok(()),
        }
    }
}

const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

/// Standard base64 with padding.
pub fn base64_encode(data: &[u8]) -> String {
    let mut out = String::with_capacity(data.len().div_ceil(3).saturating_mul(4));
    for chunk in data.chunks(3) {
        let b0 = u32::from(chunk.first().copied().unwrap_or(0));
        let b1 = u32::from(chunk.get(1).copied().unwrap_or(0));
        let b2 = u32::from(chunk.get(2).copied().unwrap_or(0));
        let n = (b0 << 16) | (b1 << 8) | b2;
        for shift in [18u32, 12, 6, 0].into_iter().take(chunk.len() + 1) {
            let idx = ((n >> shift) & 63) as usize;
            out.push(ALPHABET.get(idx).copied().map_or('?', char::from));
        }
        for _ in chunk.len()..3 {
            out.push('=');
        }
    }
    out
}

/// Decodes standard base64 with padding. Returns `None` for any malformed input.
pub fn base64_decode(text: &str) -> Option<Vec<u8>> {
    let bytes = text.as_bytes();
    if bytes.len() & 3 != 0 {
        return None;
    }
    let mut out = Vec::with_capacity(bytes.len() / 4 * 3);
    let chunk_count = bytes.len() / 4;
    for (i, chunk) in bytes.chunks(4).enumerate() {
        let last = i + 1 == chunk_count;
        let mut n: u32 = 0;
        let mut pad = 0usize;
        for (pos, &c) in chunk.iter().enumerate() {
            let v = if c == b'=' {
                if !last || pos < 2 {
                    return None;
                }
                pad += 1;
                0
            } else {
                if pad > 0 {
                    return None;
                }
                u32::try_from(ALPHABET.iter().position(|&a| a == c)?).ok()?
            };
            n = (n << 6) | v;
        }
        let triple = [(n >> 16) as u8, (n >> 8) as u8, n as u8];
        out.extend_from_slice(triple.get(..3usize.saturating_sub(pad))?);
    }
    Some(out)
}

mod base64_bytes {
    use serde::{Deserialize, Deserializer, Serializer};

    pub fn serialize<S: Serializer>(data: &[u8], s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&super::base64_encode(data))
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Vec<u8>, D::Error> {
        let text = String::deserialize(d)?;
        super::base64_decode(&text).ok_or_else(|| serde::de::Error::custom("invalid base64"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base64_matches_known_vectors() {
        for (raw, enc) in [("", ""), ("f", "Zg=="), ("fo", "Zm8="), ("foo", "Zm9v"), ("foob", "Zm9vYg=="), ("foobar", "Zm9vYmFy")] {
            assert_eq!(base64_encode(raw.as_bytes()), enc);
            assert_eq!(base64_decode(enc).unwrap(), raw.as_bytes());
        }
    }

    #[test]
    fn base64_round_trips_all_byte_values() {
        let data: Vec<u8> = (0..=255u8).collect();
        assert_eq!(base64_decode(&base64_encode(&data)).unwrap(), data);
    }

    #[test]
    fn base64_rejects_malformed() {
        for bad in ["Zg=", "Z", "Zm9v!", "=Zm9", "Zg==Zg==", "Zm=v", "Z===", "Zm9v\n"] {
            assert!(base64_decode(bad).is_none(), "{bad:?}");
        }
    }

    #[test]
    fn media_type_validation() {
        assert!(MediaType::new("image/png").is_ok());
        assert!(MediaType::new("application/vnd.api+json").is_ok());
        for bad in ["", "png", "image/", "/png", "Image/PNG", "image/png/x", "image/p ng"] {
            assert!(MediaType::new(bad).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn kinds_follow_media_type() {
        assert_eq!(Payload::inline("image/png", vec![1]).unwrap().kind(), PayloadKind::Image);
        assert_eq!(Payload::inline("application/pdf", vec![1]).unwrap().kind(), PayloadKind::Document);
        assert_eq!(Payload::text("hi").kind(), PayloadKind::Text);
    }

    #[test]
    fn inline_over_limit_is_refused_with_a_hint() {
        let limits = Limits { max_inline_bytes: 4, max_text_bytes: 4 };
        let err = Payload::inline("image/png", vec![0; 5]).unwrap().check(&limits).unwrap_err();
        assert!(err.message.contains("handle"));
        assert!(Payload::text("12345").check(&limits).is_err());
        assert!(Payload::text("1234").check(&limits).is_ok());
    }

    #[test]
    fn handles_skip_the_inline_limit() {
        let limits = Limits { max_inline_bytes: 4, max_text_bytes: 4 };
        let h = Payload::handle("file:///big.mov", "video/quicktime", Some(10_000_000_000)).unwrap();
        assert!(h.check(&limits).is_ok());
        assert_eq!(h.size_hint(), Some(10_000_000_000));
    }

    #[test]
    fn handle_rejects_bad_uris() {
        assert!(Payload::handle("", "video/mp4", None).is_err());
        assert!(Payload::handle("file:///a\nb", "video/mp4", None).is_err());
        assert!(Payload::handle(&"x".repeat(MAX_URI_LEN + 1), "video/mp4", None).is_err());
    }

    #[test]
    fn json_round_trip() {
        let p = Payload::inline("image/png", vec![0, 1, 2, 250, 255]).unwrap();
        let json = serde_json::to_string(&p).unwrap();
        assert!(json.contains("\"type\":\"inline\""));
        assert_eq!(serde_json::from_str::<Payload>(&json).unwrap(), p);
        let bad = r#"{"type":"inline","media_type":"image/png","data":"!!"}"#;
        assert!(serde_json::from_str::<Payload>(bad).is_err());
    }
}
