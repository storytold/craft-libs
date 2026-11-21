//! Params docs copied from FilmCraft and EffectCraft, plus the two dialects
//! the parser has to tell apart.

use serde_json::{Value, json};

use craft_command::schema::{SchemaSource, from_doc};

#[test]
fn filmcraft_clip_nest_schema() {
    let doc = r#"{"name":str}"#;
    let got = from_doc(doc);
    assert_eq!(got.source, SchemaSource::Derived);
    assert_eq!(
        got.value,
        json!({
            "type": "object",
            "description": doc,
            "properties": { "name": { "description": r#""name":str"# } },
            "required": ["name"],
            "additionalProperties": false,
            "x-craft-source": "derived",
        })
    );
}

#[test]
fn question_mark_after_a_type_is_optional() {
    // FilmCraft `file.newBin`. The `?` sits after the type, not after the key.
    let doc = r#"{"name":str,"parent":binId?}"#;
    let got = from_doc(doc);
    assert_eq!(got.source, SchemaSource::Derived);
    assert_eq!(
        got.value,
        json!({
            "type": "object",
            "description": doc,
            "properties": {
                "name": { "description": r#""name":str"# },
                "parent": { "description": r#""parent":binId?"# },
            },
            "required": ["name"],
            "additionalProperties": false,
            "x-craft-source": "derived",
        })
    );
}

#[test]
fn effectcraft_layer_select_schema() {
    let doc = "{layers: [id|name|#n], add?, toggle?}";
    let got = from_doc(doc);
    assert_eq!(got.source, SchemaSource::Derived);
    assert_eq!(
        got.value,
        json!({
            "type": "object",
            "description": doc,
            "properties": {
                "layers": { "description": "layers: [id|name|#n]" },
                "add": { "description": "add?" },
                "toggle": { "description": "toggle?" },
            },
            "required": ["layers"],
            "additionalProperties": false,
            "x-craft-source": "derived",
        })
    );
}

#[test]
fn effectcraft_param_doc_examples() {
    // The key lists EffectCraft's `params_docs_parse` test locks in.
    let cases = [
        (
            "{time? (s) | frame? | timecode?}",
            json!([]),
            vec![
                ("time", "time? (s) | frame? | timecode?"),
                ("frame", "time? (s) | frame? | timecode?"),
                ("timecode", "time? (s) | frame? | timecode?"),
            ],
        ),
        (
            "{interpolation?|in?|out?: linear|bezier|hold, autoBezier?}",
            json!([]),
            vec![
                ("interpolation", "interpolation?|in?|out?: linear|bezier|hold"),
                ("in", "interpolation?|in?|out?: linear|bezier|hold"),
                ("out", "interpolation?|in?|out?: linear|bezier|hold"),
                ("autoBezier", "autoBezier?"),
            ],
        ),
        ("{keys: [{layer, prop, time}], add?}", json!(["keys"]), vec![("keys", "keys: [{layer, prop, time}]"), ("add", "add?")]),
        ("{label: Red|Yellow|Aqua|\u{2026}, layers?}", json!(["label"]), vec![("label", "label: Red|Yellow|Aqua|\u{2026}"), ("layers", "layers?")]),
        (
            "{name?, background? [r,g,b]|#hex, open?}",
            json!([]),
            vec![("name", "name?"), ("background", "background? [r,g,b]|#hex"), ("open", "open?")],
        ),
    ];
    for (doc, required, props) in cases {
        let got = from_doc(doc);
        assert_eq!(got.source, SchemaSource::Derived, "{doc}");
        assert_eq!(got.value["x-craft-source"], "derived", "{doc}");
        assert_eq!(got.value["required"], required, "{doc}");
        assert_eq!(got.value["additionalProperties"], false, "{doc}");
        assert_eq!(got.value["description"], doc, "{doc}");
        let obj = got.value["properties"].as_object().expect("properties");
        assert_eq!(obj.len(), props.len(), "{doc}");
        for (key, piece) in props {
            assert_eq!(obj[key]["description"], piece, "{doc} key {key}");
        }
    }
}

#[test]
fn nested_question_mark_does_not_make_the_outer_key_optional() {
    let doc = "{target: {layer, path|prop, dimension?}, points: [[x,y,pressure?]]}";
    let got = from_doc(doc);
    assert_eq!(got.value["required"], json!(["target", "points"]));
    let props = got.value["properties"].as_object().expect("properties");
    assert_eq!(props.len(), 2);
    assert!(props.get("dimension").is_none());
    assert!(props.get("pressure").is_none());
    assert_eq!(props["target"]["description"], "target: {layer, path|prop, dimension?}");
    assert_eq!(props["points"]["description"], "points: [[x,y,pressure?]]");
}

#[test]
fn empty_key_list_is_derived_not_opaque() {
    let doc = "{}";
    assert_eq!(
        from_doc(doc).value,
        json!({
            "type": "object",
            "description": "{}",
            "properties": {},
            "required": [],
            "additionalProperties": false,
            "x-craft-source": "derived",
        })
    );
}

#[test]
fn a_doc_that_is_not_a_key_list_is_opaque() {
    for doc in ["free text", "{not closed", ""] {
        let got = from_doc(doc);
        assert_eq!(got.source, SchemaSource::Opaque, "{doc:?}");
        assert_eq!(
            got.value,
            json!({
                "type": "object",
                "description": doc,
                "additionalProperties": true,
                "x-craft-source": "opaque",
            }),
            "{doc:?}"
        );
    }
}

#[test]
fn a_json_schema_object_keeps_its_properties() {
    let doc = r#"{"type":"object","properties":{"name":{"type":"string"}},"required":["name"]}"#;
    let got = from_doc(doc);
    assert_eq!(got.source, SchemaSource::Doc);
    assert_eq!(got.value["x-craft-source"], "doc");
    assert_eq!(got.value["type"], "object");
    assert_eq!(got.value["properties"]["name"]["type"], "string");
    assert_eq!(got.value["required"], json!(["name"]));
    // Re-deriving would have invented keys named "type" and "properties".
    assert!(got.value["properties"].get("type").is_none());
    assert!(got.value["properties"].get("properties").is_none());
}

#[test]
fn an_embedded_source_tag_does_not_override_a_schema_object() {
    let doc = r#"{"type":"object","properties":{},"x-craft-source":"opaque"}"#;
    let got = from_doc(doc);
    assert_eq!(got.source, SchemaSource::Doc);
    assert_eq!(got.value["x-craft-source"], "doc");
}

#[test]
fn real_param_docs_match_parsed_schema() {
    check_fixture(include_str!("fixtures/filmcraft.json"));
    check_fixture(include_str!("fixtures/effectcraft.json"));
}

fn check_fixture(text: &str) {
    let file: Value = serde_json::from_str(text).expect("fixture json");
    let commands = file["commands"].as_array().expect("commands");
    assert_eq!(commands.len(), 20, "each app contributes 20 real param docs");
    for row in commands {
        let id = row["id"].as_str().expect("id");
        let params = row["params"].as_str().expect("params");
        let got = from_doc(params);
        assert!(got.source == SchemaSource::Derived || got.source == SchemaSource::Doc, "{id} parsed as {:?}", got.source);
        assert_eq!(got.value["x-craft-source"], got.source.as_str(), "{id}");
        assert_eq!(got.source, SchemaSource::Derived, "{id}");
        assert_eq!(got.value["type"], "object", "{id}");
        assert_eq!(got.value["description"], params, "{id}");
        assert_eq!(got.value["additionalProperties"], false, "{id}");
        assert_eq!(got.value["required"], row["required"], "{id}");
        let props = got.value["properties"].as_object().expect("properties");
        let expected = row["properties"].as_array().expect("expected properties");
        assert_eq!(props.len(), expected.len(), "{id}");
        for pair in expected {
            let key = pair[0].as_str().expect("key");
            assert_eq!(props[key]["description"], pair[1], "{id} property {key}");
        }
    }
}
