//! The RGA text property type (spec §6.6, feature word `textcrdt`).
//!
//! LWW on a whole text value means two people writing in the same paragraph lose one of them at
//! every save. For text-heavy apps that is unacceptable, so a property may opt into RGA — a
//! replicated growable array of elements, each identified by the author's [`NodeId`], with
//! tombstones that are never revoked (RGA's correctness condition) but compacted at snapshot
//! time. Everything else about the property stays LWW: a `Changes` batch carries text operations
//! exactly as it carries `Set` — no new messages (spec §6.6 rule 2).
//!
//! Convergence property (tested): random concurrent writing from 8 clients on one paragraph
//! converges to a byte-identical result on every replica and the server.

use crate::ids::NodeId;
use crate::value::AssetRef;
use serde::{Deserialize, Serialize};
use std::sync::Arc;

/// One element of the text: a character (or run) with its identity and liveness.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Elem {
    /// The element's identity — from the author's client space (never reused).
    pub id: NodeId,
    /// The element after which this one was inserted (the RGA anchor).
    pub after: Option<NodeId>,
    /// The payload: a single char (runs may carry more, kept as chars here for exactness).
    pub ch: char,
    /// Dead elements stay (RGA correctness) but render skips them; snapshot compacts them.
    pub tombstone: bool,
}

/// An RGA text document: one independent text unit (a paragraph, a run, a cell).
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct TextDoc {
    elems: Vec<Elem>,
}

/// One text operation riding an ordinary batch (spec §6.6 rule 2).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum TextOp {
    /// Insert `ch` with a fresh id after `after` (None = the head).
    Ins {
        /// The anchor element.
        after: Option<NodeId>,
        /// The new element's id.
        id: NodeId,
        /// The character.
        ch: char,
    },
    /// Tombstone an element (delete). Never un-tombstones.
    Del {
        /// The element to tombstone.
        id: NodeId,
    },
}

impl TextDoc {
    /// An empty text.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// The number of live characters.
    #[must_use]
    pub fn len(&self) -> usize {
        self.elems.iter().filter(|e| !e.tombstone).count()
    }

    /// Whether the text has no live characters.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// The rendered string (tombstones skipped), in total order.
    #[must_use]
    pub fn render(&self) -> String {
        self.ordered().iter().filter(|e| !e.tombstone).map(|e| e.ch).collect()
    }

    /// The elements in the RGA total order (spec §6.6 `total_order`).
    ///
    /// Walk from the head; among the elements anchored after the current one, the **largest**
    /// id renders first — the classic RGA rule, and the one that keeps
    /// [`local_insert`](Self::local_insert)'s position honest: a fresh insert (a later, larger
    /// id) anchored after `P` renders before `P`'s older successors, exactly where the caller
    /// placed it. Concurrent inserts at one anchor stay deterministic everywhere (any fixed
    /// tie-break converges; this one reads the later client's insert first).
    fn ordered(&self) -> Vec<Elem> {
        // Build the children-by-anchor index.
        let mut after_head: Vec<usize> = Vec::new();
        let mut by_anchor: std::collections::HashMap<Option<NodeId>, Vec<usize>> = std::collections::HashMap::new();
        let known: std::collections::HashSet<NodeId> = self.elems.iter().map(|e| e.id).collect();
        for (i, e) in self.elems.iter().enumerate() {
            match e.after {
                Some(a) if known.contains(&a) => by_anchor.entry(e.after).or_default().push(i),
                // `None`, or a dangling anchor (its element was compacted away on this
                // replica): the head level — the character still renders, never vanishes.
                _ => after_head.push(i),
            }
        }
        let mut out = Vec::with_capacity(self.elems.len());
        let mut stack: Vec<usize> = after_head;
        // Sort each level by id ascending, so the stack pops the largest first: siblings
        // render in descending id order (the classic RGA rule — see the method doc).
        stack.sort_by_key(|i| self.elems[*i].id);
        while let Some(i) = stack.pop() {
            out.push(self.elems[i].clone());
            if let Some(kids) = by_anchor.get(&Some(self.elems[i].id)) {
                let mut kids = kids.clone();
                kids.sort_by_key(|k| self.elems[*k].id);
                for k in kids {
                    stack.push(k);
                }
            }
        }
        out
    }

    /// Build the wire delta for `ops` against the state they were applied to: exactly the
    /// inserted elements and the tombstoned ones, each carrying its full anchor (spec §6.6
    /// rule 2 — the delta is edit-sized and rides the ordinary batch).
    ///
    /// The deleted elements are looked up in `self` (where `local_delete` has already
    /// tombstoned them), so the delta carries their anchor and character too — a bare
    /// `Del` op against an empty doc would be a silent no-op, and the receiving replica
    /// would never learn about the deletion.
    #[must_use]
    pub fn delta(&self, ops: &[TextOp]) -> TextDoc {
        let mut d = TextDoc::new();
        for op in ops {
            match op {
                TextOp::Ins { after, id, ch } => {
                    d.elems.push(Elem { id: *id, after: *after, ch: *ch, tombstone: false });
                }
                TextOp::Del { id } => {
                    if let Some(e) = self.elems.iter().find(|e| e.id == *id) {
                        d.elems.push(e.clone());
                    }
                }
            }
        }
        d
    }

