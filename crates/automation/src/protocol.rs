//! The JSON-lines wire format and the parameter validators every method uses.
//!
//! Request: `{"id": 1, "method": "ui.click", "params": {"x": 10, "y": 20}}` on one line. Reply:
//! `{"id": 1, "ok": true, "result": …}` or `{"id": 1, "ok": false, "error": "…"}` on one line,
//! at most [`MAX_RESPONSE_BYTES`]. Validators reject unknown fields, wrong types, unknown choices
//! and out-of-range numbers before a method acts, with PhotoCraft's `control/params.rs` messages
//! (``unknown button `x` (left, right, secondary, middle)``, `params.rs:187`).

use std::io::Write;

use std::time::Duration;

use serde_json::{Value, json};

/// Longest reply, in bytes (the newline included); a longer one is replaced by an error reply.
pub const MAX_RESPONSE_BYTES: usize = 8 << 20;
/// Largest PNG a headless render may return or write: `automation PNG exceeds {MAX_PNG_BYTES}
/// bytes` otherwise (PhotoCraft `crates/automation/src/budgets.rs:15`, checked at `:42`). The
/// UI screenshot reply doesn't apply it; [`crate::screenshot::MAX_IMAGE_BYTES`] bounds that.
pub const MAX_PNG_BYTES: usize = 5 << 20;

/// One parsed request line.
#[derive(Clone, Debug, PartialEq)]
pub struct Request {
    /// The caller's id, echoed in the reply; `null` when absent.
    pub id: Value,
    /// The method name.
    pub method: String,
    /// The method parameters; `{}` when absent.
    pub params: Value,
}

impl Request {
    /// Parse a line: `id` defaults to `null`, `method` to `""`, `params` to `{}`. Text that is not
    /// JSON is `bad JSON: {error}`.
    pub fn parse(line: &str) -> Result<Self, String> {
        let msg: Value = serde_json::from_str(line).map_err(|e| format!("bad JSON: {e}"))?;
        Ok(Self {
            id: msg.get("id").cloned().unwrap_or(Value::Null),
            method: msg.get("method").and_then(Value::as_str).unwrap_or("").to_owned(),
            params: msg.get("params").cloned().unwrap_or_else(|| json!({})),
        })
    }
}

/// `{"id", "ok": true, "result"}`.
pub fn reply_ok(id: &Value, result: Value) -> Value {
    json!({"id": id, "ok": true, "result": result})
}

/// `{"id", "ok": false, "error"}`.
pub fn reply_err(id: &Value, error: impl std::fmt::Display) -> Value {
    json!({"id": id, "ok": false, "error": error.to_string()})
}

struct LimitedWriter {
    bytes: Vec<u8>,
    maximum: usize,
}

impl Write for LimitedWriter {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        if buf.len() > self.maximum.saturating_sub(self.bytes.len()) {
            return Err(std::io::Error::new(std::io::ErrorKind::InvalidData, format!("response exceeds {} bytes", self.maximum)));
        }
        self.bytes.try_reserve(buf.len()).map_err(|error| std::io::Error::other(format!("response allocation failed: {error}")))?;
        self.bytes.extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

/// Serialize into a bounded buffer, failing as soon as `maximum` bytes would be passed.
fn encode_with_limit(value: &Value, maximum: usize) -> std::io::Result<Vec<u8>> {
    let mut writer = LimitedWriter { bytes: Vec::new(), maximum };
    serde_json::to_writer(&mut writer, value).map_err(std::io::Error::from)?;
    Ok(writer.bytes)
}

/// Write `reply` as one line, replacing a reply over [`MAX_RESPONSE_BYTES`] by an error reply with
/// the same `id` and `response exceeds {MAX_RESPONSE_BYTES} bytes; operation may have completed`.
/// The whole reply is encoded before anything is written, so no partial reply goes out.
pub fn write_reply(out: &mut impl Write, reply: &Value) -> std::io::Result<()> {
    let encoded = match encode_with_limit(reply, MAX_RESPONSE_BYTES - 1) {
        Ok(bytes) => bytes,
        Err(_) => {
            let error = json!({
                "id": reply.get("id").cloned().unwrap_or(Value::Null),
                "ok": false,
                "error": format!("response exceeds {MAX_RESPONSE_BYTES} bytes; operation may have completed"),
            });
            // Request ids arriving over the wire fit in MAX_REQUEST_BYTES, but a caller can build a
            // reply directly, so the error is bounded too.
            encode_with_limit(&error, MAX_RESPONSE_BYTES - 1)
                .unwrap_or_else(|_| b"{\"id\":null,\"ok\":false,\"error\":\"response budget exceeded\"}".to_vec())
        }
    };
    out.write_all(&encoded)?;
    out.write_all(b"\n")
}

/// The pointer buttons a request may name.
pub const BUTTONS: [&str; 4] = ["left", "right", "secondary", "middle"];
/// The per-request reply deadline.
pub const REPLY_DEADLINE: Duration = Duration::from_secs(60);

/// The modifier flags of `ui.key`.
pub const KEY_MODIFIERS: [&str; 4] = ["shift", "alt", "command", "ctrl"];
/// The modifier flags of `ui.pointer` (`space` is the momentary pan).
pub const POINTER_MODIFIERS: [&str; 5] = ["shift", "alt", "command", "ctrl", "space"];

/// Typed readers over a request's `params` object; every error names the field.
#[derive(Clone, Copy, Debug)]
pub struct Params<'a>(pub &'a Value);

