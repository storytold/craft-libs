//! Asset monikers: `craft://<app>/<doc>/<kind>/<id>`.
//!
//! The grammar is in `PROTOCOL.md` section 9. This module uses
//! only `std` and `serde`, so an app can copy it to a wasm build.

use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Deserializer, Serialize, Serializer};

use crate::error::LinkError;
use crate::ids::{valid_app_id, valid_uuid};

const SCHEME: &str = "craft://";
/// Longest decoded `id`, in bytes.
pub const MAX_ID_BYTES: usize = 512;

/// What an asset is. Unknown words are valid and kept in [`MonikerKind::Other`].
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum MonikerKind {
    Comp,
    Image,
    Vector,
    Sequence,
    Layer,
    Other(String),
}

impl MonikerKind {
    pub fn as_str(&self) -> &str {
        match self {
            Self::Comp => "comp",
            Self::Image => "image",
            Self::Vector => "vector",
            Self::Sequence => "sequence",
            Self::Layer => "layer",
            Self::Other(word) => word,
        }
    }

    fn parse(word: &str) -> Result<Self, LinkError> {
        if !valid_word(word, 32) {
            return Err(bad(format!("kind {word:?} must match [a-z][a-z0-9-]{{0,31}}")));
        }
        Ok(match word {
            "comp" => Self::Comp,
            "image" => Self::Image,
            "vector" => Self::Vector,
            "sequence" => Self::Sequence,
            "layer" => Self::Layer,
            other => Self::Other(other.to_string()),
        })
    }
}

/// A parsed moniker. `doc` keeps the hex case it was written with.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Moniker {
    app: String,
    doc: String,
    kind: MonikerKind,
    id: String,
}

impl Moniker {
    /// Build a moniker from parts. Every part is checked, so a value of this
    /// type always formats to text that parses back to an equal value.
    pub fn new(app: &str, doc: &str, kind: MonikerKind, id: &str) -> Result<Self, LinkError> {
        if !valid_app_id(app) {
            return Err(bad(format!("app {app:?} must match [a-z][a-z0-9-]{{1,31}}")));
        }
        if !valid_uuid(doc) {
            return Err(bad(format!("doc {doc:?} is not a uuid")));
        }
        if let MonikerKind::Other(word) = &kind
            && (!valid_word(word, 32) || is_known_word(word))
        {
            return Err(bad(format!("kind {word:?} is not a valid unknown kind")));
        }
        if id.is_empty() || id.len() > MAX_ID_BYTES {
            return Err(bad(format!("id must be 1 to {MAX_ID_BYTES} bytes")));
        }
        Ok(Self { app: app.to_string(), doc: doc.to_string(), kind, id: id.to_string() })
    }

    pub fn parse(text: &str) -> Result<Self, LinkError> {
        let Some(rest) = text.strip_prefix(SCHEME) else {
            return Err(bad(format!("moniker must start with {SCHEME}")));
        };
        let mut parts = rest.split('/');
        let (Some(app), Some(doc), Some(kind), Some(id), None) = (parts.next(), parts.next(), parts.next(), parts.next(), parts.next()) else {
            return Err(bad("moniker must have exactly app, doc, kind and id".to_string()));
        };
        if id.contains(['?', '#']) {
            return Err(bad("moniker has no query or fragment".to_string()));
        }
        let kind = MonikerKind::parse(kind)?;
        let id = decode_id(id)?;
        Self::new(app, doc, kind, &id)
    }

    pub fn app(&self) -> &str {
        &self.app
    }

    pub fn doc(&self) -> &str {
        &self.doc
    }

    pub fn kind(&self) -> &MonikerKind {
        &self.kind
    }

    /// The decoded id.
    pub fn id(&self) -> &str {
        &self.id
    }
}

impl fmt::Display for Moniker {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{SCHEME}{}/{}/{}/", self.app, self.doc, self.kind.as_str())?;
        for byte in self.id.bytes() {
            if is_unreserved(byte) {
                write!(f, "{}", char::from(byte))?;
            } else {
                write!(f, "%{byte:02X}")?;
            }
        }
        Ok(())
    }
}

impl FromStr for Moniker {
    type Err = LinkError;

    fn from_str(text: &str) -> Result<Self, LinkError> {
        Self::parse(text)
    }
}

impl Serialize for Moniker {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.collect_str(self)
    }
}

impl<'de> Deserialize<'de> for Moniker {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let text = String::deserialize(deserializer)?;
        Self::parse(&text).map_err(serde::de::Error::custom)
    }
}

fn bad(message: String) -> LinkError {
    LinkError::BadParams(message)
}

fn is_known_word(word: &str) -> bool {
    matches!(word, "comp" | "image" | "vector" | "sequence" | "layer")
}

fn is_unreserved(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'.' | b'_' | b'~')
}

fn valid_word(word: &str, max: usize) -> bool {
    let mut bytes = word.bytes();
    let Some(first) = bytes.next() else {
        return false;
    };
    word.len() <= max && first.is_ascii_lowercase() && bytes.all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
}

