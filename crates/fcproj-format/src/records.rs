//! Reversible record boundaries for a FilmCraft project. A bin owns an ordered list of
//! references; each project item (including a sequence) has one payload. Fields outside
//! those boundaries stay in their original JSON objects, including editor and extension data.
//!
//! These types describe logical records, independent of directory names, hashing, and commits.

use crate::{FormatError, MIGRATIONS, check_supported, detect_bytes, migrate_with};
use serde::Deserialize;
use serde_json::{Map, Value};
use std::collections::BTreeMap;

type Result<T> = std::result::Result<T, FormatError>;
type Payloads = BTreeMap<u64, Value>;

// Reassembly follows links between separately supplied records, outside the JSON parser's
// recursion limit. A repeated or cyclic bin is also rejected on its second visit.
const MAX_BIN_DEPTH: usize = 64;

/// The envelope carries the root bin ID and a complete inventory of item IDs, including
/// items outside any bin. The map keys are persisted project IDs, never display names.
#[derive(Debug)]
pub struct ProjectRecords {
    pub envelope: Value,
    pub bins: BTreeMap<u64, Value>,
    pub items: BTreeMap<u64, Value>,
}

fn corrupt(message: impl Into<String>) -> FormatError {
    FormatError::Corrupt(message.into())
}

fn object_id(value: &Value) -> Result<u64> {
    value.get("id").and_then(Value::as_u64).ok_or_else(|| corrupt("record lacks an unsigned id"))
}

fn children(bin: &mut Value, depth: usize) -> Result<&mut Vec<Value>> {
    if depth > MAX_BIN_DEPTH {
        return Err(corrupt("bin nesting exceeds the supported depth"));
    }
    bin.get_mut("children").and_then(Value::as_array_mut).ok_or_else(|| corrupt("bin lacks children"))
}

// Only nested bins change representation. Item references retain their original position.
fn nested_or_item<'a>(child: &'a mut Value, items: &Payloads) -> Result<Option<&'a mut Value>> {
    let entry = child.as_object_mut().filter(|o| o.len() == 1).ok_or_else(|| corrupt("invalid bin entry"))?;
    if let Some(item) = entry.get("Item") {
        let id = item.as_u64().ok_or_else(|| corrupt("invalid item reference"))?;
        if !items.contains_key(&id) {
            return Err(corrupt(format!("bin references missing item {id}")));
        }
        Ok(None)
    } else {
        entry.get_mut("Bin").map(Some).ok_or_else(|| corrupt("unknown bin entry"))
    }
}

fn split_bin(mut bin: Value, bins: &mut Payloads, items: &Payloads, depth: usize) -> Result<u64> {
    let id = object_id(&bin)?;
    for child in children(&mut bin, depth)? {
        if let Some(nested) = nested_or_item(child, items)? {
            *nested = Value::from(split_bin(nested.take(), bins, items, depth + 1)?);
        }
    }
    if bins.insert(id, bin).is_some() {
        return Err(corrupt(format!("duplicate bin id {id}")));
    }
    Ok(id)
}

fn join_bin(reference: &Value, bins: &mut Payloads, items: &Payloads, depth: usize) -> Result<Value> {
    let id = reference.as_u64().ok_or_else(|| corrupt("invalid bin reference"))?;
    let mut bin = bins.remove(&id).ok_or_else(|| corrupt(format!("missing or repeated bin {id}")))?;
    if object_id(&bin)? != id {
        return Err(corrupt(format!("bin payload does not match id {id}")));
    }
    for child in children(&mut bin, depth)? {
        if let Some(nested) = nested_or_item(child, items)? {
            *nested = join_bin(nested, bins, items, depth + 1)?;
        }
    }
    Ok(bin)
}

impl ProjectRecords {
    /// Split supported project bytes, applying the same historical migrations as the
    /// ordinary reader. Callers reading from storage must bound input size before parsing.
    pub fn split(bytes: &[u8]) -> Result<Self> {
        let version = detect_bytes(bytes)?;
        let doc = serde_json::from_slice(bytes).map_err(|e| corrupt(e.to_string()))?;
        let mut envelope = migrate_with(MIGRATIONS, doc, version)?;
        let project = envelope.get_mut("project").and_then(Value::as_object_mut).ok_or_else(|| corrupt("missing project"))?;
        let Some(Value::Object(payloads)) = project.remove("items") else { return Err(corrupt("missing project items")) };
        let mut items = Payloads::new();
        let mut inventory = Vec::with_capacity(payloads.len());
        for (key, value) in payloads {
            let id = object_id(&value)?;
            if id.to_string() != key {
                return Err(corrupt(format!("item key does not match id {id}")));
            }
            items.insert(id, value);
            inventory.push(Value::from(id));
        }
        project.insert("items".into(), Value::Array(inventory));
        let root = project.get_mut("root").ok_or_else(|| corrupt("missing root bin"))?;
        let mut bins = Payloads::new();
        *root = Value::from(split_bin(root.take(), &mut bins, &items, 0)?);
        Ok(Self { envelope, bins, items })
    }

    /// Reassemble a complete set, refusing missing, extra or misidentified payloads.
    /// A successful result preserves the migrated document as JSON, including unknown
    /// fields; the application still validates timeline and media semantics on load.
    pub fn into_document(mut self) -> Result<Value> {
        let probe = crate::Probe::deserialize(&self.envelope).map_err(|e| corrupt(e.to_string()))?;
        check_supported(crate::probe_version(&probe)?)?;
        let project = self.envelope.get_mut("project").and_then(Value::as_object_mut).ok_or_else(|| corrupt("missing project"))?;
        let root = project.get_mut("root").ok_or_else(|| corrupt("missing root bin"))?;
        *root = join_bin(root, &mut self.bins, &self.items, 0)?;
        if !self.bins.is_empty() {
            return Err(corrupt("unreferenced bin payload"));
        }
        let Some(Value::Array(ids)) = project.remove("items") else { return Err(corrupt("missing item inventory")) };
        let mut items = Map::new();
        for key in ids {
            let id = key.as_u64().ok_or_else(|| corrupt("invalid item inventory entry"))?;
            let item = self.items.remove(&id).ok_or_else(|| corrupt(format!("missing or repeated item {id}")))?;
            if object_id(&item)? != id {
                return Err(corrupt(format!("item payload does not match id {id}")));
            }
            items.insert(id.to_string(), item);
        }
        if !self.items.is_empty() {
            return Err(corrupt("unreferenced item payload"));
        }
        project.insert("items".into(), Value::Object(items));
        Ok(self.envelope)
    }
}
