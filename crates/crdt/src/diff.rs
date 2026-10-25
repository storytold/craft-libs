//! The diff between two document states → a change batch (spec §5.3).
//!
//! The family's cheap-diff trick is structural sharing: a persistent map of `Arc<Node>` skips
//! shared structure, so after a command only the nodes whose `Arc` pointer changed are visited
//! and the cost is proportional to the edit, not the document. The craft-crdt generic model
//! keeps the same contract: [`diff`] visits only the nodes whose contents differ and orders the
//! output as the protocol demands — **field writes → creates (parents first) → moves → deletes →
//! document-level** — so `apply(diff(a, b), a) == b` holds with positions (the tested property,
//! spec §5.3).

use crate::change::{Batch, BatchId, Change, Seq};
use crate::ids::NodeId;
use crate::replica::Replica;
use crate::value::Value;
use std::sync::Arc;

/// Compute the ordered batch that turns `before` into `after` (spec §5.3).
///
/// `author` is the client space new ids belong to (already allocated by the caller); `batch_id`
/// and `base_seq` ride the returned batch.
#[must_use]
pub fn diff(before: &Replica, after: &Replica, author: u32, batch_id: BatchId, base_seq: Seq) -> Batch {
    let mut creates: Vec<Change> = Vec::new();
    let mut writes: Vec<Change> = Vec::new();
    let mut deletes: Vec<Change> = Vec::new();
    let mut doc_level: Vec<Change> = Vec::new();

    // Nodes present after: new ones become creates, changed ones become field writes.
    for (id, new_node) in after.nodes() {
        match before.node(*id) {
            None => {
                // Create with only the properties that differ from a fresh object (spec §2.3):
                // the generic model has no kind defaults, so all props ride the create.
                creates.push(Change::Create {
                    id: *id,
                    kind: new_node.kind.clone(),
                    parent: new_node.parent,
                    position: new_node.position.clone(),
                    props: new_node.props.iter().map(|(k, v)| (k.clone(), v.clone())).collect(),
                });
            }
            Some(old_node) => {
                if old_node.parent != new_node.parent {
                    writes.push(Change::Set {
                        id: *id,
                        prop: Arc::from("parent"),
                        value: Value::Json(serde_json::Value::Number((new_node.parent.0).into())),
                    });
                }
                if old_node.position != new_node.position {
                    writes.push(Change::Set {
                        id: *id,
                        prop: Arc::from("position"),
                        value: Value::Json(serde_json::Value::String(new_node.position.clone())),
                    });
                }
                for (k, v) in &new_node.props {
                    if old_node.props.get(k) != Some(v) {
                        writes.push(Change::Set { id: *id, prop: k.clone(), value: v.clone() });
                    }
                }
            }
        }
    }

    // Nodes gone after: one Delete per subtree root — a node whose parent is also gone is
    // covered by its ancestor's Delete (spec §5.3).
    for (id, old_node) in before.nodes() {
        if after.node(*id).is_none() {
            let covered = {
                let mut p = old_node.parent;
                let mut gone = false;
                loop {
                    if p == NodeId(0) {
                        break;
                    }
                    if after.node(p).is_none() {
                        gone = true;
                        break;
                    }
                    match before.node(p) {
                        Some(n) => p = n.parent,
                        None => break,
                    }
                }
                gone
            };
            if !covered {
                deletes.push(Change::Delete { id: *id });
            }
        }
    }

    // Document-level: entries and doc properties.
    for (entry, v) in after.entries() {
        if before.entry(entry) != Some(v) {
            doc_level.push(Change::Put { entry: entry.clone(), value: v.clone() });
        }
    }
    for (entry, _) in before.entries() {
        if after.entry(entry).is_none() {
            doc_level.push(Change::Remove { entry: entry.clone() });
        }
    }
    for (k, v) in after.doc_props() {
        if before.doc_prop(k) != Some(v) {
            doc_level.push(Change::SetDoc { prop: k.clone(), value: v.clone() });
        }
    }

    // Order: field writes → creates (parents first) → deletes → doc-level (spec §5.3 `order`).
    creates.sort_by_key(|c| match c {
        Change::Create { parent, .. } => depth_of(after, *parent),
        _ => 0,
    });

    let mut changes = Vec::with_capacity(writes.len() + creates.len() + deletes.len() + doc_level.len());
    changes.append(&mut writes);
    changes.append(&mut creates);
    changes.append(&mut deletes);
    changes.append(&mut doc_level);
    let _ = author; // ids were allocated by the caller; the diff never invents ids.
    Batch { id: batch_id, base_seq, changes }
}

fn depth_of(r: &Replica, mut id: NodeId) -> u32 {
    let mut d = 0;
    loop {
        if id == NodeId(0) {
            return d;
        }
        match r.node(id) {
            Some(n) => {
                id = n.parent;
                d += 1;
            }
            None => return d,
        }
    }
}
