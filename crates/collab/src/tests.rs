//! Tests. The session is pure, so every one of these runs without a transport.

use serde_json::json;

use crate::proto::{ClientMsg, LockState, Presence, ServerMsg};
use crate::{Effect, Session, decode, encode};

const DOC: &str = "doc-42";

fn joined(seq: u64) -> ServerMsg {
    ServerMsg::Joined {
        doc: DOC.to_owned(),
        peer: "me".to_owned(),
        seq,
        roster: vec![Presence { peer: "me".to_owned(), display: "Alice".to_owned() }],
        locks: Vec::new(),
    }
}

fn op(seq: u64, origin: &str, local: Option<u64>, cell: &str) -> ServerMsg {
    ServerMsg::Op {
        doc: DOC.to_owned(),
        seq,
        origin: origin.to_owned(),
        local,
        command: "cell.set".to_owned(),
        params: json!({"cell": cell, "input": "7"}),
        scope: Some(cell.to_owned()),
    }
}

fn session() -> Session {
    let mut s = Session::new(DOC);
    s.handle(joined(0));
    s
}

// ---- joining ---------------------------------------------------------------

#[test]
fn joining_records_identity_sequence_and_roster() {
    let mut s = Session::new(DOC);
    let out = s.handle(joined(17));
    assert_eq!(s.peer(), Some("me"));
    assert_eq!(s.applied(), 17, "a join starts the client at the relay's sequence");
    assert_eq!(s.roster().len(), 1);
    assert!(matches!(out.first(), Some(Effect::Roster { .. })));
}

#[test]
fn a_join_sorts_existing_locks_into_mine_and_theirs() {
    let mut s = Session::new(DOC);
    s.handle(ServerMsg::Joined {
        doc: DOC.to_owned(),
        peer: "me".to_owned(),
        seq: 0,
        roster: Vec::new(),
        locks: vec![LockState { scope: "A1".to_owned(), holder: "me".to_owned() }, LockState { scope: "B2".to_owned(), holder: "bob".to_owned() }],
    });
    assert!(s.holds("A1"), "a lock already held by us must be recognised as ours");
    assert!(!s.holds("B2"));
    assert_eq!(s.blocked_by("B2"), Some("bob"));
    assert!(!s.may_edit("B2"));
    assert!(s.may_edit("A1"));
    assert!(s.may_edit("Z9"), "an unclaimed scope is editable");
}

#[test]
fn messages_for_another_document_are_ignored() {
    let mut s = session();
    let other = ServerMsg::Op {
        doc: "someone-else".to_owned(),
        seq: 1,
        origin: "bob".to_owned(),
        local: None,
        command: "cell.set".to_owned(),
        params: json!({}),
        scope: None,
    };
    assert!(s.handle(other).is_empty());
    assert_eq!(s.applied(), 0, "another document must not advance our sequence");
}

// ---- ordering --------------------------------------------------------------

#[test]
fn an_in_order_remote_op_is_applied() {
    let mut s = session();
    let out = s.handle(op(1, "bob", None, "B2"));
    assert_eq!(out.len(), 1);
    match &out[0] {
        Effect::Apply { command, params } => {
            assert_eq!(command, "cell.set");
            assert_eq!(params["cell"], "B2");
        }
        other => panic!("expected Apply, got {other:?}"),
    }
    assert_eq!(s.applied(), 1);
}

#[test]
fn our_own_op_is_confirmed_and_never_applied_twice() {
    let mut s = session();
    let msg = s.submit("cell.set", json!({"cell": "B2", "input": "7"}), Some("B2".into()));
    let local = match msg {
        ClientMsg::Submit { local, .. } => local,
        other => panic!("expected Submit, got {other:?}"),
    };
    assert_eq!(s.inflight(), 1);

    let out = s.handle(op(1, "me", Some(local), "B2"));
    assert_eq!(out, vec![Effect::Confirmed { local, seq: 1 }], "our own edit is already applied locally");
    assert_eq!(s.inflight(), 0, "confirmation clears the in-flight marker");
}

#[test]
fn an_early_op_is_held_until_the_gap_fills() {
    let mut s = session();

    // 3 and 2 arrive before 1. Nothing may be applied yet.
    assert!(s.handle(op(3, "bob", None, "C3")).is_empty());
    assert!(s.handle(op(2, "bob", None, "B2")).is_empty());
    assert_eq!(s.applied(), 0, "an out-of-order op must not advance the sequence");

    // 1 lands and unblocks the whole run, in order.
    let out = s.handle(op(1, "bob", None, "A1"));
    let cells: Vec<&str> = out
        .iter()
        .filter_map(|e| match e {
            Effect::Apply { params, .. } => params["cell"].as_str(),
            _ => None,
        })
        .collect();
    assert_eq!(cells, vec!["A1", "B2", "C3"], "the held run must drain in sequence order");
    assert_eq!(s.applied(), 3);
}

