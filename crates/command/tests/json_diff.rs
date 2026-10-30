use serde_json::{Value, json};

use craft_command::json_diff::{Op, Patch, Step, diff, patch};

fn key(name: &str) -> Step {
    Step::Key(name.to_string())
}

fn idx(index: usize) -> Step {
    Step::Index(index)
}

fn set(path: Vec<Step>, value: Value) -> Op {
    Op { path, value: Some(value) }
}

fn remove(path: Vec<Step>) -> Op {
    Op { path, value: None }
}

#[test]
fn object_add_change_and_remove() {
    let old = json!({"a": 1, "gone": true, "n": null});
    let new = json!({"a": 2, "n": null, "added": 3});
    // Object keys are visited in sorted order (`serde_json::Map` is a `BTreeMap`).
    let ops = diff(&old, &new);
    assert_eq!(ops.ops(), &[set(vec![key("a")], json!(2)), remove(vec![key("gone")]), set(vec![key("added")], json!(3)),]);
    let mut doc = old.clone();
    patch(&mut doc, &ops).expect("patch");
    assert_eq!(doc, new);
    assert!(doc.get("gone").is_none());
    assert!(doc["n"].is_null());
}

#[test]
fn null_survives_and_is_not_a_deletion() {
    let old = json!({"g": null, "a": 1});
    let new = json!({"g": null, "a": 1, "h": {"i": null}});
    let ops = diff(&old, &new);
    assert_eq!(ops.ops(), &[set(vec![key("h")], json!({"i": null}))]);
    let mut doc = old.clone();
    patch(&mut doc, &ops).expect("patch");
    assert_eq!(doc, new);
    assert!(doc["g"].is_null());
    assert!(doc["h"]["i"].is_null());
}

#[test]
fn equal_length_arrays_diff_element_wise() {
    let old = json!([1, null, {"c": 1}]);
    let new = json!([1, null, {"c": 4}]);
    let ops = diff(&old, &new);
    assert_eq!(ops.ops(), &[set(vec![idx(2), key("c")], json!(4))]);
    let mut doc = old.clone();
    patch(&mut doc, &ops).expect("patch");
    assert_eq!(doc, new);
    assert!(doc[1].is_null());
}

#[test]
fn different_length_arrays_are_replaced() {
    let old = json!({"b": [1, 2]});
    let new = json!({"b": [1, 2, 3]});
    let ops = diff(&old, &new);
    assert_eq!(ops.ops(), &[set(vec![key("b")], json!([1, 2, 3]))]);
    let mut doc = old.clone();
    patch(&mut doc, &ops).expect("patch");
    assert_eq!(doc, new);

    let root_old = json!([1, 2]);
    let root_new = json!([1]);
    let root = diff(&root_old, &root_new);
    assert_eq!(root.ops(), &[set(vec![], json!([1]))]);
    let mut doc = root_old;
    patch(&mut doc, &root).expect("patch root");
    assert_eq!(doc, root_new);
}

#[test]
fn patch_of_diff_round_trips_the_upstream_document() {
    let old = json!({"a": 1, "b": [1, 2, {"c": null}], "d": {"e": "x", "f": [1]}, "g": null});
    let new = json!({"a": 2, "b": [1, 3, {"c": 4}], "d": {"f": [1, 2]}, "g": null, "h": {"i": null}});
    let ops = diff(&old, &new);
    assert_eq!(
        ops.ops(),
        &[
            set(vec![key("a")], json!(2)),
            set(vec![key("b"), idx(1)], json!(3)),
            set(vec![key("b"), idx(2), key("c")], json!(4)),
            remove(vec![key("d"), key("e")]),
            set(vec![key("d"), key("f")], json!([1, 2])),
            set(vec![key("h")], json!({"i": null})),
        ]
    );
    let mut doc = old.clone();
    patch(&mut doc, &ops).expect("forward");
    assert_eq!(doc, new);
    let mut back = new.clone();
    patch(&mut back, &diff(&new, &old)).expect("reverse");
    assert_eq!(back, old);
    assert!(diff(&old, &old).is_empty());
    assert!(doc["g"].is_null());
    assert!(doc["h"]["i"].is_null());
}

#[test]
fn op_json_uses_untagged_steps_and_omits_a_missing_value() {
    let changed = set(vec![key("b"), idx(1)], json!(9));
    assert_eq!(serde_json::to_value(&changed).expect("value"), json!({"path": ["b", 1], "value": 9}));
    let parsed: Op = serde_json::from_value(json!({"path": ["b", 1], "value": 9})).expect("op");
    assert_eq!(parsed, changed);

    let removed = remove(vec![key("d")]);
    assert_eq!(serde_json::to_value(&removed).expect("value"), json!({"path": ["d"]}));
    let parsed: Op = serde_json::from_value(json!({"path": ["d"]})).expect("op");
    assert_eq!(parsed, removed);
}

#[test]
fn patch_reports_a_path_that_does_not_match() {
    let old = json!({"a": 1, "b": [1]});
    let new = json!({"a": 2});
    let err = patch(&mut json!(3), &diff(&old, &new)).expect_err("mismatch");
    assert_eq!(err.message(), "path does not match the document");

    let missing = Patch::new(vec![set(vec![key("a"), key("b")], json!(1))]);
    let err = patch(&mut json!({"z": 1}), &missing).expect_err("missing member");
    assert_eq!(err.message(), "no member `a`");

    let root = Patch::new(vec![Op { path: vec![], value: None }]);
    let err = patch(&mut json!(1), &root).expect_err("remove root");
    assert_eq!(err.message(), "cannot remove the document");
}
