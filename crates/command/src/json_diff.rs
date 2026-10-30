// Copyright (c) 2026 ArtCraft Team and the EffectCraft contributors
// SPDX-License-Identifier: MIT OR Apache-2.0
//
// Adapted from EffectCraft `diff`, `diff_into`, `patch`, `Op`, and `Step`
// (`crates/engine/src/remote.rs`). Those sources have no per-file license
// header. The copyright and the MIT OR Apache-2.0 terms live in that
// repository's LICENSE-MIT and LICENSE-APACHE. See ATTRIBUTION.md.

//! Structural JSON diff and patch.
//!
//! Object members are added, changed, or removed. Arrays of equal length are
//! compared element by element. Arrays of different length are replaced.
//! `null` is a value, so it is not used as a deletion mark.

use serde::Deserialize;
use serde::Serialize;
use serde_json::Value;

/// One step of a path into a JSON document.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Step {
    /// An object member name.
    Key(String),
    /// An array index.
    Index(usize),
}

/// One change. `value: Some` sets the value at `path`. `value: None` removes
/// the object member at `path`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Op {
    /// Path from the document root. Empty replaces the whole document.
    pub path: Vec<Step>,
    /// New value, or `None` to remove an object member.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub value: Option<Value>,
}

/// The op list that turns one JSON value into another.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Patch {
    ops: Vec<Op>,
}

impl Patch {
    /// A patch from an op list. Used by tests and by callers that build ops.
    pub fn new(ops: Vec<Op>) -> Self {
        Self { ops }
    }

    /// The ops, in apply order.
    pub fn ops(&self) -> &[Op] {
        &self.ops
    }

    /// True when `a` and `b` were equal.
    pub fn is_empty(&self) -> bool {
        self.ops.is_empty()
    }
}

/// Why `patch` could not apply an op.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PatchError(String);

impl PatchError {
    pub(crate) fn new(message: impl Into<String>) -> Self {
        Self(message.into())
    }

    /// The error text. The strings match EffectCraft's `patch`.
    pub fn message(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for PatchError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for PatchError {}

/// The changes that turn `old` into `new`. Empty when the values are equal.
pub fn diff(old: &Value, new: &Value) -> Patch {
    let mut ops = Vec::new();
    diff_into(old, new, &mut Vec::new(), &mut ops);
    Patch { ops }
}

fn diff_into(old: &Value, new: &Value, path: &mut Vec<Step>, ops: &mut Vec<Op>) {
    if old == new {
        return;
    }
    match (old, new) {
        (Value::Object(a), Value::Object(b)) => {
            for (k, va) in a {
                path.push(Step::Key(k.clone()));
                match b.get(k) {
                    Some(vb) => diff_into(va, vb, path, ops),
                    None => ops.push(Op { path: path.clone(), value: None }),
                }
                path.pop();
            }
            for (k, vb) in b {
                if !a.contains_key(k) {
                    path.push(Step::Key(k.clone()));
                    ops.push(Op { path: path.clone(), value: Some(vb.clone()) });
                    path.pop();
                }
            }
        }
        (Value::Array(a), Value::Array(b)) if a.len() == b.len() => {
            for (i, (va, vb)) in a.iter().zip(b).enumerate() {
                path.push(Step::Index(i));
                diff_into(va, vb, path, ops);
                path.pop();
            }
        }
        _ => ops.push(Op { path: path.clone(), value: Some(new.clone()) }),
    }
}

/// Apply `changes` to `doc`.
pub fn patch(doc: &mut Value, changes: &Patch) -> Result<(), PatchError> {
    for op in &changes.ops {
        let Some((last, parents)) = op.path.split_last() else {
            *doc = op.value.clone().ok_or_else(|| PatchError::new("cannot remove the document"))?;
            continue;
        };
        let mut cur = &mut *doc;
        for step in parents {
            cur = match (step, cur) {
                (Step::Key(k), Value::Object(map)) => map.get_mut(k).ok_or_else(|| PatchError::new(format!("no member `{k}`")))?,
                (Step::Index(i), Value::Array(items)) => items.get_mut(*i).ok_or_else(|| PatchError::new(format!("no element {i}")))?,
                _ => return Err(PatchError::new("path does not match the document")),
            };
        }
        match (last, cur, &op.value) {
            (Step::Key(k), Value::Object(map), Some(value)) => {
                map.insert(k.clone(), value.clone());
            }
            (Step::Key(k), Value::Object(map), None) => {
                map.remove(k);
            }
            (Step::Index(i), Value::Array(items), Some(value)) => {
                let item = items.get_mut(*i).ok_or_else(|| PatchError::new(format!("no element {i}")))?;
                *item = value.clone();
            }
            _ => return Err(PatchError::new("path does not match the document")),
        }
    }
    Ok(())
}
