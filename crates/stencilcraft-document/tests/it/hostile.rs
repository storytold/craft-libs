//! Hostile input: malformed, wrong-format, future-version, duplicate-id and
//! dangling-membership files all fail or recover with a `DocError`, never a panic.

use craft_stencilcraft_document::{DocError, Document, Operation, OperationKind, OperationSettings};
use kurbo::BezPath;

fn rect_path() -> BezPath {
    let mut p = BezPath::new();
    p.move_to((10.0, 20.0));
    p.line_to((70.0, 20.0));
    p.line_to((70.0, 80.0));
    p.line_to((10.0, 80.0));
    p.close_path();
    p
}

/// A valid v2 document with one cut shape, returned as parsed JSON so tests can
/// mutate it field by field before loading.
fn valid_v2_value() -> Result<serde_json::Value, Box<dyn std::error::Error>> {
    let mut doc = Document::default();
    doc.push_shape(0, rect_path())?;
    doc.operations.push(Operation {
        id: 1,
        name: "cut".to_string(),
        kind: OperationKind::Cut,
        settings: OperationSettings::default(),
        color: None,
        visible: true,
        locked: false,
        members: vec![1],
        icon: None,
    });
    let json = doc.save_json()?;
    Ok(serde_json::from_str(&json)?)
}

fn load_value(v: &serde_json::Value) -> Result<Document, DocError> {
    let json = serde_json::to_string(v).map_err(|e| DocError::Parse { path: "input".to_string(), source: e })?;
    Document::load_json(&json)
}

#[test]
fn malformed_json_is_a_parse_error_not_a_crash() {
    for junk in ["{ not json", "", "[1, 2,", "\"unterminated"] {
        assert!(matches!(Document::load_json(junk), Err(DocError::Parse { .. })), "{junk:?}: malformed JSON maps to DocError::Parse");
    }
}

#[test]
fn non_stencilcraft_header_is_rejected() {
    for json in [
        r#"{"stencilcraft":{"name":"other","version":2},"board":{},"kerf":{},"layers":[]}"#,
        r#"{"stencilcraft":{"name":"stencilcraft","version":"2"},"board":{},"kerf":{},"layers":[]}"#,
        r#"{"board":{},"kerf":{},"layers":[]}"#,
        r#"{}"#,
        r#"[]"#,
        r#""scalar""#,
    ] {
        assert!(matches!(Document::load_json(json), Err(DocError::NotAStencilcraftFile { .. })), "{json}: no stencilcraft header");
    }
}

#[test]
fn future_version_header_is_rejected() {
    let json = r#"{"stencilcraft":{"name":"stencilcraft","version":3},"board":{},"kerf":{},"layers":[]}"#;
    assert!(
        matches!(Document::load_json(json), Err(DocError::FormatVersion { found: 3, .. })),
        "a v3 header is rejected so old builds never read v2 (D-13)"
    );
}

#[test]
fn unknown_top_level_fields_are_ignored() -> Result<(), Box<dyn std::error::Error>> {
    let mut v = valid_v2_value()?;
    v.as_object_mut().map(|o| o.insert("future-field".to_string(), serde_json::json!({"anything": true})));
    let doc = load_value(&v)?;
    assert_eq!(doc.shape_count(), 1, "the document loads despite the unknown field");
    Ok(())
}

#[test]
fn duplicate_shape_ids_are_a_model_error() -> Result<(), Box<dyn std::error::Error>> {
    let mut v = valid_v2_value()?;
    let shapes = v["shapes"].as_array().cloned().unwrap_or_default();
    let dup = shapes.first().cloned().unwrap_or(serde_json::Value::Null);
    v["shapes"] = serde_json::json!([dup, dup]);
    assert!(matches!(load_value(&v), Err(DocError::Model(_))), "duplicate shape ids fail validation");
    Ok(())
}

#[test]
fn dangling_member_id_is_pruned_and_load_succeeds() -> Result<(), Box<dyn std::error::Error>> {
    let mut v = valid_v2_value()?;
    v["operations"][0]["members"] = serde_json::json!([1, 999]);
    let doc = load_value(&v)?;
    assert_eq!(doc.operations[0].members, vec![1], "the dangling member id is pruned, not fatal");
    Ok(())
}

#[test]
fn non_finite_power_or_zero_passes_fail_save_with_model_error() -> Result<(), Box<dyn std::error::Error>> {
    let doc = valid_v2_value()?;
    let doc = load_value(&doc)?;

    let mut bad_power = doc.clone();
    bad_power.operations[0].settings.power = f64::INFINITY;
    assert!(matches!(bad_power.save_json(), Err(DocError::Model(_))), "non-finite power fails save_json");

    let mut zero_passes = doc.clone();
    zero_passes.operations[0].settings.passes = 0;
    assert!(matches!(zero_passes.save_json(), Err(DocError::Model(_))), "zero passes fails save_json");

    // The untouched document still saves fine.
    assert!(doc.save_json().is_ok());
    Ok(())
}
