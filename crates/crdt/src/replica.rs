//! The synchronized state: the replica every client and the server keep (spec §2.3, §4.6).
//!
//! A replica is the whole synchronized truth of one document: a flat map of nodes (each with its
//! properties, parent and position key), the children index per parent ordered by `(key, id)`,
//! and the document-level entries. Applying a change batch is either *strict* (the server stops
//! at the first invalid change) or *lenient* (a client replaying its pending batches skips what
//! no longer applies). The convergence guarantee — `apply(diff(a, b), a) == b` — is tested over
//! randomly generated real edits (spec §5.3).

use crate::Error;
use crate::change::{Batch, Change, EntryId, Position, Seq};
use crate::frac;
use crate::ids::NodeId;
use crate::value::Value;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

/// One synchronized object.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Node {
    /// The object's kind (adapter-defined, e.g. `"frame"`).
    pub kind: Arc<str>,
    /// The parent object id (the tree is two properties on the child — spec §2.3).
    pub parent: NodeId,
    /// The fractional position among siblings.
    pub position: Position,
    /// The object's properties.
    pub props: BTreeMap<Arc<str>, Value>,
}

/// How [`Replica::apply`] treats changes that no longer fit.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ApplyMode {
    /// Stop at the first invalid change with an error (the server's stance — spec §5.4).
    Strict,
    /// Skip what no longer applies (a client replaying its pending batches — spec §5.5).
    Lenient,
}

/// The whole synchronized document state.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Replica {
    /// The nodes, by id.
    nodes: BTreeMap<NodeId, Node>,
    /// Children per parent, kept ordered by `(position, id)`.
    children: BTreeMap<NodeId, BTreeSet<(Position, NodeId)>>,
    /// Document-level entries (styles, variables, comments, versions...).
    entries: BTreeMap<EntryId, Value>,
    /// Document-level properties.
    doc: BTreeMap<Arc<str>, Value>,
    /// Ids removed during this session's lifetime (accepted back by re-Create with same id).
    /// A set, not a hash table: postcard must round-trip byte-for-byte in [`crate::SessionState::decode`].
    removed: BTreeSet<NodeId>,
    /// The highest server seq this replica has applied.
    seq: Seq,
}

impl Replica {
    /// An empty replica at seq 0.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// The highest applied server seq.
    #[must_use]
    pub fn seq(&self) -> Seq {
        self.seq
    }

    /// Set the applied seq (used when installing a catch-up snapshot).
    pub fn set_seq(&mut self, seq: Seq) {
        self.seq = seq;
    }

    /// Get a node.
    #[must_use]
    pub fn node(&self, id: NodeId) -> Option<&Node> {
        self.nodes.get(&id)
    }

    /// All nodes, by id.
    pub fn nodes(&self) -> impl Iterator<Item = (&NodeId, &Node)> {
        self.nodes.iter()
    }

    /// The node count (checked against `Limits::max_nodes`).
    #[must_use]
    pub fn node_count(&self) -> usize {
        self.nodes.len()
    }

