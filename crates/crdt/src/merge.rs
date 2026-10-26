//! Three-way merge and per-list merge strategies (spec §6.8, docs/merge-matrix.md).
//!
//! Contributions are suggestion branches above a base seq; merging replays the three-way
//! resolution over the main state: what only the branch touched lands, what both changed the
//! same way lands once, and what both changed *differently* is a conflict spot the owner
//! resolves in the review UI (accept-mine / accept-theirs / edit / reject — spec §8.1).
//!
//! List-valued properties merge per the extendable strategy table (the figcraft fills/strokes/
//! effects element-wise rule, generalized): additions from both sides are kept, removals win,
//! and element-wise conflicts resolve by index with the branch's or main's value flagged.

use crate::change::{Change, EntryId, Seq};
use crate::ids::NodeId;
use crate::replica::Replica;
use crate::value::Value;
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
use std::sync::Arc;

/// One merge decision over a conflicted target.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum Resolution {
    /// Take the branch's value.
    TakeBranch,
    /// Keep main's value.
    KeepMain,
    /// The owner typed a new value.
    Manual(crate::value::Value),
}

/// A conflict spot the owner must resolve (spec §6.8 `NeedsResolution`).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Conflict {
    /// The node (or entry) in conflict.
    pub target: Target,
    /// The property (for node targets).
    pub prop: Option<Arc<str>>,
    /// Main's current value.
    pub main: crate::value::Value,
    /// The branch's value.
    pub branch: crate::value::Value,
}

/// What a change targets.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum Target {
    /// A node property.
    Node(crate::ids::NodeId),
    /// A document-level entry.
    Entry(EntryId),
    /// A whole-document property.
    Doc(Arc<str>),
}

/// The merge plan: what lands automatically and what needs the owner.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct MergePlan {
    /// `Changes` that apply without conflict (the vast majority — spec §6.8).
    pub auto: Vec<Change>,
    /// Spots both sides changed differently.
    pub conflicts: Vec<Conflict>,
    /// The base seq the branch forked from.
    pub base_seq: Seq,
}

/// Compute the three-way merge of `branch_changes` (already ordered against base) onto `main`.
///
/// `base` is the state at the fork point; the caller replays `branch_changes` against a copy of
/// it to obtain the branch head. Everything the branch wrote that main has not touched since
/// `base_seq` lands automatically; the rest are conflicts.
#[must_use]
pub fn three_way(base: &Replica, branch_changes: &[Change], main: &Replica, base_seq: Seq) -> MergePlan {
    // Materialize the branch head by replaying the branch over the base.
    let mut branch = base.clone();
    let replay = crate::change::Batch { id: 0, base_seq, changes: branch_changes.to_vec() };
    let _ = branch.apply(&replay, crate::replica::ApplyMode::Lenient);

    let mut plan = MergePlan { auto: Vec::new(), conflicts: Vec::new(), base_seq };

    // Union of base and branch ids so a branch deletion is visible (branch.nodes() alone
    // never matches the `(None, Some, _)` arm).
    let mut node_ids: BTreeSet<NodeId> = BTreeSet::new();
    node_ids.extend(branch.nodes().map(|(id, _)| *id));
    node_ids.extend(base.nodes().map(|(id, _)| *id));
    for id in node_ids {
        let b = branch.node(id);
        let base_n = base.node(id);
        let m = main.node(id);
        match (b, base_n, m) {
            (Some(bn), Some(bn0), Some(mn)) => {
                for (k, v) in &bn.props {
                    let base_v = bn0.props.get(k);
                    if base_v == Some(v) {
                        continue;
                    }
                    let main_v = mn.props.get(k);
                    if main_v.is_none_or(|x| Some(x) == base_v) {
                        plan.auto.push(Change::Set { id, prop: k.clone(), value: v.clone() });
                    } else if main_v != Some(v) {
                        let mv = main_v.cloned().unwrap_or(Value::Json(serde_json::Value::Null));
                        plan.conflicts.push(Conflict { target: Target::Node(id), prop: Some(k.clone()), main: mv, branch: v.clone() });
                    }
                }
                three_way_parent(&mut plan, id, bn0.parent, bn.parent, mn.parent);
                three_way_position(&mut plan, id, &bn0.position, &bn.position, &mn.position);
            }
            (Some(bn), None, _) => {
                plan.auto.push(Change::Create {
                    id,
                    kind: bn.kind.clone(),
                    parent: bn.parent,
                    position: bn.position.clone(),
                    props: bn.props.iter().map(|(k, v)| (k.clone(), v.clone())).collect(),
                });
            }
            (None, Some(_), Some(_)) => {
                plan.auto.push(Change::Delete { id });
            }
            _ => {}
        }
    }

    let mut entry_ids: BTreeSet<EntryId> = BTreeSet::new();
    entry_ids.extend(branch.entries().map(|(e, _)| e.clone()));
    entry_ids.extend(base.entries().map(|(e, _)| e.clone()));
    for entry in entry_ids {
        let bv = branch.entry(&entry);
        let base_v = base.entry(&entry);
        let main_v = main.entry(&entry);
        match (bv, base_v, main_v) {
            (None, Some(_), Some(_)) => plan.auto.push(Change::Remove { entry }),
            (None, _, _) => {}
            (Some(bv), _, _) => {
                if base_v == Some(bv) {
                    continue;
                }
                if main_v.is_none_or(|x| Some(x) == base_v) {
                    plan.auto.push(Change::Put { entry, value: bv.clone() });
                } else if main_v != Some(bv) {
                    let mv = main_v.cloned().unwrap_or(Value::Json(serde_json::Value::Null));
                    plan.conflicts.push(Conflict { target: Target::Entry(entry), prop: None, main: mv, branch: bv.clone() });
                }
            }
        }
    }
    for (k, bv) in branch.doc_props() {
        let base_v = base.doc_prop(k);
        if base_v == Some(bv) {
            continue;
        }
        let main_v = main.doc_prop(k);
        if main_v.is_none_or(|x| Some(x) == base_v) {
            plan.auto.push(Change::SetDoc { prop: k.clone(), value: bv.clone() });
        } else if main_v != Some(bv) {
            let mv = main_v.cloned().unwrap_or(crate::value::Value::Json(serde_json::Value::Null));
            plan.conflicts.push(Conflict { target: Target::Doc(k.clone()), prop: None, main: mv, branch: bv.clone() });
        }
    }
    plan
}

