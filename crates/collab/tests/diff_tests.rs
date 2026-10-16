//! (Test code: assertions may panic — that is their point.)
#![allow(clippy::expect_used, clippy::unwrap_used, clippy::panic, clippy::cast_possible_truncation, clippy::cast_possible_wrap)]

//! The diff contract (spec §5.3): `apply(diff(a, b), a) == b` — the literal convergence
//! property ported from figcraft's `diff_tests.rs`, over randomly generated real edits.

use craft_collab::change::{Batch, Change};
use craft_collab::ids::NodeId;
use craft_collab::replica::{ApplyMode, Replica};
use craft_collab::sim::Rng;
use std::sync::Arc;

fn build(r: &Replica, space: u32, rng: &mut Rng) -> Vec<Change> {
    // A random real-ish edit: create a few nodes, edit one existing node's props.
    let mut changes = Vec::new();
    for _ in 0..=rng.below(3) {
        let id = NodeId::compose(space, (rng.below(10_000) + 1) as u32);
        changes.push(Change::Create {
            id,
            kind: Arc::from("rect"),
            parent: NodeId(0),
            position: craft_collab::frac::between(None, None).expect("key"),
            props: vec![
                (Arc::from("name"), craft_collab::value::Value::Json(serde_json::Value::String(format!("n{}", rng.below(50))))),
                (Arc::from("opacity"), craft_collab::value::Value::Json(serde_json::Value::from(rng.below(100) as i64))),
            ],
        });
    }
    if let Some((id, _)) = r.nodes().find(|_| rng.chance(1, 2)) {
        changes.push(Change::Set {
            id: *id,
            prop: Arc::from("name"),
            value: craft_collab::value::Value::Json(serde_json::Value::String(format!("renamed-{}", rng.below(9)))),
        });
    }
    changes
}

fn apply_changes(r: &mut Replica, changes: &[Change]) {
    let b = Batch { id: 0, base_seq: r.seq(), changes: changes.to_vec() };
    let _ = r.apply(&b, ApplyMode::Lenient);
}

/// The literal property: for randomly edited pairs, applying the diff of the edit to the
/// before-state equals the after-state — nodes, props, positions, children order (spec §5.3).
#[test]
fn diff_then_apply_equals_target() {
    for seed in 1..=300u64 {
        let mut rng = Rng::new(seed);
        let mut a = Replica::new();
        // Build up a random document.
        for _ in 0..10 + rng.below(20) {
            let changes = build(&a, 0, &mut rng);
            apply_changes(&mut a, &changes);
        }
        let before = a.clone();
        let mut b = a.clone();
        for _ in 0..5 {
            let changes = build(&b, 0, &mut rng);
            apply_changes(&mut b, &changes);
        }
        let batch = craft_collab::diff::diff(&before, &b, 0, 1, before.seq());
        let mut reconstructed = before.clone();
        let _ = reconstructed.apply(&batch, craft_collab::replica::ApplyMode::Strict);
        assert!(craft_collab::sim::replicas_equal(&reconstructed, &b), "diff roundtrip failed at seed {seed}");
    }
}

/// A pure deletion diff produces exactly one Delete per removed subtree root (spec §5.3).
#[test]
fn deletes_collapse_per_subtree() {
    let mut r = Replica::new();
    apply_changes(
        &mut r,
        &[
            Change::Create { id: NodeId(1), kind: Arc::from("g"), parent: NodeId(0), position: "a0".into(), props: vec![] },
            Change::Create { id: NodeId(2), kind: Arc::from("r"), parent: NodeId(1), position: "a0".into(), props: vec![] },
            Change::Create { id: NodeId(3), kind: Arc::from("r"), parent: NodeId(1), position: "a1".into(), props: vec![] },
            Change::Create { id: NodeId(4), kind: Arc::from("r"), parent: NodeId(0), position: "a1".into(), props: vec![] },
        ],
    );
    let before = r.clone();
    r.delete(NodeId(1)); // Takes 2 and 3 with it.
    let changes = craft_collab::diff::diff(&before, &r, 0, 1, before.seq());
    let deletes = changes.changes.iter().filter(|c| matches!(c, Change::Delete { .. })).count();
    assert_eq!(deletes, 1, "one Delete per subtree root, got {deletes}");
    assert!(matches!(changes.changes[0], Change::Delete { id: NodeId(1) }));
}
