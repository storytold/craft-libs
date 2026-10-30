//! Panel trees: the wire shape and the two limits the host enforces.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use craft_plugin_sdk::json;
use craft_plugin_sdk::ui::{self, MAX_NODES, TreeError};

#[test]
fn a_tree_serializes_to_the_wire_shape() {
    let tree = ui::column("root")
        .child(ui::label("l", "Hi"))
        .child(ui::row("r").child(ui::checkbox("c", "On", true)).child(ui::slider("s", 0.0, 1.0, 0.5, 0.1)))
        .child(ui::dropdown("d", &["a", "b"], "a"))
        .child(ui::list("li", &["x"], None))
        .child(ui::separator("sep"));
    let v = tree.build().unwrap();
    assert_eq!(v["type"], "column");
    assert_eq!(v["id"], "root");
    assert_eq!(v["children"][0], json!({ "type": "label", "id": "l", "text": "Hi" }));
    assert_eq!(v["children"][1]["children"][0], json!({ "type": "checkbox", "id": "c", "label": "On", "value": true }));
    assert_eq!(v["children"][1]["children"][1], json!({ "type": "slider", "id": "s", "min": 0.0, "max": 1.0, "value": 0.5, "step": 0.1 }));
    assert_eq!(v["children"][2]["options"], json!(["a", "b"]));
    assert_eq!(v["children"][3]["selected"], json!(null));
    assert!(v["children"][4].get("children").is_none(), "a leaf has no children key");
}

#[test]
fn duplicate_and_empty_ids_are_rejected() {
    let dup = ui::column("a").child(ui::label("x", "1")).child(ui::label("x", "2"));
    assert_eq!(dup.build(), Err(TreeError::DuplicateId("x".into())));
    assert_eq!(ui::column("a").child(ui::label("", "1")).build(), Err(TreeError::EmptyId));
}

#[test]
fn the_node_limit_is_2000_inclusive() {
    let mut ok = ui::column("root");
    for i in 0..MAX_NODES - 1 {
        ok = ok.child(ui::label(&format!("n{i}"), "x"));
    }
    assert_eq!(ok.node_count(), MAX_NODES);
    assert!(ok.build().is_ok());
    let over = ok.child(ui::label("one-more", "x"));
    assert_eq!(over.build(), Err(TreeError::TooManyNodes { count: MAX_NODES + 1 }));
}

#[test]
fn prop_sets_extra_properties() {
    let v = ui::text("t", "hi").prop("multiline", true).build().unwrap();
    assert_eq!(v["multiline"], true);
    let form = ui::param_form("f", json!({ "type": "object" }), json!({})).build().unwrap();
    assert_eq!(form["schema"], json!({ "type": "object" }));
}
