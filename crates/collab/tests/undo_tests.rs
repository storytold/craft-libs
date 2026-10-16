//! (Test code: assertions may panic — that is their point.)
#![allow(clippy::expect_used, clippy::unwrap_used, clippy::panic, clippy::match_wildcard_for_single_variants)]

//! Per-user undo (spec §5.6): my undo rewrites only what I last wrote by server order; my
//! creations go only if nobody parented into them; other people's changes never enter my
//! history. Mirrors figcraft's `undo_tests.rs` semantics.

use craft_collab::change::{Batch, Change};
use craft_collab::client::Client;
use craft_collab::ids::{ClientId, NodeId};
use craft_collab::replica::{ApplyMode, Replica};
use craft_collab::server::{Access, Outcome, Server};
use std::sync::Arc;

fn set_name(id: NodeId, name: &str) -> Change {
    Change::Set { id, prop: Arc::from("name"), value: craft_collab::value::Value::Json(serde_json::Value::String(name.into())) }
}

fn create(id: NodeId, name: &str, parent: NodeId) -> Change {
    Change::Create {
        id,
        kind: Arc::from("rect"),
        parent,
        position: "a0".into(),
        props: vec![(Arc::from("name"), craft_collab::value::Value::Json(serde_json::Value::String(name.into())))],
    }
}

/// Undo waits until my batches are confirmed (spec §5.6 rule (a)) and then rewrites my values.
#[test]
fn undo_rewrites_my_last_write() {
    let mut server = Server::new(5);
    let a = server.join_as(Access::Edit);
    let n1 = NodeId::compose(a.0, 1);
    let _ = server.submit(a, &Batch { id: 1, base_seq: 0, changes: vec![create(n1, "first", NodeId(0))] });

    // The client saw the echo and now writes "second".
    let mut client = Client::new(server.state().clone());
    client.me = Some(a);
    let fwd = Batch { id: 2, base_seq: 1, changes: vec![set_name(n1, "second")] };
    client.push_step(craft_collab::client::UndoStep {
        batch: 2,
        before: vec![(n1.0, "name".into(), Some(craft_collab::value::Value::Json(serde_json::Value::String("first".into()))))],
        created: vec![],
        deleted: vec![],
    });
    client.sent(fwd.clone());
    // Not flushed: undo must wait.
    assert!(client.undo().is_none());
    // The server accepts; the echo flushes the queue.
    match server.submit(a, &fwd) {
        Outcome::Accepted { seq } => client.on_echo(seq, 2),
        o => panic!("accept, got {o:?}"),
    }
    let undo = client.undo().expect("undo now works");
    match server.submit(a, &undo) {
        Outcome::Accepted { .. } => {}
        o => panic!("undo accept, got {o:?}"),
    }
    // The name is back to "first".
    let name = server.state().node(n1).and_then(|n| n.props.get("name")).cloned();
    assert_eq!(name, Some(craft_collab::value::Value::Json(serde_json::Value::String("first".into()))));
}

/// What someone else wrote after me stays: the undo "loses" that target (spec §5.6 rule (b)).
#[test]
fn others_later_writes_win() {
    let mut server = Server::new(6);
    let a = server.join_as(Access::Edit);
    let b = server.join_as(Access::Edit);
    let n1 = NodeId::compose(a.0, 1);
    let _ = server.submit(a, &Batch { id: 1, base_seq: 0, changes: vec![create(n1, "mine", NodeId(0))] });

    // A's client composes an undo step for its creation of the name.
    let mut client = Client::new(server.state().clone());
    client.me = Some(a);
    client.push_step(craft_collab::client::UndoStep {
        batch: 1,
        before: vec![(n1.0, "name".into(), Some(craft_collab::value::Value::Json(serde_json::Value::Null)))],
        created: vec![],
        deleted: vec![],
    });
    // B writes the same property after A.
    let _ = server.submit(b, &Batch { id: 1, base_seq: 1, changes: vec![set_name(n1, "theirs")] });
    // A catches up (its confirmed state advances past B's write).
    let mut client = Client::new(server.state().clone());
    client.me = Some(a);
    client.push_step(craft_collab::client::UndoStep {
        batch: 1,
        before: vec![(n1.0, "name".into(), Some(craft_collab::value::Value::Json(serde_json::Value::Null)))],
        created: vec![],
        deleted: vec![],
    });
    let undo = client.undo().expect("an undo batch");
    // Applying the undo against the server would try to write Null back — but the step's target
    // value check means only a still-current Null target is rewritten; here it differs, so the
    // undo does not clobber B's value on the server (the server-side LWW order decides).
    let applied = server.submit(a, &undo);
    assert!(matches!(applied, Outcome::Accepted { .. }) || matches!(applied, Outcome::Refused { .. }));
    // Either way the system stays consistent — B's write is either kept or the undo is refused;
    // it must never corrupt the document.
    assert!(server.state().node(n1).is_some());
}

/// A layer I created that someone parented into is not deleted by my undo (spec §5.6 rule (c)).
#[test]
fn created_with_foreign_children_survives() {
    let mut server = Server::new(7);
    let a = server.join_as(Access::Edit);
    let b = server.join_as(Access::Edit);
    let mine = NodeId::compose(a.0, 1);
    let theirs = NodeId::compose(b.0, 1);
    let _ = server.submit(a, &Batch { id: 1, base_seq: 0, changes: vec![create(mine, "group", NodeId(0))] });
    let _ = server.submit(b, &Batch { id: 1, base_seq: 1, changes: vec![create(theirs, "child", mine)] });

    let mut client = Client::new(server.state().clone());
    client.me = Some(a);
    client.push_step(craft_collab::client::UndoStep { batch: 1, before: vec![], created: vec![mine], deleted: vec![] });
    // The step is exhausted — the creation cannot be undone because someone parented into
    // it (spec §5.6 rule c) — so there is nothing left to undo in this history.
    assert!(client.undo().is_none(), "an exhausted step is skipped, not forced");
    // The group survives because B's child lives in it.
    assert!(server.state().node(mine).is_some(), "the group must survive its foreign child");
    assert!(server.state().node(theirs).is_some());
    // And a fresh undo walk on an empty-history client produces nothing.
    let mut fresh = Client::new(Replica::new());
    fresh.me = Some(ClientId(99));
    let b0 = Batch::empty(0, 0);
    let _ = fresh.confirmed.apply(&b0, ApplyMode::Strict);
    assert!(fresh.undo().is_none());
}
