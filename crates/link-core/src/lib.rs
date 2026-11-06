//! Transport-neutral semantic link contracts and optional durable native storage.
//! Applications keep native engines; committed heads are projected asynchronously.
//! See the crate README for the experimental contract and consistency boundaries.
#![forbid(unsafe_code)]
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};
use uuid::Uuid;

pub const SCHEMA: u32 = 1;
pub type Id = Uuid;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("conflict: {0}")]
    Conflict(String),
    #[error("capability: {0}")]
    Capability(String),
    #[error("invalid: {0}")]
    Invalid(String),
    #[error("missing: {0}")]
    Missing(String),
    #[error("storage: {0}")]
    Storage(String),
}
pub type Result<T> = std::result::Result<T, Error>;

/// Immutable native bytes with their origin; never assume a receiver snapshot
/// uses the canonical object's owning application's format.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct NativePayload {
    pub hash: String,
    pub format: String,
    pub application: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Manifest {
    pub schema: u32,
    pub application: String,
    pub version: String,
    pub object_types: BTreeMap<String, BTreeSet<String>>,
    pub representations: Vec<String>,
    pub read: bool,
    pub write: bool,
    pub history: String,
    pub transactions: String,
    pub lifecycle: String,
    pub authentication: String,
    pub filesystem: String,
    pub fidelity: String,
}
impl Manifest {
    pub fn validate(&self) -> Result<()> {
        if self.schema != SCHEMA || self.application.is_empty() || self.application.len() > 128 {
            return Err(Error::Capability("unsupported manifest schema or identity".into()));
        }
        if self.object_types.is_empty() || !self.read {
            return Err(Error::Capability("readable object types required".into()));
        }
        Ok(())
    }
    pub fn negotiate(&self, kind: &str, properties: &[String], write: bool) -> Result<()> {
        self.validate()?;
        let allowed = self.object_types.get(kind).ok_or_else(|| Error::Capability(kind.into()))?;
        if (write && !self.write) || properties.iter().any(|p| !allowed.contains(p)) {
            return Err(Error::Capability("unsupported property or permission".into()));
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Change {
    pub object: Id,
    pub document: Id,
    pub application: String,
    pub kind: String,
    pub expected: u64,
    pub value: Value,
    pub native: Option<NativePayload>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Transaction {
    pub operation: Id,
    pub changes: Vec<Change>,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Object {
    pub object: Id,
    pub document: Id,
    pub application: String,
    pub kind: String,
    pub revision: u64,
    pub value: Value,
    pub native: Option<NativePayload>,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Receipt {
    pub operation: Id,
    pub sequence: u64,
    pub revisions: BTreeMap<Id, u64>,
    pub invalidated: BTreeSet<Id>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Event {
    pub sequence: u64,
    pub kind: String,
    pub data: Value,
}

/// Edges point upstream -> dependent. Editing back uses the same canonical object,
/// never a reverse edge that would introduce a synchronization cycle.
pub fn affected(edges: &[(Id, Id)], source: Id) -> BTreeSet<Id> {
    affected_many(edges, std::iter::once(source))
}
/// Traverse the union of changed roots once, with one adjacency index. This
/// avoids rescanning all edges for every visited node while holding a writer lock.
pub fn affected_many(edges: &[(Id, Id)], sources: impl IntoIterator<Item = Id>) -> BTreeSet<Id> {
    let mut adjacency: BTreeMap<Id, Vec<Id>> = BTreeMap::new();
    for (source, target) in edges {
        adjacency.entry(*source).or_default().push(*target);
    }
    let mut found = BTreeSet::new();
    let mut visited = BTreeSet::new();
    let mut pending = sources.into_iter().collect::<Vec<_>>();
    while let Some(id) = pending.pop() {
        if !visited.insert(id) {
            continue;
        }
        for target in adjacency.get(&id).into_iter().flatten() {
            if found.insert(*target) {
                pending.push(*target);
            }
        }
    }
    found
}
pub fn validate_edge(edges: &[(Id, Id)], source: Id, target: Id) -> Result<()> {
    if source == target || affected(edges, target).contains(&source) {
        return Err(Error::Invalid("dependency cycle".into()));
    }
    Ok(())
}

#[cfg(feature = "native")]
mod store;
#[cfg(feature = "native")]
pub use store::Store;
