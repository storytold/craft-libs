//! The parameter schema PhotoCraft and VectorCraft share (`{"name": {"type": "number", ...}}`).
//!
//! Both v1 manifests spell parameters the same way: a JSON object whose key order is the dialog
//! order, and four types. They differ in a few rules (duplicate keys, how a default is clamped,
//! how a whole number is read), so each adapter keeps its own `resolve_params`.

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

/// One declared parameter.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum ParamSpec {
    /// A real number in `min..=max`.
    Number { min: f64, max: f64, default: f64 },
    /// A whole number in `min..=max`.
    Int {
        #[serde(default = "int_min")]
        min: i64,
        #[serde(default = "int_max")]
        max: i64,
        #[serde(default)]
        default: i64,
    },
    /// A checkbox.
    Bool {
        #[serde(default)]
        default: bool,
    },
    /// One of `options`.
    Choice {
        options: Vec<String>,
        #[serde(default)]
        default: Option<String>,
    },
}

fn int_min() -> i64 {
    -1_000_000
}

fn int_max() -> i64 {
    1_000_000
}

impl ParamSpec {
    /// The value a parameter takes when it is not given.
    pub fn default_value(&self) -> Value {
        match self {
            ParamSpec::Number { default, .. } => json!(default),
            ParamSpec::Int { default, .. } => json!(default),
            ParamSpec::Bool { default } => json!(default),
            ParamSpec::Choice { options, default } => json!(default.clone().or_else(|| options.first().cloned()).unwrap_or_default()),
        }
    }

    /// The same parameter as a craft-abi 1 manifest `params` entry.
    pub(crate) fn to_craft(&self) -> Value {
        match self {
            ParamSpec::Number { min, max, default } => json!({ "type": "number", "min": min, "max": max, "default": default }),
            ParamSpec::Int { min, max, default } => json!({ "type": "int", "min": min, "max": max, "default": default }),
            ParamSpec::Bool { default } => json!({ "type": "bool", "default": default }),
            ParamSpec::Choice { options, .. } => json!({ "type": "choice", "options": options, "default": self.default_value() }),
        }
    }
}

/// Reads the `params` object keeping its key order (`serde_json::Map` would sort it).
/// `DEDUP` rejects a key declared twice. VectorCraft does, PhotoCraft does not.
pub(crate) fn ordered_params<'de, D: serde::Deserializer<'de>, const DEDUP: bool, const MAX: usize>(
    d: D,
) -> std::result::Result<Vec<(String, ParamSpec)>, D::Error> {
    struct V<const DEDUP: bool, const MAX: usize>;
    impl<'de, const DEDUP: bool, const MAX: usize> serde::de::Visitor<'de> for V<DEDUP, MAX> {
        type Value = Vec<(String, ParamSpec)>;

        fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
            f.write_str("an object of parameter specs")
        }

        fn visit_unit<E: serde::de::Error>(self) -> std::result::Result<Self::Value, E> {
            Ok(Vec::new())
        }

        fn visit_map<A: serde::de::MapAccess<'de>>(self, mut map: A) -> std::result::Result<Self::Value, A::Error> {
            let mut out: Vec<(String, ParamSpec)> = Vec::new();
            while let Some((k, v)) = map.next_entry::<String, ParamSpec>()? {
                if out.len() >= MAX {
                    return Err(serde::de::Error::custom(format!("too many parameters (limit {MAX})")));
                }
                if DEDUP && out.iter().any(|(o, _)| *o == k) {
                    return Err(serde::de::Error::custom(format!("parameter `{k}` is declared twice")));
                }
                out.push((k, v));
            }
            Ok(out)
        }
    }
    d.deserialize_any(V::<DEDUP, MAX>)
}

pub(crate) fn params_as_object<S: serde::Serializer>(p: &[(String, ParamSpec)], s: S) -> std::result::Result<S::Ok, S::Error> {
    use serde::ser::SerializeMap;
    let mut m = s.serialize_map(Some(p.len()))?;
    for (k, v) in p {
        m.serialize_entry(k, v)?;
    }
    m.end()
}

/// `[A-Za-z0-9._-]{1,64}`.
pub fn valid_id(id: &str) -> bool {
    (1..=64).contains(&id.len()) && id.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-'))
}

/// The craft-abi 1 `params` object for a v1 parameter list.
pub(crate) fn craft_params(params: &[(String, ParamSpec)]) -> Value {
    Value::Object(params.iter().map(|(k, v)| (k.clone(), v.to_craft())).collect())
}
