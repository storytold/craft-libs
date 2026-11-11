// Copyright (c) 2026 ArtCraft Team and the EffectCraft contributors
// SPDX-License-Identifier: MIT OR Apache-2.0
//
// Adapted from EffectCraft `accepted_params`, `doc_pieces`, and `params_schema`
// (`crates/engine/src/commands/mod.rs`). Those sources have no per-file license
// header. The copyright and the MIT OR Apache-2.0 terms live in that
// repository's LICENSE-MIT and LICENSE-APACHE. See ATTRIBUTION.md.

//! JSON Schema derived from a command params doc.
//!
//! EffectCraft docs are key lists: `{layers: [id|name|#n], add?, toggle?}`.
//! FilmCraft docs are `"key":type` lists: `{"name":str}`, `{"parent":binId?}`.
//! A doc that is already a JSON Schema object is kept. A doc that is not a
//! key list is opaque: it does not pretend to name any properties.

use serde_json::{Map, Value};

/// Where a schema came from. The same word is stored on the JSON value as
/// `x-craft-source`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SchemaSource {
    /// The doc was already a JSON Schema object (`type` and `properties`).
    Doc,
    /// The doc parsed into a property list.
    Derived,
    /// The doc is not a key list and not a JSON Schema object.
    Opaque,
}

impl SchemaSource {
    /// The `x-craft-source` string for this tag.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Doc => "doc",
            Self::Derived => "derived",
            Self::Opaque => "opaque",
        }
    }
}

/// A schema value plus the source tag stored inside that value.
#[derive(Clone, Debug, PartialEq)]
pub struct SchemaResult {
    /// JSON Schema object. Always contains `x-craft-source`.
    pub value: Value,
    /// Same tag as `value["x-craft-source"]`.
    pub source: SchemaSource,
}

/// Build a schema for one params doc.
pub fn from_doc(doc: &str) -> SchemaResult {
    if let Some(schema) = json_schema_doc(doc) {
        return finish(schema, SchemaSource::Doc);
    }
    if accepted_params(doc).is_some() {
        return finish(derived_schema(doc), SchemaSource::Derived);
    }
    finish(opaque_schema(doc), SchemaSource::Opaque)
}

fn finish(mut value: Value, source: SchemaSource) -> SchemaResult {
    if let Some(map) = value.as_object_mut() {
        map.insert("x-craft-source".to_string(), Value::String(source.as_str().to_string()));
    }
    SchemaResult { value, source }
}

/// A doc that already has `type` and `properties` is a JSON Schema object.
/// Key-list docs are not JSON (`str` is unquoted), so they fall through.
fn json_schema_doc(doc: &str) -> Option<Value> {
    let value: Value = serde_json::from_str(doc).ok()?;
    let obj = value.as_object()?;
    if obj.contains_key("type") && obj.contains_key("properties") { Some(value) } else { None }
}

fn derived_schema(doc: &str) -> Value {
    let mut props = Map::new();
    let mut required = Vec::new();
    if let Some(pieces) = doc_pieces(doc) {
        for piece in pieces {
            let keys = accepted_params(&format!("{{{piece}}}")).unwrap_or_default();
            let alternatives = keys.len() > 1;
            for key in keys {
                let optional = piece_is_optional(&piece, &key, alternatives);
                if !optional && !required.contains(&key) {
                    required.push(key.clone());
                }
                props.insert(key, serde_json::json!({ "description": piece }));
            }
        }
    }
    serde_json::json!({
        "type": "object",
        "description": doc,
        "properties": props,
        "required": required,
        "additionalProperties": false,
    })
}

/// Unparseable docs stay honest: any property is allowed, and none are named.
/// An empty key list `{}` is derived, not this shape.
fn opaque_schema(doc: &str) -> Value {
    serde_json::json!({
        "type": "object",
        "description": doc,
        "additionalProperties": true,
    })
}

/// `key?` (EffectCraft) is optional. So is every key in a `|` group
/// (`path|prop`). FilmCraft writes the marker after the type
/// (`"parent":binId?`, `"mix":"Stereo|Mono"?`). A `?` nested in `[]`, `{}`,
/// or `()` does not mark the outer key, so `points: [[x,y,pressure?]]` keeps
/// `points` required.
fn piece_is_optional(piece: &str, key: &str, alternatives: bool) -> bool {
    alternatives || piece.contains(&format!("{key}?")) || question_after_type(piece)
}

fn question_after_type(piece: &str) -> bool {
    let mut depth = 0i32;
    let mut in_string = false;
    let mut escaped = false;
    let mut seen_colon = false;
    for c in piece.chars() {
        if in_string {
            if escaped {
                escaped = false;
                continue;
            }
            if c == '\\' {
                escaped = true;
                continue;
            }
            if c == '"' {
                in_string = false;
            }
            continue;
        }
        match c {
            '"' => in_string = true,
            '{' | '[' | '(' => depth += 1,
            '}' | ']' | ')' => depth -= 1,
            ':' if depth == 0 => seen_colon = true,
            '?' if depth == 0 && seen_colon => return true,
            _ => {}
        }
    }
    false
}

/// Top-level keys of a params doc such as `{layers: [id|name|#n], add?, toggle?}`
/// or `{"name":str,"parent":binId?}`. `None` when the doc is not a `{…}` key list.
fn accepted_params(doc: &str) -> Option<Vec<String>> {
    let body = doc.trim().strip_prefix('{')?;
    let (mut keys, mut cur, mut depth, mut in_value) = (Vec::new(), String::new(), 0i32, false);
    for c in body.chars() {
        match c {
            '{' | '[' | '(' => depth += 1,
            ']' | ')' => depth -= 1,
            '}' if depth == 0 => {
                push_key(&mut cur, &mut keys);
                return Some(keys);
            }
            '}' => depth -= 1,
            ',' if depth == 0 => {
                push_key(&mut cur, &mut keys);
                in_value = false;
            }
            '|' if depth == 0 && !in_value => push_key(&mut cur, &mut keys),
            ':' if depth == 0 => {
                push_key(&mut cur, &mut keys);
                in_value = true;
            }
            _ if depth == 0 && !in_value => cur.push(c),
            _ => {}
        }
    }
    None
}

/// Quoted FilmCraft keys (`"name"`) yield the same identifier as bare keys.
fn push_key(cur: &mut String, keys: &mut Vec<String>) {
    let name = key_name(cur);
    if !name.is_empty() && !keys.contains(&name) {
        keys.push(name);
    }
    cur.clear();
}

fn key_name(raw: &str) -> String {
    let trimmed = raw.trim().trim_start_matches('"').trim_start();
    trimmed.chars().take_while(|c| c.is_ascii_alphanumeric() || *c == '_').collect()
}

/// Top-level pieces of a params doc, split at depth-0 commas.
/// `{layer?, path|prop, value}` is `["layer?", "path|prop", "value"]`.
fn doc_pieces(doc: &str) -> Option<Vec<String>> {
    let body = doc.trim().strip_prefix('{')?;
    let (mut out, mut cur, mut depth) = (Vec::new(), String::new(), 0i32);
    for c in body.chars() {
        match c {
            '{' | '[' | '(' => depth += 1,
            ']' | ')' => depth -= 1,
            '}' if depth == 0 => {
                if !cur.trim().is_empty() {
                    out.push(cur.trim().to_string());
                }
                return Some(out);
            }
            '}' => depth -= 1,
            ',' if depth == 0 => {
                if !cur.trim().is_empty() {
                    out.push(cur.trim().to_string());
                }
                cur.clear();
                continue;
            }
            _ => {}
        }
        cur.push(c);
    }
    None
}
