//! Commands with parameters and their binding keys.

use std::fmt;

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Longest binding key [`CommandRef::parse_key`] accepts, in bytes.
pub const MAX_KEY: usize = 256;

/// A command id with the parameters a binding runs it with.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CommandRef {
    pub id: String,
    /// `Null` for no parameters.
    #[serde(default)]
    pub params: Value,
}

impl CommandRef {
    /// A command without parameters.
    pub fn new(id: impl Into<String>) -> CommandRef {
        CommandRef { id: id.into(), params: Value::Null }
    }

    pub fn with_params(id: impl Into<String>, params: Value) -> CommandRef {
        CommandRef { id: id.into(), params }
    }

    /// The binding key: the id, or `id {json}` with object keys sorted and no spaces. `Null` and
    /// an empty object give the bare id.
    pub fn key(&self) -> String {
        match &self.params {
            Value::Null => self.id.clone(),
            Value::Object(m) if m.is_empty() => self.id.clone(),
            p => {
                let mut out = String::with_capacity(self.id.len() + 16);
                out.push_str(&self.id);
                out.push(' ');
                write_canonical(p, &mut out);
                out
            }
        }
    }

    /// Reads a binding key: the id up to the first space, then JSON parameters. `None` for an
    /// empty id, a key over [`MAX_KEY`] bytes or parameters that are not JSON.
    pub fn parse_key(key: &str) -> Option<CommandRef> {
        if key.len() > MAX_KEY {
            return None;
        }
        let (id, params) = match key.split_once(' ') {
            Some((id, rest)) => (id, serde_json::from_str(rest).ok()?),
            None => (key, Value::Null),
        };
        if id.is_empty() || id.chars().any(char::is_whitespace) {
            return None;
        }
        Some(CommandRef { id: id.to_string(), params })
    }
}

impl fmt::Display for CommandRef {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.key())
    }
}

impl From<&str> for CommandRef {
    fn from(id: &str) -> CommandRef {
        CommandRef::new(id)
    }
}

// Written by hand so the key does not depend on serde_json's map order: an app's dependency graph
// may turn on `preserve_order`.
fn write_canonical(v: &Value, out: &mut String) {
    match v {
        Value::Null => out.push_str("null"),
        Value::Bool(b) => out.push_str(if *b { "true" } else { "false" }),
        Value::Number(n) => out.push_str(&n.to_string()),
        Value::String(s) => write_string(s, out),
        Value::Array(items) => {
            out.push('[');
            for (i, item) in items.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                write_canonical(item, out);
            }
            out.push(']');
        }
        Value::Object(map) => {
            let mut keys: Vec<&String> = map.keys().collect();
            keys.sort();
            out.push('{');
            for (i, k) in keys.into_iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                write_string(k, out);
                out.push(':');
                if let Some(item) = map.get(k) {
                    write_canonical(item, out);
                }
            }
            out.push('}');
        }
    }
}

fn write_string(s: &str, out: &mut String) {
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
}
