//! Cross-check the Rust manifest types against `schemas/plugin-manifest.schema.json`.
//! The schema is the reference. These tests fail if the types and the schema disagree.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use craft_plugin_abi::Manifest;
use serde_json::{Value, json};

fn validator() -> jsonschema::Validator {
    let path = format!("{}/schemas/plugin-manifest.schema.json", env!("CARGO_MANIFEST_DIR"));
    let schema: Value = serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
    jsonschema::options().should_validate_formats(true).build(&schema).unwrap()
}

fn good() -> Value {
    serde_json::from_str(include_str!("fixtures/manifest_good.json")).unwrap()
}

type Case = (&'static str, Box<dyn Fn(&mut Value)>);

fn errors(v: &jsonschema::Validator, doc: &Value) -> Vec<String> {
    v.iter_errors(doc).map(|e| format!("{} at {}", e, e.instance_path())).collect()
}

#[test]
fn the_good_example_satisfies_the_schema_and_the_types() {
    let v = validator();
    let doc = good();
    assert_eq!(errors(&v, &doc), Vec::<String>::new());
    Manifest::parse(&serde_json::to_vec(&doc).unwrap()).unwrap();
}

#[test]
fn a_parsed_manifest_serializes_to_something_the_schema_accepts() {
    let v = validator();
    let mut doc = good();
    doc["future_field"] = json!({"kept": true});
    doc["contributions"][0]["future_c"] = json!(7);
    let m = Manifest::parse(&serde_json::to_vec(&doc).unwrap()).unwrap();
    let out = serde_json::to_value(&m).unwrap();
    assert_eq!(errors(&v, &out), Vec::<String>::new(), "serialized: {out}");
    assert_eq!(out["future_field"], json!({"kept": true}));
    assert_eq!(out["contributions"][0]["future_c"], json!(7));
    // Re-parse is a fixed point.
    assert_eq!(Manifest::parse(&serde_json::to_vec(&out).unwrap()).unwrap(), m);
}

#[test]
fn a_minimal_manifest_serializes_without_invented_fields() {
    let v = validator();
    let doc = json!({
        "schema": 1, "id": "a", "name": "n", "version": "1",
        "abi": { "craft": 1 },
        "contributions": [{ "kind": "command", "id": "c", "entry": 0 }]
    });
    let m = Manifest::parse(&serde_json::to_vec(&doc).unwrap()).unwrap();
    let out = serde_json::to_value(&m).unwrap();
    assert_eq!(out, doc, "serialization must not add defaults or nulls");
    assert!(v.is_valid(&out));
}

/// Each bad document must be rejected by BOTH the schema and the Rust types.
#[test]
fn schema_and_types_reject_the_same_bad_documents() {
    let v = validator();
    let cases: Vec<Case> = vec![
        ("reserved prefix ec.", Box::new(|d| d["id"] = json!("ec.blur"))),
        ("reserved prefix pc.", Box::new(|d| d["id"] = json!("pc.blur"))),
        ("reserved prefix vc.", Box::new(|d| d["id"] = json!("vc.blur"))),
        ("reserved prefix craft.", Box::new(|d| d["id"] = json!("craft.blur"))),
        ("reserved prefix app.", Box::new(|d| d["id"] = json!("app.filmcraft"))),
        ("reserved prefix plugin.", Box::new(|d| d["id"] = json!("plugin.x"))),
        ("reserved prefix in capitals", Box::new(|d| d["id"] = json!("App.filmcraft"))),
        ("reserved prefix mixed case", Box::new(|d| d["id"] = json!("Craft.blur"))),
        ("reserved id app", Box::new(|d| d["id"] = json!("app"))),
        ("reserved id plugin", Box::new(|d| d["id"] = json!("PLUGIN"))),
        ("id with space", Box::new(|d| d["id"] = json!("a b"))),
        ("link.exec without glob", Box::new(|d| d["permissions"]["required"] = json!(["link.exec:filmcraft"]))),
        ("link.exec with empty glob", Box::new(|d| d["permissions"]["required"] = json!(["link.exec:filmcraft:"]))),
        ("link.exec with a bad app", Box::new(|d| d["permissions"]["required"] = json!(["link.exec:Film:x.*"]))),
        ("link.exec with a space in the glob", Box::new(|d| d["permissions"]["required"] = json!(["link.exec:filmcraft:a b"]))),
        ("id too long", Box::new(|d| d["id"] = json!("x".repeat(65)))),
        ("unknown permission", Box::new(|d| d["permissions"]["required"] = json!(["commands.exec"]))),
        ("http origin", Box::new(|d| d["permissions"]["optional"] = json!(["net.fetch:http://example.org"]))),
        ("doc.read bad scope", Box::new(|d| d["permissions"]["required"] = json!(["doc.read:all"]))),
        ("before hook without match", Box::new(|d| d["contributions"] = json!([{ "kind": "hook", "id": "h", "entry": 0, "phase": "before" }]))),
        ("hook without phase", Box::new(|d| d["contributions"] = json!([{ "kind": "hook", "id": "h", "entry": 0, "match": ["*"] }]))),
        ("format without extensions", Box::new(|d| d["contributions"] = json!([{ "kind": "format", "id": "f", "entry": 0 }]))),
        (
            "format with a dotted extension",
            Box::new(|d| d["contributions"] = json!([{ "kind": "format", "id": "f", "entry": 0, "extensions": [".xyz"] }])),
        ),
        ("schema 2", Box::new(|d| d["schema"] = json!(2))),
        ("abi.craft 2", Box::new(|d| d["abi"]["craft"] = json!(2))),
        ("codec cbor", Box::new(|d| d["abi"]["codec"] = json!("cbor"))),
        ("signature set", Box::new(|d| d["signature"] = json!({"alg": "ed25519"}))),
        ("legacy unknown", Box::new(|d| d["legacy"] = json!("gridcraft-v1"))),
        ("unknown feature", Box::new(|d| d["requires"]["features"] = json!(["relaxed-simd"]))),
        ("empty contributions", Box::new(|d| d["contributions"] = json!([]))),
        ("kind with capital", Box::new(|d| d["contributions"][1]["kind"] = json!("Teleport"))),
        ("kind with space", Box::new(|d| d["contributions"][1]["kind"] = json!("a b"))),
        ("kind too long", Box::new(|d| d["contributions"][1]["kind"] = json!("x".repeat(33)))),
        ("empty kind", Box::new(|d| d["contributions"][1]["kind"] = json!(""))),
        ("memory_mb 0", Box::new(|d| d["limits"]["memory_mb"] = json!(0))),
        ("memory_mb 4097", Box::new(|d| d["limits"]["memory_mb"] = json!(4097))),
        ("stack_kb 63", Box::new(|d| d["limits"]["stack_kb"] = json!(63))),
        ("overlap 257", Box::new(|d| d["contributions"][0]["process"]["overlap"] = json!(257))),
        ("band_bytes 65535", Box::new(|d| d["contributions"][0]["process"]["band_bytes"] = json!(65535))),
        ("negative entry", Box::new(|d| d["contributions"][0]["entry"] = json!(-1))),
        ("param name with dash", Box::new(|d| d["contributions"][0]["params"] = json!({ "a-b": { "type": "bool" } }))),
        ("param type unknown", Box::new(|d| d["contributions"][0]["params"]["bands"]["type"] = json!("matrix"))),
        ("contribution id with dot", Box::new(|d| d["contributions"][1]["id"] = json!("a.b"))),
        (
            "missing id",
            Box::new(|d| {
                d.as_object_mut().unwrap().remove("id");
            }),
        ),
        (
            "missing contributions",
            Box::new(|d| {
                d.as_object_mut().unwrap().remove("contributions");
            }),
        ),
    ];
    for (label, mutate) in cases {
        let mut doc = good();
        mutate(&mut doc);
        assert!(!v.is_valid(&doc), "{label}: the schema accepted it, so this case proves nothing");
        assert!(Manifest::parse(&serde_json::to_vec(&doc).unwrap()).is_err(), "{label}: the schema rejects it but the Rust types accept it");
    }
}

/// And the reverse for edge values the schema allows.
#[test]
fn schema_and_types_accept_the_same_edge_documents() {
    let v = validator();
    let cases: Vec<Case> = vec![
        ("after hook without match", Box::new(|d| d["contributions"] = json!([{ "kind": "hook", "id": "h", "entry": 0, "phase": "after" }]))),
        (
            "before hook with empty match",
            Box::new(|d| d["contributions"] = json!([{ "kind": "hook", "id": "h", "entry": 0, "phase": "before", "match": [] }])),
        ),
        (
            "format with extensions",
            Box::new(|d| d["contributions"] = json!([{ "kind": "format", "id": "f", "entry": 0, "extensions": ["xyz", "x1"] }])),
        ),
        ("memory_mb 4096", Box::new(|d| d["limits"]["memory_mb"] = json!(4096))),
        ("overlap 256", Box::new(|d| d["contributions"][0]["process"]["overlap"] = json!(256))),
        ("id 64 chars", Box::new(|d| d["id"] = json!("x".repeat(64)))),
        ("near-miss prefix", Box::new(|d| d["id"] = json!("ecx.blur"))),
        ("near-miss prefix apps.", Box::new(|d| d["id"] = json!("apps.blur"))),
        ("id that contains app.", Box::new(|d| d["id"] = json!("my.app.blur"))),
        ("link.exec permission", Box::new(|d| d["permissions"]["required"] = json!(["link.exec:filmcraft:effect.*"]))),
        ("frames.read permission", Box::new(|d| d["permissions"]["required"] = json!(["frames.read"]))),
        ("unknown kind", Box::new(|d| d["contributions"][1]["kind"] = json!("teleport"))),
        ("unknown kind, 32 chars", Box::new(|d| d["contributions"][1]["kind"] = json!("x".repeat(32)))),
        ("unknown top-level field", Box::new(|d| d["x_future"] = json!([1]))),
    ];
    for (label, mutate) in cases {
        let mut doc = good();
        mutate(&mut doc);
        assert_eq!(errors(&v, &doc), Vec::<String>::new(), "{label}: schema");
        Manifest::parse(&serde_json::to_vec(&doc).unwrap()).unwrap_or_else(|e| panic!("{label}: types reject what the schema accepts: {e}"));
    }
}