fn three_way_parent(plan: &mut MergePlan, id: NodeId, base: NodeId, branch: NodeId, main: NodeId) {
    if branch == base {
        return;
    }
    if main == base {
        plan.auto.push(Change::Set {
            id,
            prop: Arc::from("parent"),
            value: Value::Json(serde_json::Value::Number(branch.0.into())),
        });
    } else if main != branch {
        plan.conflicts.push(Conflict {
            target: Target::Node(id),
            prop: Some(Arc::from("parent")),
            main: Value::Json(serde_json::Value::Number(main.0.into())),
            branch: Value::Json(serde_json::Value::Number(branch.0.into())),
        });
    }
}

fn three_way_position(plan: &mut MergePlan, id: NodeId, base: &str, branch: &str, main: &str) {
    if branch == base {
        return;
    }
    if main == base {
        plan.auto.push(Change::Set {
            id,
            prop: Arc::from("position"),
            value: Value::Json(serde_json::Value::String(branch.to_owned())),
        });
    } else if main != branch {
        plan.conflicts.push(Conflict {
            target: Target::Node(id),
            prop: Some(Arc::from("position")),
            main: Value::Json(serde_json::Value::String(main.to_owned())),
            branch: Value::Json(serde_json::Value::String(branch.to_owned())),
        });
    }
}

/// The merge strategy for a list-valued property (the extendable matrix, docs/merge-matrix.md).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ListStrategy {
    /// Whole-list LWW (the default: big or list-valued properties replace atomically — spec §2.3).
    ReplaceWhole,
    /// Element-wise (figcraft fills/strokes/effects rule, spec §18.8): additions from both kept,
    /// removals win, same-index different values conflict.
    ElementWise,
    /// Text properties merge through RGA (spec §6.6).
    RgaText,
    /// Add-add both kept (general lists): union with id tie-break.
    UnionKeepBoth,
}

/// `Resolve` one conflict per the chosen resolution (returns the change to apply).
#[must_use]
pub fn resolve(c: &Conflict, r: &Resolution) -> Option<Change> {
    let value = match r {
        Resolution::TakeBranch => c.branch.clone(),
        Resolution::KeepMain => return None,
        Resolution::Manual(v) => v.clone(),
    };
    match &c.target {
        Target::Node(id) => Some(Change::Set { id: *id, prop: c.prop.clone()?, value }),
        Target::Entry(e) => Some(Change::Put { entry: e.clone(), value }),
        Target::Doc(k) => Some(Change::SetDoc { prop: k.clone(), value }),
    }
}
