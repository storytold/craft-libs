//! (Test code: assertions may panic — that is their point.)
#![allow(clippy::expect_used, clippy::unwrap_used, clippy::panic)]

use craft_crdt::change::{Batch, Change, EntryId};
use craft_crdt::ids::NodeId;
use craft_crdt::merge::three_way;
use craft_crdt::replica::{ApplyMode, Replica};
use craft_crdt::value::Value;
use std::sync::Arc;

fn create(id: u64, parent: u64, pos: &str) -> Change {
    Change::Create {
        id: NodeId(id),
        kind: Arc::from("rect"),
        parent: NodeId(parent),
        position: pos.into(),
        props: vec![],
    }
}

fn apply(r: &mut Replica, changes: Vec<Change>) {
    r.apply(&Batch { id: 0, base_seq: r.seq(), changes }, ApplyMode::Strict).expect("apply");
}

#[test]
fn a_branch_only_move_lands() {
    let mut base = Replica::new();
    apply(&mut base, vec![create(1, 0, "a0"), create(2, 0, "a1")]);
    let main = base.clone();
    let branch_changes = vec![Change::Set {
        id: NodeId(2),
        prop: Arc::from("parent"),
        value: Value::Json(serde_json::Value::Number(1.into())),
    }];
    let plan = three_way(&base, &branch_changes, &main, 0);
    assert!(plan.conflicts.is_empty(), "main never moved it: {plan:?}");
    assert!(
        plan.auto.iter().any(|c| matches!(c, Change::Set { id: NodeId(2), prop, .. } if prop.as_ref() == "parent")),
        "branch move must auto-apply: {:?}",
        plan.auto
    );
}

#[test]
fn a_branch_only_reorder_lands() {
    let mut base = Replica::new();
    apply(&mut base, vec![create(1, 0, "a0"), create(2, 0, "a1")]);
    let main = base.clone();
    let branch_changes = vec![Change::Set {
        id: NodeId(2),
        prop: Arc::from("position"),
        value: Value::Json(serde_json::Value::String("a0V".into())),
    }];
    let plan = three_way(&base, &branch_changes, &main, 0);
    assert!(plan.conflicts.is_empty());
    assert!(plan.auto.iter().any(|c| matches!(c, Change::Set { id: NodeId(2), prop, .. } if prop.as_ref() == "position")));
}

#[test]
fn a_branch_deletion_lands() {
    let mut base = Replica::new();
    apply(&mut base, vec![create(1, 0, "a0")]);
    let main = base.clone();
    let plan = three_way(&base, &[Change::Delete { id: NodeId(1) }], &main, 0);
    assert!(plan.conflicts.is_empty());
    assert_eq!(plan.auto, vec![Change::Delete { id: NodeId(1) }]);
}

#[test]
fn a_branch_entry_removal_lands() {
    let mut base = Replica::new();
    apply(&mut base, vec![Change::Put { entry: EntryId::style(1), value: Value::Json(serde_json::Value::String("s".into())) }]);
    let main = base.clone();
    let plan = three_way(&base, &[Change::Remove { entry: EntryId::style(1) }], &main, 0);
    assert_eq!(plan.auto, vec![Change::Remove { entry: EntryId::style(1) }]);
}

#[test]
fn both_sides_moving_to_different_parents_conflict() {
    let mut base = Replica::new();
    apply(&mut base, vec![create(1, 0, "a0"), create(2, 0, "a1"), create(3, 0, "a2")]);
    let mut main = base.clone();
    apply(
        &mut main,
        vec![Change::Set {
            id: NodeId(3),
            prop: Arc::from("parent"),
            value: Value::Json(serde_json::Value::Number(1.into())),
        }],
    );
    let branch_changes = vec![Change::Set {
        id: NodeId(3),
        prop: Arc::from("parent"),
        value: Value::Json(serde_json::Value::Number(2.into())),
    }];
    let plan = three_way(&base, &branch_changes, &main, 0);
    assert!(plan.conflicts.iter().any(|c| c.prop.as_deref() == Some("parent")));
}

