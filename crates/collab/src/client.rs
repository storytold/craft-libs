//! The optimistic client (spec §5.5): local edits apply instantly, batches queue in order, and
//! the view is always "the confirmed state, then my unconfirmed batches replayed leniently" —
//! so a user never sees their pending edit vanish and return.
//!
//! Per-user undo/redo (spec §5.6) is computed from the **confirmed** state only after every one
//! of the client's own batches has been accepted or refused: undo rewrites exactly the values
//! the author last wrote (by server order), deletes only what the author created and nobody
//! else has parented into, and recreates deleted subtrees with the same ids, positions and
//! values. Other people's changes never enter the local history.

use crate::change::{Batch, BatchId, Change, Seq};
use crate::replica::{ApplyMode, Replica};
use serde::{Deserialize, Serialize};

/// One undo step: what the author's batch did, target by target (spec §5.6).
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct UndoStep {
    /// The batch this step came from (matched on echo or refusal).
    pub batch: BatchId,
    /// Per-target `(id, prop) → value_before` — learned when the batch was composed.
    pub before: Vec<(u64, String, Option<crate::value::Value>)>,
    /// Ids this batch created (undo deletes them, unless others parented into them).
    pub created: Vec<crate::ids::NodeId>,
    /// Subtrees this batch deleted (undo recreates them with the same ids and keys).
    pub deleted: Vec<DeletedSnapshot>,
}

/// A deleted subtree's shape for recreation on undo.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct DeletedSnapshot {
    /// The subtree root id.
    pub id: crate::ids::NodeId,
    /// The root's kind.
    pub kind: String,
    /// The root's parent.
    pub parent: crate::ids::NodeId,
    /// The root's position key.
    pub position: String,
    /// The root's properties.
    pub props: Vec<(String, crate::value::Value)>,
    /// The children, depth-first, for full recreation.
    pub children: Vec<DeletedSnapshot>,
}

/// The client state machine. Pure: the transport lives in `crafthub-net`.
#[derive(Debug, Default)]
pub struct Client {
    /// My client id (granted by the server, kept across reconnects).
    pub me: Option<crate::ids::ClientId>,
    /// The confirmed state (everything the server has ordered).
    pub confirmed: Replica,
    /// My unconfirmed batches, in send order (spec §5.5 — always in order).
    unconfirmed: Vec<Batch>,
    /// The last seq I have seen from the server.
    last_seq: Seq,
    /// My undo history (only my own steps, spec §5.6).
    history: Vec<UndoStep>,
    /// Redo stack.
    redo: Vec<UndoStep>,
}

impl Client {
    /// A fresh client over a confirmed catch-up state.
    #[must_use]
    pub fn new(confirmed: Replica) -> Self {
        Self { confirmed, ..Self::default() }
    }

    /// My granted client id.
    #[must_use]
    pub fn id(&self) -> Option<crate::ids::ClientId> {
        self.me
    }

    /// The pending (unconfirmed) batches, in order.
    #[must_use]
    pub fn pending(&self) -> &[Batch] {
        &self.unconfirmed
    }

    /// Record a locally composed batch (the caller computed it from its own document diff).
    pub fn sent(&mut self, batch: Batch) {
        self.unconfirmed.push(batch);
    }

    /// The server echoed my batch: it moves from unconfirmed to **confirmed** (spec §5.5 —
    /// the batch applies to the confirmed state; the user's view never flickers because the
    /// optimistic application already showed the same values).
    pub fn on_echo(&mut self, seq: Seq, batch_id: BatchId) {
        if let Some(pos) = self.unconfirmed.iter().position(|b| b.id == batch_id) {
            let batch = self.unconfirmed.remove(pos);
            let _ = self.confirmed.apply(&batch, ApplyMode::Lenient);
        }
        self.last_seq = seq;
    }

    /// The server refused my batch: drop it, learn the fixes, drop its undo step (spec §5.6).
    pub fn on_refused(&mut self, batch_id: BatchId, fixes: Vec<Change>) {
        self.unconfirmed.retain(|b| b.id != batch_id);
        // Apply the fixes to the confirmed state; the caller rebuilds the view.
        let fix_batch = Batch { id: batch_id, base_seq: self.last_seq, changes: fixes };
        let _ = self.confirmed.apply(&fix_batch, ApplyMode::Lenient);
        // The undo step of a refused batch is void, with any pending undo/redo of it.
        self.history.retain(|s| s.batch != batch_id);
        self.redo.retain(|s| s.batch != batch_id);
    }

