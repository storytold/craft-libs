//! `craft bindgen` on rows captured from the real `filmcraft-cli commands --json`, plus the
//! edge cases the generator must handle.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use craft_cli::bindgen::{generate, parse_command_list, snake_case};
use serde_json::json;

fn sample() -> serde_json::Value {
    serde_json::from_str(include_str!("fixtures/filmcraft-commands.json")).unwrap()
}

#[test]
fn snake_case_handles_dots_camel_humps_and_digits() {
    assert_eq!(snake_case("file.newProject"), "file_new_project");
    assert_eq!(snake_case("clip.setTrack2"), "clip_set_track2");
    assert_eq!(snake_case("a--b..c"), "a_b_c");
    assert_eq!(snake_case("9lives"), "_9lives");
    assert_eq!(snake_case("URLParser.open"), "urlparser_open");
}

#[test]
fn rows_from_a_key_list_doc_get_a_struct_with_required_and_optional_fields() {
    let rows = parse_command_list(&json!([{ "id": "file.newBin", "label": "Bin", "params": "{\"name\":str,\"parent\":binId?}" }])).unwrap();
    assert_eq!(rows[0].source, "derived");
    let code = generate("filmcraft", &rows).unwrap();
    assert!(code.contains("pub struct FileNewBinParams {"), "{code}");
    assert!(code.contains("pub name: Value,"), "a required key is not an Option: {code}");
    assert!(code.contains("#[serde(skip_serializing_if = \"Option::is_none\")]\n    pub parent: Option<Value>,"), "{code}");
    assert!(
        code.contains("pub fn file_new_bin(params: &FileNewBinParams) -> Result<Value, OpError> {\n    cmd::execute(\"file.newBin\", params)"),
        "{code}"
    );
}

#[test]
fn a_json_schema_row_uses_its_types() {
    let rows = parse_command_list(&json!({ "commands": [{
        "id": "x.do",
        "schema": { "type": "object", "properties": { "n": { "type": "integer", "description": "How\nmany" }, "on": { "type": "boolean" }, "s": { "type": "string" }, "f": { "type": "number" }, "l": { "type": "array" } }, "required": ["n", "s"] }
    }] }))
    .unwrap();
    let code = generate("a", &rows).unwrap();
    for want in ["pub n: i64,", "pub s: String,", "pub on: Option<bool>,", "pub f: Option<f64>,", "pub l: Option<Vec<Value>>,", "/// How many"] {
        assert!(code.contains(want), "missing {want:?}:\n{code}");
    }
}

#[test]
fn keyword_and_renamed_fields_keep_their_wire_names() {
    let rows = parse_command_list(&json!([{ "id": "x.y", "schema": { "type": "object", "properties": { "type": { "type": "string" }, "importAs": { "type": "string" } }, "required": ["type"] } }])).unwrap();
    let code = generate("a", &rows).unwrap();
    assert!(code.contains("#[serde(rename = \"type\")]\n    pub type_: String,"), "{code}");
    assert!(code.contains("#[serde(rename = \"importAs\")]"), "{code}");
    assert!(code.contains("pub import_as: Option<String>,"), "{code}");
}

#[test]
fn an_opaque_doc_takes_plain_json_params() {
    let rows = parse_command_list(&json!([{ "id": "odd.cmd", "params": "free text, not a key list" }])).unwrap();
    assert_eq!(rows[0].source, "opaque");
    let code = generate("a", &rows).unwrap();
    assert!(code.contains("pub fn odd_cmd(params: &Value) -> Result<Value, OpError>"), "{code}");
    assert!(!code.contains("OddCmdParams"));
}

#[test]
fn colliding_names_are_an_error_that_names_both() {
    let rows = parse_command_list(&json!([{ "id": "a.bC", "params": "{}" }, { "id": "a.b.c", "params": "{}" }])).unwrap();
    let err = generate("x", &rows).unwrap_err().to_string();
    assert!(err.contains("a.bC") && err.contains("a.b.c") && err.contains("a_b_c"), "{err}");

    let rows = parse_command_list(&json!([{ "id": "p", "schema": { "type": "object", "properties": { "a-b": {}, "a_b": {} } } }])).unwrap();
    let err = generate("x", &rows).unwrap_err().to_string();
    assert!(err.contains("a-b") && err.contains("a_b"), "{err}");
}

#[test]
fn a_bad_list_is_an_error() {
    assert!(parse_command_list(&json!("nope")).is_err());
    assert!(parse_command_list(&json!({ "no": "commands" })).is_err());
    assert!(parse_command_list(&json!([{ "label": "no id" }])).is_err());
}

#[test]
fn the_real_filmcraft_sample_generates_one_function_per_command() {
    let rows = parse_command_list(&sample()).unwrap();
    assert_eq!(rows.len(), 45);
    let code = generate("filmcraft", &rows).unwrap();
    assert_eq!(code.matches("\npub fn ").count(), 45, "every command has a function");
    assert!(code.contains("pub fn file_new_sequence("), "{}", &code[..400]);
    assert!(code.contains("pub struct FileNewSequenceParams"));
    // `"fps":f64=23.976` has a default, but the generator only knows what the schema says.
    assert!(code.contains("pub name: Value,"));
}