fn decode_id(text: &str) -> Result<String, LinkError> {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while let Some(&byte) = bytes.get(index) {
        if byte == b'%' {
            let pair = bytes.get(index + 1..index + 3).ok_or_else(|| bad("id has a truncated %XX escape".to_string()))?;
            let hi = hex_value(pair.first().copied());
            let lo = hex_value(pair.get(1).copied());
            let (Some(hi), Some(lo)) = (hi, lo) else {
                return Err(bad("id has a bad %XX escape".to_string()));
            };
            out.push(hi * 16 + lo);
            index += 3;
        } else if is_unreserved(byte) {
            out.push(byte);
            index += 1;
        } else {
            return Err(bad(format!("id has the raw byte 0x{byte:02X}, which must be percent-encoded")));
        }
    }
    String::from_utf8(out).map_err(|_| bad("id is not valid UTF-8".to_string()))
}

fn hex_value(byte: Option<u8>) -> Option<u8> {
    match byte? {
        b @ b'0'..=b'9' => Some(b - b'0'),
        b @ b'a'..=b'f' => Some(b - b'a' + 10),
        b @ b'A'..=b'F' => Some(b - b'A' + 10),
        _ => None,
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;

    const DOC: &str = "550e8400-e29b-41d4-a716-446655440000";

    fn sample(id: &str) -> Moniker {
        Moniker::new("photocraft", DOC, MonikerKind::Image, id).unwrap()
    }

    #[test]
    fn plain_id_formats_and_parses() {
        let text = format!("craft://photocraft/{DOC}/image/layer-3");
        assert_eq!(sample("layer-3").to_string(), text);
        assert_eq!(Moniker::parse(&text).unwrap(), sample("layer-3"));
    }

    #[test]
    fn round_trip_keeps_awkward_ids() {
        for id in ["a b", "a/b", "100%", "x?y#z", "caf\u{e9}", "\u{1F600}", "~._-", "%2F", "a%20b", &"x".repeat(MAX_ID_BYTES)] {
            let m = sample(id);
            let text = m.to_string();
            let back = Moniker::parse(&text).unwrap();
            assert_eq!(back, m, "{id:?} via {text}");
            assert_eq!(back.id(), id);
            assert_eq!(back.to_string(), text);
        }
    }

    #[test]
    fn encoded_slash_stays_inside_the_id() {
        let m = sample("a/b");
        assert_eq!(m.to_string(), format!("craft://photocraft/{DOC}/image/a%2Fb"));
        assert_eq!(Moniker::parse(&m.to_string()).unwrap().id(), "a/b");
    }

    #[test]
    fn lowercase_escapes_parse_and_format_uppercase() {
        let m = Moniker::parse(&format!("craft://photocraft/{DOC}/image/a%2fb")).unwrap();
        assert_eq!(m.id(), "a/b");
        assert_eq!(m.to_string(), format!("craft://photocraft/{DOC}/image/a%2Fb"));
    }

    #[test]
    fn doc_case_is_kept() {
        let upper = DOC.to_uppercase();
        let m = Moniker::parse(&format!("craft://photocraft/{upper}/comp/c1")).unwrap();
        assert_eq!(m.doc(), upper);
        assert_eq!(m.to_string(), format!("craft://photocraft/{upper}/comp/c1"));
    }

    #[test]
    fn kinds_known_and_unknown() {
        for (word, kind) in [
            ("comp", MonikerKind::Comp),
            ("image", MonikerKind::Image),
            ("vector", MonikerKind::Vector),
            ("sequence", MonikerKind::Sequence),
            ("layer", MonikerKind::Layer),
            ("palette", MonikerKind::Other("palette".into())),
        ] {
            let m = Moniker::parse(&format!("craft://designcraft/{DOC}/{word}/x")).unwrap();
            assert_eq!(m.kind(), &kind);
            assert_eq!(m.to_string(), format!("craft://designcraft/{DOC}/{word}/x"));
        }
        assert!(Moniker::new("designcraft", DOC, MonikerKind::Other("comp".into()), "x").is_err(), "known word must use its variant");
    }

    #[test]
    fn rejects_bad_text() {
        let bad_texts = [
            "".to_string(),
            "craft:/photocraft".to_string(),
            "http://photocraft/x/image/a".to_string(),
            format!("craft://photocraft/{DOC}/image"),
            format!("craft://photocraft/{DOC}/image/a/b"),
            format!("craft://photocraft/{DOC}/image/"),
            format!("craft://photocraft/{DOC}/image/a?x=1"),
            format!("craft://photocraft/{DOC}/image/a#frag"),
            format!("craft://photocraft/{DOC}/image/a b"),
            format!("craft://photocraft/{DOC}/image/%"),
            format!("craft://photocraft/{DOC}/image/%4"),
            format!("craft://photocraft/{DOC}/image/%GG"),
            format!("craft://photocraft/{DOC}/image/%FF"),
            format!("craft://Photo/{DOC}/image/a"),
            format!("craft://p/{DOC}/image/a"),
            "craft://photocraft/not-a-uuid/image/a".to_string(),
            format!("craft://photocraft/{DOC}/Image/a"),
            format!("craft://photocraft/{DOC}/1image/a"),
            format!("craft://photocraft/{DOC}/image/{}", "x".repeat(MAX_ID_BYTES + 1)),
        ];
        for text in bad_texts {
            assert!(Moniker::parse(&text).is_err(), "{text:?} should not parse");
        }
    }

    #[test]
    fn serde_uses_the_string_form() {
        let m = sample("a b");
        let value = serde_json::to_value(&m).unwrap();
        assert_eq!(value, serde_json::json!(format!("craft://photocraft/{DOC}/image/a%20b")));
        assert_eq!(serde_json::from_value::<Moniker>(value).unwrap(), m);
        assert!(serde_json::from_value::<Moniker>(serde_json::json!("craft://nope")).is_err());
    }
}
