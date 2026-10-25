//! The change algebra (spec §2.3): everything that syncs is a change to an object's properties.
//!
//! Tree structure is two properties on the child: `parent` (an object id) and `position` (a
//! fractional key), so two clients inserting into the same frame never conflict and a move is a
//! single property write. Document-level data (styles, variables, collections, modes, comments,
//! versions) are *entries* synced the same way. Batches are atomic: one user action is one batch
//! and one undo step (spec §2.3, §5.4).

use crate::ids::NodeId;
use crate::value::Value;
use serde::{Deserialize, Serialize};
use std::sync::Arc;

/// A document-level entry id (spec §2.2): styles, variables, collections, modes, comment threads.
///
/// Entries take their numbers from the same client-scoped allocator as nodes, so two people
/// creating a style in the same instant both keep theirs. Objects inside an entry (modes of a
/// collection, values per mode, replies of a thread) are not independent objects — concurrent
/// writes to different slots of the same entry resolve LWW on the whole entry.
#[derive(Clone, PartialEq, Eq, Hash, Serialize, Deserialize, Debug, PartialOrd, Ord)]
pub struct EntryId(pub Arc<str>);

impl EntryId {
    /// A style entry (`S:<n>` in v2).
    #[must_use]
    pub fn style(n: u64) -> Self {
        EntryId(Arc::from(format!("S:{n}")))
    }
    /// A variable id (`VariableID:<n>`).
    #[must_use]
    pub fn variable(n: u64) -> Self {
        EntryId(Arc::from(format!("VariableID:{n}")))
    }
    /// A variable collection (`VariableCollectionId:<n>`).
    #[must_use]
    pub fn collection(n: u64) -> Self {
        EntryId(Arc::from(format!("VariableCollectionId:{n}")))
    }
    /// A mode of a collection (`<collection>/mode<n>`).
    #[must_use]
    pub fn mode(collection: u64, n: u64) -> Self {
        EntryId(Arc::from(format!("VariableCollectionId:{collection}/mode{n}")))
    }
    /// A comment thread by number.
    #[must_use]
    pub fn comment(n: u64) -> Self {
        EntryId(Arc::from(format!("Comment:{n}")))
    }

    /// The numeric tail of the entry id (the digits at the end), if any.
    fn numeric_tail(&self) -> Option<u64> {
        let digits: String = self.0.chars().rev().take_while(char::is_ascii_digit).collect::<String>().chars().rev().collect();
        if digits.is_empty() { None } else { digits.parse().ok() }
    }

    /// The client space this entry was created in (the tail's high 32 bits), for server checks.
    #[must_use]
    pub fn space(&self) -> Option<u32> {
        self.numeric_tail().map(|v| (v >> 32) as u32)
    }

    /// The numeric tail itself.
    #[must_use]
    pub fn number(&self) -> Option<u64> {
        self.numeric_tail()
    }
}

/// A fractional position key (spec §5.2); see [`crate::frac`].
pub type Position = String;

/// A batch id: unique per client, echoed back on accept or reject.
pub type BatchId = u64;

/// The server sequence number stamped on every accepted batch.
pub type Seq = u64;

/// One synchronized change (spec §2.3, §4.6 — v2 shapes verbatim, `Move` folded into `Set`).
#[derive(Clone, Serialize, Deserialize, Debug, PartialEq)]
pub enum Change {
    /// Create an object: only the properties that differ from a fresh object of the kind.
    Create {
        /// The new object's id (must be in the sender's space, or a previously removed id).
        id: NodeId,
        /// The object kind (adapter-defined).
        kind: Arc<str>,
        /// The parent object id.
        parent: NodeId,
        /// The fractional position among siblings.
        position: Position,
        /// Initial properties (only the non-default ones).
        props: Vec<(Arc<str>, Value)>,
    },
    /// Set one property of one object (also carries `parent`/`position` writes for moves).
    Set {
        /// The target object.
        id: NodeId,
        /// The property name.
        prop: Arc<str>,
        /// The new value.
        value: Value,
    },
    /// Delete an object subtree; deleting a deleted object does nothing (tolerant).
    Delete {
        /// The subtree root.
        id: NodeId,
    },
    /// Set a whole-document property (page order, doc metadata).
    SetDoc {
        /// The document property.
        prop: Arc<str>,
        /// The value.
        value: Value,
    },
    /// Put or replace a document-level entry (style/variable/collection/comment/version...).
    Put {
        /// The entry id.
        entry: EntryId,
        /// The entry value.
        value: Value,
    },
    /// Remove a document-level entry.
    Remove {
        /// The entry id.
        entry: EntryId,
    },
}

impl Change {
    /// The object touched by this change, if any (used for region-lock scoping, spec §6.7).
    #[must_use]
    pub fn target_node(&self) -> Option<NodeId> {
        match self {
            Change::Create { id, .. } | Change::Set { id, .. } | Change::Delete { id } => Some(*id),
            Change::SetDoc { .. } | Change::Put { .. } | Change::Remove { .. } => None,
        }
    }

    /// The properties written by this change (for per-property region locks, spec §7.6).
    #[must_use]
    pub fn target_props(&self) -> Vec<Arc<str>> {
        match self {
            Change::Create { props, .. } => props.iter().map(|(k, _)| k.clone()).collect(),
            Change::Set { prop, .. } => vec![prop.clone()],
            Change::Delete { .. } | Change::SetDoc { .. } | Change::Put { .. } | Change::Remove { .. } => Vec::new(),
        }
    }
}

/// A batch: one user action, one undo step (spec §2.3).
#[derive(Clone, Serialize, Deserialize, Debug, PartialEq)]
pub struct Batch {
    /// Unique id from the authoring client.
    pub id: BatchId,
    /// The last server seq the batch was computed against.
    pub base_seq: Seq,
    /// The changes, ordered: field writes → creates (parents first) → moves → deletes → doc-level.
    pub changes: Vec<Change>,
}

impl Batch {
    /// An empty batch at `base_seq` (used by tests and heartbeat paths).
    #[must_use]
    pub fn empty(id: BatchId, base_seq: Seq) -> Self {
        Batch { id, base_seq, changes: Vec::new() }
    }
}