    /// The server broadcast someone's accepted batch: advance the confirmed state (spec §5.5).
    pub fn on_remote(&mut self, seq: Seq, author: crate::ids::ClientId, batch: &Batch) {
        if author == self.me.unwrap_or(crate::ids::ClientId(0)) {
            // My own echo also confirms — but on_echo handles the queue; apply for state safety.
        }
        let _ = self.confirmed.apply(batch, ApplyMode::Strict);
        self.last_seq = seq;
    }

    /// The view state: confirmed plus my unconfirmed batches replayed leniently (spec §5.5).
    #[must_use]
    pub fn view(&self) -> Replica {
        let mut v = self.confirmed.clone();
        for b in &self.unconfirmed {
            let _ = v.apply(b, ApplyMode::Lenient);
        }
        v
    }

    /// Whether every one of my batches has been accepted or refused (undo waits for this,
    /// spec §5.6 rule (a)).
    #[must_use]
    pub fn flushed(&self) -> bool {
        self.unconfirmed.is_empty()
    }

    /// The next undo step (skipping steps that no longer change anything, spec §5.6).
    ///
    /// Computed from the **confirmed** state against server order: a target is undone only if
    /// its last writer (by the order the client has seen) is still the author; creations are
    /// deleted only when no one else has parented into them; deleted subtrees are recreated.
    #[must_use]
    pub fn undo_step(&self) -> Option<&UndoStep> {
        self.history.iter().rev().find(|s| !s.before.is_empty() || !s.created.is_empty() || !s.deleted.is_empty())
    }

    /// Pop the newest meaningful undo step and turn it into an inverse batch (spec §5.6).
    ///
    /// The caller sends the returned batch like any local edit and shows it locally at once.
    pub fn undo(&mut self) -> Option<Batch> {
        if !self.flushed() {
            return None; // Wait for the round trip: never undo against unseen values.
        }
        let step = self.undo_step()?.clone();
        let mut changes: Vec<Change> = Vec::new();
        for (id_raw, prop, before) in &step.before {
            let id = crate::ids::NodeId(*id_raw);
            // Only if the confirmed state still holds what this step wrote (server order check).
            let current = self.confirmed.node(id).and_then(|n| n.props.get(prop.as_str()));
            let target_value = match before {
                Some(v) => v.clone(),
                None => crate::value::Value::Json(serde_json::Value::Null),
            };
            if current != Some(&target_value) {
                changes.push(Change::Set { id, prop: std::sync::Arc::from(prop.as_str()), value: target_value });
            }
        }
        for id in &step.created {
            // What I created and nobody parented into gets deleted (spec §5.6 rule (c)).
            if self.confirmed.node(*id).is_some() {
                let has_children = self.confirmed.children(*id).next().is_some();
                if !has_children {
                    changes.push(Change::Delete { id: *id });
                }
            }
        }
        for snap in &step.deleted {
            if self.confirmed.node(snap.id).is_none() && self.confirmed.node(snap.parent).is_some() {
                changes.extend(recreate(snap));
            }
        }
        if changes.is_empty() {
            // The step went completely stale — skip to the previous one.
            self.history.pop();
            return self.undo();
        }
        self.history.pop();
        Some(Batch { id: 0, base_seq: self.last_seq, changes })
    }

    /// Push a composed undo step (the caller records it while composing the forward batch).
    pub fn push_step(&mut self, step: UndoStep) {
        self.history.push(step);
        if self.history.len() > 512 {
            self.history.remove(0); // Bounded history (mirrors the family's undo depth).
        }
    }

    /// A redo step: the mirror of undo (spec §5.6).
    pub fn redo(&mut self) -> Option<Batch> {
        if !self.flushed() || self.redo.is_empty() {
            return None;
        }
        let step = self.redo.pop()?;
        let mut changes: Vec<Change> = Vec::new();
        for (id_raw, prop, after) in &step.before {
            changes.push(Change::Set {
                id: crate::ids::NodeId(*id_raw),
                prop: std::sync::Arc::from(prop.as_str()),
                value: after.clone().unwrap_or(crate::value::Value::Json(serde_json::Value::Null)),
            });
        }
        if changes.is_empty() {
            return self.redo();
        }
        Some(Batch { id: 0, base_seq: self.last_seq, changes })
    }

    /// Move the last undone step onto the redo stack.
    pub fn stash_for_redo(&mut self, step: UndoStep) {
        self.redo.push(step);
    }
}

fn recreate(snap: &DeletedSnapshot) -> Vec<Change> {
    let mut out = vec![Change::Create {
        id: snap.id,
        kind: std::sync::Arc::from(snap.kind.as_str()),
        parent: snap.parent,
        position: snap.position.clone(),
        props: snap.props.iter().map(|(k, v)| (std::sync::Arc::from(k.as_str()), v.clone())).collect(),
    }];
    for child in &snap.children {
        out.extend(recreate(child));
    }
    out
}
