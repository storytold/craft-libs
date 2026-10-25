//! Property values: JSON, asset references and RGA text (spec §4.6).
//!
//! Every synchronized value is one of these. JSON values ride the wire as text inside the binary
//! encoding with `float_roundtrip` so decimal rotation is exact (v2 decision, kept). Asset bytes
//! never travel in batches — only their content hash does (spec §3.5 `assets`, the explicit fix
//! of the v2 base64-images design flaw). Text values opt into RGA per property (spec §6.6).

use crate::text::TextDoc;
use serde::{Deserialize, Serialize};
use std::sync::Arc;

/// A content-addressed asset reference: SHA-256 is the id, the bytes live in the asset channel.
#[derive(Clone, PartialEq, Eq, Hash, Serialize, Deserialize, Debug)]
pub struct AssetRef {
    /// Lowercase hex SHA-256 of the asset bytes.
    pub sha256: String,
    /// Byte length (validated on upload; the ref alone never carries bytes).
    pub bytes: u64,
}

/// A property value.
///
/// On the wire (postcard, spec §2.1) every value rides as **compact JSON text** — the v2
/// decision verbatim ("property values inside it are compact JSON text"), which also keeps
/// decimal rotation exact through `float_roundtrip`. The custom serde impl below is that
/// contract.
#[derive(Clone, Debug)]
pub enum Value {
    /// A JSON value (the default for every property).
    Json(serde_json::Value),
    /// An asset reference — bytes travel on the asset channel, never inside a batch (spec §3.5).
    AssetRef(AssetRef),
    /// An RGA text value (feature word `textcrdt`, spec §6.6) for fine-grained concurrent writing.
    Text(TextDoc),
}

impl Serialize for Value {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        serializer.serialize_str(&self.to_wire_json())
    }
}

impl<'de> Deserialize<'de> for Value {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let text = String::deserialize(deserializer)?;
        Value::from_wire_json(&text).ok_or_else(|| serde::de::Error::custom("malformed value wire json"))
    }
}

impl PartialEq for Value {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (Value::Json(a), Value::Json(b)) => a == b,
            (Value::AssetRef(a), Value::AssetRef(b)) => a == b,
            (Value::Text(a), Value::Text(b)) => a.render() == b.render(),
            _ => false,
        }
    }
}

impl Value {
    /// The compact JSON text this value rides the wire as (spec §2.1).
    #[must_use]
    pub fn to_wire_json(&self) -> String {
        match self {
            Value::Json(v) => v.to_string(),
            Value::AssetRef(r) => serde_json::json!({ "$asset": { "sha256": r.sha256, "bytes": r.bytes } }).to_string(),
            Value::Text(t) => serde_json::json!({ "$text": t }).to_string(),
        }
    }

    /// Parse a value from its wire JSON text (the exact inverse of [`Value::to_wire_json`]).
    #[must_use]
    pub fn from_wire_json(text: &str) -> Option<Self> {
        let v: serde_json::Value = serde_json::from_str(text).ok()?;
        match &v {
            serde_json::Value::Object(m) => {
                if let Some(asset) = m.get("$asset") {
                    let sha256 = asset.get("sha256")?.as_str()?.to_string();
                    let bytes = asset.get("bytes")?.as_u64()?;
                    return Some(Value::AssetRef(AssetRef { sha256, bytes }));
                }
                if let Some(text) = m.get("$text") {
                    let doc: TextDoc = serde_json::from_value(text.clone()).ok()?;
                    return Some(Value::Text(doc));
                }
                Some(Value::Json(v))
            }
            _ => Some(Value::Json(v)),
        }
    }

    /// The JSON view of this value (asset refs and text render to their JSON form).
    #[must_use]
    pub fn to_json(&self) -> serde_json::Value {
        match self {
            Value::Json(v) => v.clone(),
            Value::AssetRef(r) => serde_json::json!({ "sha256": r.sha256, "bytes": r.bytes }),
            Value::Text(t) => serde_json::Value::String(t.render()),
        }
    }

    /// A clone that never deep-copies large JSON more than once (Arc the big strings).
    #[must_use]
    pub fn shared(self) -> Self {
        match self {
            Value::Json(v) => Value::Json(v),
            v => v,
        }
    }
}

/// A property name (e.g. `"fills"`, `"transform"`): free-form, documented by the adapter.
pub type PropName = Arc<str>;

/// The kind of an object, defined by the app adapter (e.g. `"frame"`, `"layer"`, `"paragraph"`).
pub type KindName = Arc<str>;
