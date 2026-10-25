//! (Test code: assertions may panic — that is their point.)
#![allow(clippy::expect_used, clippy::unwrap_used, clippy::panic, clippy::match_wildcard_for_single_variants)]

//! The pure server's contract (spec §5.4): atomic accept with seq, refusal with reason, id
//! spaces, cycle rejection, view access — mirroring figcraft's `server_tests.rs`.

use craft_crdt::change::{Batch, Change};
use craft_crdt::ids::NodeId;
use craft_crdt::server::{Access, Outcome, Server};
use std::sync::Arc;

fn create(id: u64, name: &str) -> Change {
    Change::Create {
        id: NodeId(id),
        kind: Arc::from("rect"),
        parent: NodeId(0),
        position: "a0".into(),
        props: vec![(Arc::from("name"), craft_crdt::value::Value::Json(serde_json::Value::String(name.into())))],
    }
}

fn batch(id: u64, seq: u64, changes: Vec<Change>) -> Batch {
    Batch { id, base_seq: seq, changes }
}

#[test]
fn accepts_and_orders() {
    let mut s = Server::new(7);
    let a = s.join_as(Access::Edit);
    let b = s.join_as(Access::Edit);
    match s.submit(a, &batch(1, 0, vec![create(NodeId((u64::from(a.0) << 32) | 1).0, "one")])) {
        Outcome::Accepted { seq } => assert_eq!(seq, 1),
        o => panic!("expected accept, got {o:?}"),
    }
    match s.submit(b, &batch(1, 1, vec![create(NodeId((u64::from(b.0) << 32) | 1).0, "two")])) {
        Outcome::Accepted { seq } => assert_eq!(seq, 2),
        o => panic!("expected accept, got {o:?}"),
    }
    assert_eq!(s.state().seq(), 2);
}

#[test]
fn view_access_is_refused_server_side() {
    let mut s = Server::new(1);
    let viewer = s.join_as(Access::View);
    let out = s.submit(viewer, &batch(1, 0, vec![create(1, "x")]));
    match out {
        Outcome::Refused { reason, fixes } => {
            assert!(reason.contains("view"), "reason: {reason}");
            assert_eq!(fixes, [] as [craft_crdt::Change; 0]);
        }
        o => panic!("view must be refused, got {o:?}"),
    }
}

#[test]
fn foreign_id_space_is_refused() {
    let mut s = Server::new(1);
    let a = s.join_as(Access::Edit);
    // An id in space 9 while the author is not 9.
    let foreign = NodeId::compose(9, 1);
    match s.submit(a, &batch(1, 0, vec![create(foreign.0, "spy")])) {
        Outcome::Refused { reason, .. } => assert!(reason.contains("space"), "reason: {reason}"),
        o => panic!("foreign space must be refused, got {o:?}"),
    }
}

#[test]
fn cycles_are_refused() {
    let mut s = Server::new(1);
    let a = s.join_as(Access::Edit);
    let n1 = NodeId::compose(a.0, 1);
    let n2 = NodeId::compose(a.0, 2);
    let _ = s.submit(a, &batch(1, 0, vec![create(n1.0, "a"), create(n2.0, "b")]));
    // Make n1 the child of n2, then n2 the child of n1 — a cycle.
    let set_parent = |id: NodeId, parent: NodeId| Change::Set {
        id,
        prop: Arc::from("parent"),
        value: craft_crdt::value::Value::Json(serde_json::Value::Number(parent.0.into())),
    };
    let _ = s.submit(a, &batch(2, 2, vec![set_parent(n1, n2)]));
    match s.submit(a, &batch(3, 3, vec![set_parent(n2, n1)])) {
        Outcome::Refused { reason, .. } => assert!(reason.contains("cycle"), "reason: {reason}"),
        o => panic!("cycle must be refused, got {o:?}"),
    }
}

#[test]
fn deleted_ids_are_recreated_not_duplicated() {
    let mut s = Server::new(1);
    let a = s.join_as(Access::Edit);
    let n1 = NodeId::compose(a.0, 1);
    let _ = s.submit(a, &batch(1, 0, vec![create(n1.0, "x")]));
    let _ = s.submit(a, &batch(2, 1, vec![Change::Delete { id: n1 }]));
    assert!(s.state().node(n1).is_none());
    // Deleting a deleted id is a tolerant no-op.
    match s.submit(a, &batch(3, 2, vec![Change::Delete { id: n1 }])) {
        Outcome::Accepted { .. } => {}
        o => panic!("tolerant delete, got {o:?}"),
    }
    // Recreating with the same id is accepted (the removed set remembers it, spec §2.2).
    match s.submit(a, &batch(4, 3, vec![create(n1.0, "back")])) {
        Outcome::Accepted { .. } => {}
        o => panic!("recreate, got {o:?}"),
    }
    assert_eq!(s.state().node_count(), 1);
}

#[test]
fn state_roundtrips_through_serde() {
    let mut s = Server::new(99);
    let a = s.join_as(Access::Edit);
    let _ = s.submit(a, &batch(1, 0, vec![create(NodeId::compose(a.0, 1).0, "x")]));
    let state = s.state_out();
    let bytes = state.encode().expect("encode");
    let back = craft_crdt::SessionState::decode(&bytes).expect("decode");
    let restored = Server::restore(back);
    assert_eq!(restored.state().seq(), s.state().seq());
    assert_eq!(restored.state().node_count(), s.state().node_count());
}