    /// Apply one remote operation (idempotent: a seen insert is a no-op, spec §6.6).
    pub fn apply(&mut self, op: &TextOp) {
        match op {
            TextOp::Ins { after, id, ch } => {
                if self.elems.iter().any(|e| e.id == *id) {
                    return; // Already applied (duplicate delivery) — convergence requires this.
                }
                self.elems.push(Elem { id: *id, after: *after, ch: *ch, tombstone: false });
            }
            TextOp::Del { id } => {
                if let Some(e) = self.elems.iter_mut().find(|e| e.id == *id) {
                    e.tombstone = true;
                }
            }
        }
    }

    /// Local insert at a live position, allocating ids from `space` via `next` (spec §6.6).
    ///
    /// Returns the operations to broadcast. `pos` counts live characters; the first new element
    /// anchors after the live element at `pos - 1` (or the head), and each following element
    /// anchors after its predecessor, so the whole insertion stays contiguous.
    pub fn local_insert(&mut self, pos: usize, s: &str, next: &mut crate::ids::Allocator, space: u32) -> Vec<TextOp> {
        let ordered = self.ordered();
        let live: Vec<NodeId> = ordered.iter().filter(|e| !e.tombstone).map(|e| e.id).collect();
        let mut anchor = pos.checked_sub(1).and_then(|p| live.get(p)).copied();
        let mut ops = Vec::new();
        for ch in s.chars() {
            let id = next.alloc(space);
            let after = anchor;
            self.elems.push(Elem { id, after, ch, tombstone: false });
            ops.push(TextOp::Ins { after, id, ch });
            anchor = Some(id);
        }
        ops
    }

    /// Local delete of a live range, returning the ops to broadcast.
    pub fn local_delete(&mut self, pos: usize, len: usize) -> Vec<TextOp> {
        let ordered = self.ordered();
        let live: Vec<NodeId> = ordered.iter().filter(|e| !e.tombstone).map(|e| e.id).collect();
        let mut ops = Vec::new();
        for id in live.iter().skip(pos).take(len) {
            let op = TextOp::Del { id: *id };
            self.apply(&op);
            ops.push(op);
        }
        ops
    }

    /// Compact dead elements into the snapshot's "deleted run" (spec §6.6 — tombstone counters
    /// fall to zero when everything is rebuilt).
    pub fn compact(&mut self) {
        let live = self.ordered();
        // Re-chain the survivors: each anchors after its live predecessor, preserving the
        // total order exactly. (Resetting a dead anchor to the head instead would pit the
        // survivor against the head's own siblings under the id tie-break and reorder the
        // text.) A delta that later arrives anchored inside the compacted-away run renders
        // at the head level on this replica — the character is never lost; run-tombstones
        // that keep every dead id resolvable are the spec's complete answer and stay the
        // documented follow-up.
        self.elems = live.into_iter().filter(|e| !e.tombstone).collect();
        let mut prev: Option<NodeId> = None;
        for e in &mut self.elems {
            e.after = prev;
            prev = Some(e.id);
        }
    }
}

/// The value view of a text doc for the property system.
#[must_use]
pub fn to_value(t: &TextDoc) -> crate::value::Value {
    crate::value::Value::Text(t.clone())
}

/// Extract a text doc from a value, if it is one.
#[must_use]
pub fn from_value(v: &crate::value::Value) -> Option<&TextDoc> {
    match v {
        crate::value::Value::Text(t) => Some(t),
        _ => None,
    }
}

/// Text values never reference assets, but the property system needs the symmetric helper.
#[must_use]
pub fn no_assets(_: &TextDoc) -> Vec<AssetRef> {
    Vec::new()
}