fn apply_auto(main: &Replica, plan: &craft_crdt::merge::MergePlan) -> Replica {
    let mut r = main.clone();
    r.apply(&Batch { id: 0, base_seq: r.seq(), changes: plan.auto.clone() }, ApplyMode::Strict).expect("strict apply of plan.auto");
    r
}

#[test]
fn a_move_into_a_new_group_creates_the_group_first() {
    let mut base = Replica::new();
    apply(&mut base, vec![create(2, 0, "a0")]);
    let main = base.clone();
    let branch_changes = vec![
        create(5, 0, "a1"),
        Change::Set {
            id: NodeId(2),
            prop: Arc::from("parent"),
            value: Value::Json(serde_json::Value::Number(5.into())),
        },
    ];
    let plan = three_way(&base, &branch_changes, &main, 0);
    assert!(plan.conflicts.is_empty(), "{plan:?}");
    let create_at = plan.auto.iter().position(|c| matches!(c, Change::Create { id: NodeId(5), .. }));
    let move_at = plan.auto.iter().position(|c| matches!(c, Change::Set { id: NodeId(2), prop, .. } if prop.as_ref() == "parent"));
    assert!(create_at.expect("create group") < move_at.expect("move into group"), "got {:?}", plan.auto);
    let r = apply_auto(&main, &plan);
    assert_eq!(r.node(NodeId(2)).expect("moved").parent, NodeId(5));
    assert!(r.node(NodeId(5)).is_some());
}

#[test]
fn a_move_out_lands_before_the_old_parent_is_deleted() {
    let mut base = Replica::new();
    apply(&mut base, vec![create(1, 0, "a0"), create(2, 1, "a0")]);
    let main = base.clone();
    let branch_changes = vec![
        Change::Set {
            id: NodeId(2),
            prop: Arc::from("parent"),
            value: Value::Json(serde_json::Value::Number(0.into())),
        },
        Change::Delete { id: NodeId(1) },
    ];
    let plan = three_way(&base, &branch_changes, &main, 0);
    assert!(plan.conflicts.is_empty(), "{plan:?}");
    let move_at = plan.auto.iter().position(|c| matches!(c, Change::Set { id: NodeId(2), prop, .. } if prop.as_ref() == "parent"));
    let delete_at = plan.auto.iter().position(|c| matches!(c, Change::Delete { id: NodeId(1) }));
    assert!(move_at.expect("move out") < delete_at.expect("delete parent"), "got {:?}", plan.auto);
    let r = apply_auto(&main, &plan);
    assert!(r.node(NodeId(1)).is_none(), "parent gone");
    assert_eq!(r.node(NodeId(2)).expect("child survived").parent, NodeId(0));
}

#[test]
fn parent_moves_that_would_cycle_in_id_order_are_unblocked() {
    let mut base = Replica::new();
    apply(&mut base, vec![create(1, 0, "a0"), create(2, 1, "a0")]);
    let main = base.clone();
    let branch_changes = vec![
        Change::Set {
            id: NodeId(2),
            prop: Arc::from("parent"),
            value: Value::Json(serde_json::Value::Number(0.into())),
        },
        Change::Set {
            id: NodeId(1),
            prop: Arc::from("parent"),
            value: Value::Json(serde_json::Value::Number(2.into())),
        },
    ];
    let plan = three_way(&base, &branch_changes, &main, 0);
    assert!(plan.conflicts.is_empty(), "{plan:?}");
    let move2 = plan.auto.iter().position(|c| matches!(c, Change::Set { id: NodeId(2), prop, .. } if prop.as_ref() == "parent"));
    let move1 = plan.auto.iter().position(|c| matches!(c, Change::Set { id: NodeId(1), prop, .. } if prop.as_ref() == "parent"));
    assert!(move2.expect("move 2 to root") < move1.expect("move 1 under 2"), "got {:?}", plan.auto);
    let r = apply_auto(&main, &plan);
    assert_eq!(r.node(NodeId(2)).expect("2").parent, NodeId(0));
    assert_eq!(r.node(NodeId(1)).expect("1").parent, NodeId(2));
}