#[test]
fn a_replayed_op_is_dropped() {
    let mut s = session();
    assert_eq!(s.handle(op(1, "bob", None, "A1")).len(), 1);
    assert!(s.handle(op(1, "bob", None, "A1")).is_empty(), "a re-sent op must not apply twice");
    assert_eq!(s.applied(), 1);
}

#[test]
fn a_mixed_run_confirms_ours_and_applies_theirs_in_order() {
    let mut s = session();
    let mine = match s.submit("cell.set", json!({"cell": "B2"}), None) {
        ClientMsg::Submit { local, .. } => local,
        other => panic!("{other:?}"),
    };
    // Their op is ordered first, ours second, and both arrive at once out of order.
    assert!(s.handle(op(2, "me", Some(mine), "B2")).is_empty());
    let out = s.handle(op(1, "bob", None, "A1"));
    assert_eq!(out.len(), 2);
    assert!(matches!(out[0], Effect::Apply { .. }));
    assert_eq!(out[1], Effect::Confirmed { local: mine, seq: 2 });
    assert_eq!(s.applied(), 2);
}

#[test]
fn too_large_a_gap_asks_for_a_resync() {
    let mut s = session();
    // Hold far more than the cap, all beyond the gap at 1.
    let mut asked = None;
    for seq in 2..=600 {
        for e in s.handle(op(seq, "bob", None, "X1")) {
            if let Effect::Send { msg } = e {
                asked = Some(msg);
            }
        }
        if asked.is_some() {
            break;
        }
    }
    match asked {
        Some(ClientMsg::Resync { have, doc }) => {
            assert_eq!(have, 0, "a resync asks from what we actually have");
            assert_eq!(doc, DOC);
        }
        other => panic!("expected a Resync once the hold buffer filled, got {other:?}"),
    }
}

#[test]
fn a_snapshot_replaces_everything_and_resets() {
    let mut s = session();
    let _ = s.submit("cell.set", json!({}), None);
    assert!(s.handle(op(9, "bob", None, "Z9")).is_empty(), "held, because 9 is out of order");

    let out = s.handle(ServerMsg::Snapshot { doc: DOC.to_owned(), seq: 50, document: json!({"cells": {}}) });
    assert_eq!(out, vec![Effect::Replace { document: json!({"cells": {}}), seq: 50 }]);
    assert_eq!(s.applied(), 50);
    assert_eq!(s.inflight(), 0, "local edits cannot survive a document they did not derive");

    // The previously held op is now behind the snapshot and must be dropped.
    assert!(s.handle(op(9, "bob", None, "Z9")).is_empty());
    // And the sequence continues from the snapshot.
    assert_eq!(s.handle(op(51, "bob", None, "A1")).len(), 1);
}

// ---- locks -----------------------------------------------------------------

#[test]
fn a_granted_lock_is_held() {
    let mut s = session();
    let out = s.handle(ServerMsg::LockGranted { doc: DOC.to_owned(), scope: "B2".to_owned() });
    assert_eq!(out, vec![Effect::LockHeld { scope: "B2".to_owned() }]);
    assert!(s.holds("B2"));
    assert!(s.may_edit("B2"));
}

#[test]
fn a_denied_lock_records_the_holder() {
    let mut s = session();
    let out = s.handle(ServerMsg::LockDenied { doc: DOC.to_owned(), scope: "B2".to_owned(), holder: "bob".to_owned() });
    assert_eq!(out, vec![Effect::LockRefused { scope: "B2".to_owned(), holder: "bob".to_owned() }]);
    assert!(!s.holds("B2"));
    assert_eq!(s.blocked_by("B2"), Some("bob"));
    assert!(!s.may_edit("B2"), "a scope another peer holds is not editable");
}

#[test]
fn releasing_a_lock_drops_it_locally_at_once() {
    let mut s = session();
    s.handle(ServerMsg::LockGranted { doc: DOC.to_owned(), scope: "B2".to_owned() });
    let msg = s.release_lock("B2");
    assert!(!s.holds("B2"), "a release cannot be refused, so it applies immediately");
    assert_eq!(msg, ClientMsg::Unlock { doc: DOC.to_owned(), scope: "B2".to_owned() });
}