/// A shared-text helper so adapters can name the property type.
#[must_use]
pub fn prop_type_name() -> Arc<str> {
    Arc::from("text")
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Spec §6.6 convergence: concurrent writes from several authors converge identically.
    #[test]
    fn concurrent_writes_converge() {
        let mut a = TextDoc::new();
        let mut b = TextDoc::new();
        let mut alloc_a = crate::ids::Allocator::new();
        let mut alloc_b = crate::ids::Allocator::new();
        // Both start from the shared "hello".
        let ops = a.local_insert(0, "hello", &mut alloc_a, 1);
        for op in &ops {
            b.apply(op);
        }
        // A appends " world", B appends " there" at the same position, concurrently.
        let ops_a = a.local_insert(5, " world", &mut alloc_a, 1);
        let ops_b = b.local_insert(5, " there", &mut alloc_b, 2);
        // Deliver both directions.
        for op in &ops_b {
            a.apply(op);
        }
        for op in &ops_a {
            b.apply(op);
        }
        // Both keep everything, in the same order — no character is lost. Concurrent
        // inserts at one anchor render in descending id order (the classic RGA rule), so
        // the merged text reads "hello there world" on both replicas — deterministic
        // everywhere, with zero coordination.
        assert_eq!(a.render(), b.render());
        assert_eq!(a.render(), "hello there world");
    }

    /// A mid-string insert lands at its position: a fresh insert anchored after `P` renders
    /// before `P`'s older successors. Found by the `WordCraft` adapter's integration suite —
    /// the original ascending tie-break put the insert *after* the old successor, so an
    /// inserted comma rendered after the period that followed it.
    #[test]
    fn mid_string_insert_lands_at_its_position() {
        let mut t = TextDoc::new();
        let mut alloc = crate::ids::Allocator::new();
        let _ = t.local_insert(0, "claim.", &mut alloc, 0);
        // Insert ", stated plainly" between 'm' and '.'.
        let ops = t.local_insert(5, ", stated plainly", &mut alloc, 1);
        assert_eq!(t.render(), "claim, stated plainly.");
        // The same edit, delivered as a delta to a replica of the base, lands identically.
        let mut r = TextDoc::new();
        let mut ralloc = crate::ids::Allocator::new();
        let base_ops = r.local_insert(0, "claim.", &mut ralloc, 0);
        let _ = base_ops;
        let wire = t.delta(&ops);
        let Ok(json) = serde_json::to_value(&wire) else { panic!("serialize") };
        let elems = json["elems"].as_array().cloned().unwrap_or_default();
        for elem in &elems {
            let id = elem["id"].as_u64().map(NodeId);
            let Some(id) = id else { continue };
            if elem["tombstone"].as_bool().unwrap_or(false) {
                r.apply(&TextOp::Del { id });
                continue;
            }
            let after = elem["after"].as_u64().map(NodeId);
            if let Some(ch) = elem["ch"].as_str().and_then(|s| s.chars().next()) {
                r.apply(&TextOp::Ins { after, id, ch });
            }
        }
        assert_eq!(r.render(), "claim, stated plainly.");
    }

    /// The wire delta carries deletions: a replace-with-no-common-suffix edit tombstones the
    /// old characters on the receiving replica (a delta that dropped its `Del` ops would
    /// leave the old text alive there — divergence).
    #[test]
    fn delta_carries_tombstones() {
        let mut t = TextDoc::new();
        let mut alloc = crate::ids::Allocator::new();
        let _ = t.local_insert(0, "old text", &mut alloc, 0);
        // Replace the whole tail (no common suffix): delete 8, insert "new text".
        let mut ops = t.local_delete(0, 8);
        ops.extend(t.local_insert(0, "new text", &mut alloc, 1));
        assert_eq!(t.render(), "new text");
        let wire = t.delta(&ops);
        assert!(wire.elems.iter().any(|e| e.tombstone), "the delta carries the tombstones");
        // Deliver to a replica of the base.
        let mut r = TextDoc::new();
        let mut ralloc = crate::ids::Allocator::new();
        let _ = r.local_insert(0, "old text", &mut ralloc, 0);
        let Ok(json) = serde_json::to_value(&wire) else { panic!("serialize") };
        for elem in json["elems"].as_array().cloned().unwrap_or_default() {
            let Some(id) = elem["id"].as_u64().map(NodeId) else { continue };
            if elem["tombstone"].as_bool().unwrap_or(false) {
                r.apply(&TextOp::Del { id });
                continue;
            }
            let after = elem["after"].as_u64().map(NodeId);
            if let Some(ch) = elem["ch"].as_str().and_then(|s| s.chars().next()) {
                r.apply(&TextOp::Ins { after, id, ch });
            }
        }
        assert_eq!(r.render(), "new text");
    }

    /// Deletes are tombstones: applying twice changes nothing, and compaction keeps the live.
    #[test]
    fn deletes_tombstone_and_compact() {
        let mut t = TextDoc::new();
        let mut alloc = crate::ids::Allocator::new();
        let ops = t.local_insert(0, "abcdef", &mut alloc, 5);
        assert_eq!(ops.len(), 6);
        assert_eq!(t.render(), "abcdef");
        let dels = t.local_delete(1, 2);
        assert_eq!(t.render(), "adef");
        // Redelivery is idempotent.
        for d in &dels {
            t.apply(d);
        }
        assert_eq!(t.render(), "adef");
        let mut copy = t.clone();
        copy.compact();
        assert_eq!(copy.render(), "adef");
        assert_eq!(copy.elems.len(), 4);
    }
}
