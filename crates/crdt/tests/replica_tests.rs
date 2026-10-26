//! (Test code: assertions may panic — that is their point.)
#![allow(clippy::expect_used, clippy::unwrap_used, clippy::panic)]

use craft_crdt::change::{Batch, Change};
use craft_crdt::ids::NodeId;
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
fn a_node_can_move_back_to_the_root() {
    let mut r = Replica::new();
    apply(&mut r, vec![create(1, 0, "a0"), create(2, 1, "a0")]);
    apply(
        &mut r,
        vec![Change::Set {
            id: NodeId(2),
            prop: Arc::from("parent"),
            value: Value::Json(serde_json::Value::Number(0.into())),
        }],
    );
    let n = r.node(NodeId(2)).expect("moved");
    assert_eq!(n.parent, NodeId(0));
    assert!(r.children(NodeId(0)).any(|id| id == NodeId(2)));
}

#[test]
fn lenient_apply_skips_an_invalid_set_and_keeps_the_rest() {
    let mut r = Replica::new();
    apply(&mut r, vec![create(1, 0, "a0")]);
    let batch = Batch {
        id: 1,
        base_seq: r.seq(),
        changes: vec![
            Change::Set {
                id: NodeId(1),
                prop: Arc::from("parent"),
                value: Value::Json(serde_json::Value::Number(1.into())),
            },
            Change::SetDoc { prop: Arc::from("title"), value: Value::Json(serde_json::Value::String("ok".into())) },
        ],
    };
    r.apply(&batch, ApplyMode::Lenient).expect("lenient skips the cycle");
    assert_eq!(r.node(NodeId(1)).expect("still there").parent, NodeId(0));
    assert_eq!(r.doc_prop("title"), Some(&Value::Json(serde_json::Value::String("ok".into()))));
}

#[test]
fn snapshot_roundtrips_with_two_removed_ids() {
    let mut s = craft_crdt::Server::new(1);
    let a = s.join_as(craft_crdt::server::Access::Edit);
    let n1 = NodeId::compose(a.0, 1);
    let n2 = NodeId::compose(a.0, 2);
    let mk = |id: NodeId| Change::Create {
        id,
        kind: Arc::from("rect"),
        parent: NodeId(0),
        position: "a0".into(),
        props: vec![],
    };
    assert!(matches!(s.submit(a, &Batch { id: 1, base_seq: 0, changes: vec![mk(n1), mk(n2)] }), craft_crdt::Outcome::Accepted { .. }));
    assert!(matches!(
        s.submit(a, &Batch { id: 2, base_seq: 1, changes: vec![Change::Delete { id: n1 }, Change::Delete { id: n2 }] }),
        craft_crdt::Outcome::Accepted { .. }
    ));
    let bytes = s.state_out().encode().expect("encode");
    let back = craft_crdt::SessionState::decode(&bytes).expect("decode must be byte-stable with two removed ids");
    assert_eq!(back.encode().expect("re-encode"), bytes);
}