#[test]
fn a_remote_release_clears_the_block() {
    let mut s = session();
    s.handle(ServerMsg::LockDenied { doc: DOC.to_owned(), scope: "B2".to_owned(), holder: "bob".to_owned() });
    assert!(!s.may_edit("B2"));
    let out = s.handle(ServerMsg::LockReleased { doc: DOC.to_owned(), scope: "B2".to_owned() });
    assert_eq!(out, vec![Effect::LockDropped { scope: "B2".to_owned() }]);
    assert!(s.may_edit("B2"));
}

#[test]
fn request_and_join_messages_carry_the_document() {
    let s = session();
    assert_eq!(s.join("Alice"), ClientMsg::Join { doc: DOC.to_owned(), display: "Alice".to_owned() });
    assert_eq!(s.request_lock("B2"), ClientMsg::Lock { doc: DOC.to_owned(), scope: "B2".to_owned() });
}

// ---- presence and notices --------------------------------------------------

#[test]
fn presence_updates_the_roster() {
    let mut s = session();
    let roster =
        vec![Presence { peer: "me".to_owned(), display: "Alice".to_owned() }, Presence { peer: "bob".to_owned(), display: "Bob".to_owned() }];
    let out = s.handle(ServerMsg::Presence { doc: DOC.to_owned(), roster: roster.clone() });
    assert_eq!(out, vec![Effect::Roster { roster: roster.clone() }]);
    assert_eq!(s.roster(), roster.as_slice());
}

#[test]
fn an_error_is_advisory_and_does_not_disturb_ordering() {
    let mut s = session();
    let out = s.handle(ServerMsg::Error { doc: DOC.to_owned(), detail: "scope busy".to_owned() });
    assert_eq!(out, vec![Effect::Notice { detail: "scope busy".to_owned() }]);
    assert_eq!(s.applied(), 0);
    assert_eq!(s.handle(op(1, "bob", None, "A1")).len(), 1, "ordering continues after a notice");
}

// ---- wire format -----------------------------------------------------------

#[test]
fn client_messages_round_trip() {
    let msgs = vec![
        ClientMsg::Join { doc: DOC.to_owned(), display: "Alice".to_owned() },
        ClientMsg::Leave { doc: DOC.to_owned() },
        ClientMsg::Submit {
            doc: DOC.to_owned(),
            local: 7,
            command: "cell.set".to_owned(),
            params: json!({"cell": "B2"}),
            scope: Some("B2".to_owned()),
        },
        ClientMsg::Lock { doc: DOC.to_owned(), scope: "B2".to_owned() },
        ClientMsg::Unlock { doc: DOC.to_owned(), scope: "B2".to_owned() },
        ClientMsg::Resync { doc: DOC.to_owned(), have: 12 },
    ];
    for m in msgs {
        let text = encode(&m).expect("encodes");
        let back: ClientMsg = serde_json::from_str(&text).expect("decodes");
        assert_eq!(back, m);
    }
}

#[test]
fn server_messages_round_trip() {
    let msgs = vec![
        joined(3),
        op(4, "bob", Some(1), "B2"),
        ServerMsg::LockGranted { doc: DOC.to_owned(), scope: "B2".to_owned() },
        ServerMsg::LockDenied { doc: DOC.to_owned(), scope: "B2".to_owned(), holder: "bob".to_owned() },
        ServerMsg::LockReleased { doc: DOC.to_owned(), scope: "B2".to_owned() },
        ServerMsg::Presence { doc: DOC.to_owned(), roster: Vec::new() },
        ServerMsg::Snapshot { doc: DOC.to_owned(), seq: 9, document: json!({}) },
        ServerMsg::Error { doc: DOC.to_owned(), detail: "x".to_owned() },
    ];
    for m in msgs {
        let text = serde_json::to_string(&m).expect("encodes");
        assert_eq!(decode(&text).expect("decodes"), m);
    }
}

#[test]
fn the_tag_field_is_what_discriminates() {
    let text = serde_json::to_string(&ClientMsg::Leave { doc: DOC.to_owned() }).expect("encodes");
    assert!(text.contains("\"t\":\"leave\""), "{text}");
}

#[test]
fn an_absent_scope_is_omitted_from_the_wire() {
    let m = ClientMsg::Submit { doc: DOC.to_owned(), local: 1, command: "c".to_owned(), params: json!({}), scope: None };
    let text = encode(&m).expect("encodes");
    assert!(!text.contains("scope"), "None must not serialise a null: {text}");
}

#[test]
fn garbage_decodes_to_an_error_not_a_panic() {
    for bad in ["", "{}", "not json", r#"{"t":"nope"}"#] {
        assert!(decode(bad).is_err(), "{bad:?} must be refused");
    }
}