impl Params<'_> {
    /// `key` as a bool, `None` when absent.
    pub fn bool_field(&self, key: &str) -> Result<Option<bool>, String> {
        match self.0.get(key) {
            None => Ok(None),
            Some(v) => v.as_bool().map(Some).ok_or_else(|| format!("{key} must be a boolean")),
        }
    }

    /// `key` as a non-negative integer.
    pub fn uint_field(&self, key: &str) -> Result<Option<u64>, String> {
        match self.0.get(key) {
            None => Ok(None),
            Some(v) if v.is_u64() => Ok(v.as_u64()),
            Some(v) => v
                .as_f64()
                .filter(|f| f.fract() == 0.0 && *f >= 0.0)
                .map(|f| Some(f as u64))
                .ok_or_else(|| format!("{key} must be a non-negative integer")),
        }
    }

    /// `key` as a finite number.
    pub fn num_field(&self, key: &str) -> Result<Option<f64>, String> {
        match self.0.get(key) {
            None => Ok(None),
            Some(v) => v.as_f64().filter(|f| f.is_finite()).map(Some).ok_or_else(|| format!("{key} must be a finite number")),
        }
    }

    /// `key` as a finite number within `f32` range.
    pub fn f32_field(&self, key: &str) -> Result<Option<f64>, String> {
        match self.0.get(key) {
            None => Ok(None),
            Some(v) => v
                .as_f64()
                .filter(|f| f.abs() <= f64::from(f32::MAX))
                .map(Some)
                .ok_or_else(|| format!("{key} must be a finite number within f32 range")),
        }
    }

    /// `key` as a string.
    pub fn str_field(&self, key: &str) -> Result<Option<&str>, String> {
        self.choice(key, key, &[])
    }

    /// The value must be an object with only `fields`, returned with its null fields dropped so
    /// they mean "omitted" (a typo must not reply ok after acting on a default). `what` is `None`
    /// for a method's params (`params must be an object`, ``unknown field `x` (fields: a, b)``,
    /// `null` is `{}`) and the object's name for a nested one (`modifiers must be an object`,
    /// ``unknown modifiers field `x` (fields: a, b)``).
    pub fn only(&self, what: Option<&str>, fields: &[&str]) -> Result<Value, String> {
        let object = match (self.0, what) {
            (Value::Null, None) => return Ok(json!({})),
            (Value::Object(o), _) => o,
            (_, what) => return Err(format!("{} must be an object", what.unwrap_or("params"))),
        };
        if let Some(key) = object.keys().find(|k| !fields.contains(&k.as_str())) {
            let field = what.map_or_else(|| "field".to_owned(), |w| format!("{w} field"));
            return Err(format!("unknown {field} `{key}` (fields: {})", fields.join(", ")));
        }
        Ok(Value::Object(object.iter().filter(|(_, v)| !v.is_null()).map(|(k, v)| (k.clone(), v.clone())).collect()))
    }

    /// `key` as a number within `range` (inclusive), `name` in the error.
    pub fn number_in(&self, key: &str, name: &str, range: Option<(f64, f64)>) -> Result<Option<f64>, String> {
        let n = self.num_field(key).map_err(|_| format!("{name} must be a finite number"))?;
        match (n, range) {
            (Some(n), Some((lo, hi))) if !(lo..=hi).contains(&n) => Err(format!("{name} must be between {lo} and {hi} (got {n})")),
            _ => Ok(n),
        }
    }

    /// `key` as one of `accepted`.
    pub fn choice<'v>(&'v self, key: &str, name: &str, accepted: &[&str]) -> Result<Option<&'v str>, String> {
        match self.0.get(key) {
            None => Ok(None),
            Some(Value::String(s)) if accepted.is_empty() || accepted.contains(&s.as_str()) => Ok(Some(s)),
            Some(Value::String(s)) => Err(format!("unknown {name} `{s}` ({})", accepted.join(", "))),
            Some(_) => Err(format!("{name} must be a string")),
        }
    }

    /// The modifier flags, top-level or grouped under `modifiers` (an object of the `flags`
    /// only, each a bool, errors prefixed `modifiers.`). When grouped, the top-level flags are
    /// ignored but still type-checked. The result is as the platform sees it
    /// ([`craft_keymap::Mods::effective`]): off the Mac `ctrl` folds into `cmd`, so a request that
    /// sets only one reaches the same shortcuts as a real Ctrl press; on the Mac they stay apart.
    #[cfg(feature = "ui")]
    pub fn modifiers(&self, flags: &[&str]) -> Result<craft_keymap::Mods, String> {
        for flag in flags {
            self.bool_field(flag)?;
        }
        let grouped = match self.0.get("modifiers") {
            None => None,
            Some(m) => {
                let m = Params(m).only(Some("modifiers"), flags)?;
                for flag in flags {
                    Params(&m).bool_field(flag).map_err(|e| format!("modifiers.{e}"))?;
                }
                Some(m)
            }
        };
        let m = grouped.as_ref().unwrap_or(self.0);
        let flag = |k: &str| m.get(k).and_then(Value::as_bool).unwrap_or(false);
        let held = craft_keymap::Mods { cmd: flag("command"), ctrl: flag("ctrl"), alt: flag("alt"), shift: flag("shift") };
        Ok(held.effective(craft_keymap::Platform::current()))
    }

    /// Deserialize `T` from the whole object, replacing `current`. A key `current` doesn't have
    /// when serialized is ``unknown {what} field `k` ``, whether or not `T` denies unknown fields;
    /// a non-object is `{what} must be an object` and a serde error is `{what}: {error}`.
    pub fn whole<T: serde::Serialize + serde::de::DeserializeOwned>(&self, what: &str, current: &T) -> Result<T, String> {
        let Some(patch) = self.0.as_object() else { return Err(format!("{what} must be an object")) };
        let known = serde_json::to_value(current).map_err(|e| e.to_string())?;
        if let Some(key) = patch.keys().find(|k| known.as_object().is_none_or(|o| !o.contains_key(*k))) {
            return Err(format!("unknown {what} field `{key}`"));
        }
        serde_json::from_value(Value::Object(patch.clone())).map_err(|e| format!("{what}: {e}"))
    }

    /// `current` serialized, the object's fields merged over it, deserialized back: a partial
    /// `ui.set`-style update. A field `current` doesn't have is ``unknown {what} field `k` ``.
    pub fn merged_into<T: serde::Serialize + serde::de::DeserializeOwned>(&self, what: &str, current: &T) -> Result<T, String> {
        let Some(patch) = self.0.as_object() else { return Err(format!("{what} must be an object")) };
        let mut merged = serde_json::to_value(current).map_err(|e| e.to_string())?;
        let Some(fields) = merged.as_object_mut() else { return Err(format!("{what} must be an object")) };
        for (key, value) in patch {
            if !fields.contains_key(key) {
                return Err(format!("unknown {what} field `{key}`"));
            }
            fields.insert(key.clone(), value.clone());
        }
        serde_json::from_value(merged).map_err(|e| format!("{what}: {e}"))
    }
}