    /// A parent's children in `(position, id)` order.
    pub fn children(&self, parent: NodeId) -> impl Iterator<Item = NodeId> + '_ {
        self.children.get(&parent).into_iter().flat_map(|m| m.iter().map(|(_, id)| *id))
    }

    /// A document-level entry.
    #[must_use]
    pub fn entry(&self, id: &EntryId) -> Option<&Value> {
        self.entries.get(id)
    }

    /// All document-level entries.
    pub fn entries(&self) -> impl Iterator<Item = (&EntryId, &Value)> {
        self.entries.iter()
    }

    /// A document-level property.
    #[must_use]
    pub fn doc_prop(&self, name: &str) -> Option<&Value> {
        self.doc.get(name)
    }

    /// All document-level properties.
    pub fn doc_props(&self) -> impl Iterator<Item = (&Arc<str>, &Value)> {
        self.doc.iter()
    }

    /// Whether an id was removed earlier in the session (its re-Create is accepted, spec §2.2).
    #[must_use]
    pub fn was_removed(&self, id: NodeId) -> bool {
        self.removed.contains(&id)
    }

    /// Create a node (parents-first ordering is the batch's responsibility; here we validate).
    pub fn create(&mut self, id: NodeId, kind: Arc<str>, parent: NodeId, position: Position, props: Vec<(Arc<str>, Value)>) -> Result<()> {
        if self.nodes.contains_key(&id) && !self.removed.contains(&id) {
            return Err(Error::Invalid(format!("object {id:?} already exists")));
        }
        if !self.nodes.contains_key(&parent) && parent != NodeId(0) {
            return Err(Error::Invalid(format!("parent {parent:?} does not exist")));
        }
        frac::validate(&position).map_err(|e| Error::Invalid(e.to_string()))?;
        let node = Node { kind, parent, position: position.clone(), props: props.into_iter().collect() };
        self.children.entry(parent).or_default().insert((position, id));
        self.nodes.insert(id, node);
        self.removed.remove(&id);
        Ok(())
    }

    /// Set one property (a `Set` with prop `parent` moves the node; prop `position` re-orders it).
    pub fn set(&mut self, id: NodeId, prop: &str, value: Value) -> Result<()> {
        let Some(node) = self.nodes.get_mut(&id) else {
            return Err(Error::Unknown(id));
        };
        match prop {
            "parent" => {
                let Value::Json(serde_json::Value::Number(n)) = &value else {
                    return Err(Error::Invalid("parent must be a number".into()));
                };
                let Some(raw) = n.as_u64() else {
                    return Err(Error::Invalid("parent must be a u64 id".into()));
                };
                let new_parent = NodeId(raw);
                if new_parent != NodeId(0) && !self.nodes.contains_key(&new_parent) {
                    return Err(Error::Invalid(format!("parent {new_parent:?} does not exist")));
                }
                if self.would_cycle(id, new_parent) {
                    return Err(Error::Invalid("the move would create a cycle".into()));
                }
                // Read what we need, then re-borrow: the cycle check above borrows self immutably.
                let (old_parent, pos) = {
                    let node = self.nodes.get(&id).ok_or(Error::Unknown(id))?;
                    (node.parent, node.position.clone())
                };
                if let Some(m) = self.children.get_mut(&old_parent) {
                    m.remove(&(pos.clone(), id));
                    if m.is_empty() {
                        self.children.remove(&old_parent);
                    }
                }
                self.children.entry(new_parent).or_default().insert((pos, id));
                if let Some(node) = self.nodes.get_mut(&id) {
                    node.parent = new_parent;
                }
            }
            "position" => {
                let Value::Json(serde_json::Value::String(p)) = &value else {
                    return Err(Error::Invalid("position must be a string key".into()));
                };
                frac::validate(p).map_err(|e| Error::Invalid(e.to_string()))?;
                let (old_parent, _) = {
                    let node = self.nodes.get(&id).ok_or(Error::Unknown(id))?;
                    (node.parent, node.position.clone())
                };
                let new_pos = p.clone();
                if let Some(m) = self.children.get_mut(&old_parent) {
                    let old_key = m.iter().find(|(_, nid)| *nid == id).cloned();
                    if let Some((old_pos, _)) = old_key {
                        m.remove(&(old_pos, id));
                        if m.is_empty() {
                            self.children.remove(&old_parent);
                        }
                    }
                }
                self.children.entry(old_parent).or_default().insert((new_pos.clone(), id));
                if let Some(node) = self.nodes.get_mut(&id) {
                    node.position = new_pos;
                }
            }
            _ => {
                node.props.insert(Arc::from(prop), value);
            }
        }
        Ok(())
    }

    /// Delete a subtree root (idempotent for already-deleted ids).
    pub fn delete(&mut self, id: NodeId) {
        // Collect the subtree; deleting a deleted id is a no-op (spec §2.3).
        if !self.nodes.contains_key(&id) {
            return;
        }
        let mut stack = vec![id];
        while let Some(cur) = stack.pop() {
            if let Some(node) = self.nodes.remove(&cur) {
                if let Some(m) = self.children.get_mut(&node.parent) {
                    m.remove(&(node.position.clone(), cur));
                    if m.is_empty() {
                        self.children.remove(&node.parent);
                    }
                }
                if let Some(kids) = self.children.remove(&cur) {
                    stack.extend(kids.iter().map(|(_, cid)| *cid));
                }
            }
        }
        self.removed.insert(id);
    }

    /// Put/replace a document-level entry.
    pub fn put(&mut self, entry: EntryId, value: Value) {
        self.entries.insert(entry, value);
    }

    /// Remove a document-level entry (a no-op if absent).
    pub fn remove_entry(&mut self, entry: &EntryId) {
        self.entries.remove(entry);
    }

    /// Set a document-level property.
    pub fn set_doc(&mut self, prop: &str, value: Value) {
        self.doc.insert(Arc::from(prop), value);
    }

    /// Apply one change in the given mode. Returns whether it applied.
    fn apply_one(&mut self, change: &Change, mode: ApplyMode) -> Result<bool> {
        let strict = |e: Error| if mode == ApplyMode::Strict { Err(e) } else { Ok(false) };
        match change {
            Change::Create { id, kind, parent, position, props } => {
                if self.nodes.contains_key(id) && !self.removed.contains(id) {
                    return strict(Error::Invalid(format!("object {id:?} already exists")));
                }
                match self.create(*id, kind.clone(), *parent, position.clone(), props.clone()) {
                    Ok(()) => Ok(true),
                    Err(e) => strict(e).map(|_| false),
                }
            }
            Change::Set { id, prop, value } => match self.set(*id, prop, value.clone()) {
                Ok(()) => Ok(true),
                Err(e) => strict(e).map(|_| false),
            },
            Change::Delete { id } => {
                self.delete(*id);
                Ok(true)
            }
            Change::SetDoc { prop, value } => {
                self.set_doc(prop, value.clone());
                Ok(true)
            }
            Change::Put { entry, value } => {
                self.put(entry.clone(), value.clone());
                Ok(true)
            }
            Change::Remove { entry } => {
                self.remove_entry(entry);
                Ok(true)
            }
        }
    }

    /// Apply a batch. Strict mode stops at the first invalid change; lenient skips (spec §5.4/§5.5).
    pub fn apply(&mut self, batch: &Batch, mode: ApplyMode) -> Result<()> {
        for c in &batch.changes {
            self.apply_one(c, mode)?;
        }
        if batch.base_seq >= self.seq {
            self.seq = batch.base_seq;
        }
        Ok(())
    }

    /// Would making `id` a child of `parent` create a cycle?
    fn would_cycle(&self, id: NodeId, parent: NodeId) -> bool {
        if id == parent {
            return true;
        }
        let mut cur = parent;
        loop {
            if cur == id {
                return true;
            }
            match self.nodes.get(&cur) {
                Some(n) => cur = n.parent,
                None => return false,
            }
        }
    }

    /// Re-space a parent's children whose keys grew too long (spec §5.2).
    ///
    /// Returns the changes to broadcast (position writes ride an ordinary batch).
    pub fn respace_children(&mut self, parent: NodeId, author_space: u32, batch_id: crate::change::BatchId, base_seq: Seq) -> Option<Batch> {
        let keys: Vec<(String, u64)> = self.children(parent).filter_map(|id| self.node(id).map(|n| (n.position.clone(), id.0))).collect();
        let pairs = frac::respace(&keys).ok()?;
        if pairs.is_empty() {
            return None;
        }
        let changes = pairs
            .into_iter()
            .filter_map(|(old, new)| {
                let id = self.children(parent).find(|cid| self.node(*cid).is_some_and(|n| n.position == old))?;
                Some(Change::Set { id, prop: Arc::from("position"), value: Value::Json(serde_json::Value::String(new)) })
            })
            .collect::<Vec<_>>();
        if changes.is_empty() {
            return None;
        }
        let _ = author_space;
        Some(Batch { id: batch_id, base_seq, changes })
    }
}

/// A [`Result`] alias for the whole crate.
pub type Result<T, E = Error> = std::result::Result<T, E>;
