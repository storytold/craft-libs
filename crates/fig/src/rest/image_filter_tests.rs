//! Malformed image paint filters are skipped with a warning, image hashes too long to name
//! an image aren't spelled out, and pattern paints keep the layer they tile.

#![allow(clippy::unwrap_used, clippy::panic)]

use super::*;

#[test]
fn malformed_non_finite_filter_is_skipped_with_warning() {
    let paint = Kv::Object(vec![("type", Kv::Enum("IMAGE")), ("paintFilter", Kv::Object(vec![("exposure", Kv::Float(f32::NAN))]))]);
    let mut warnings = Vec::new();
    let value = super::paint(&paint, &mut |warning| warnings.push(warning)).unwrap();
    assert!(value.get("filters").is_none(), "{value}");
    assert!(warnings.iter().any(|warning| warning.contains("malformed image filter `exposure`")), "{warnings:?}");
}

#[test]
fn boolean_filter_values_are_malformed_even_when_false() {
    for (field, value, expected_warning) in [
        ("exposure", Kv::Bool(true), "malformed image filter `exposure`"),
        ("exposure", Kv::Bool(false), "malformed image filter `exposure`"),
        ("mystery", Kv::Bool(false), "malformed unsupported image filter `mystery`"),
    ] {
        let paint = Kv::Object(vec![("type", Kv::Enum("IMAGE")), ("paintFilter", Kv::Object(vec![(field, value)]))]);
        let mut warnings = Vec::new();
        let value = super::paint(&paint, &mut |warning| warnings.push(warning)).unwrap();
        assert!(value.get("filters").is_none(), "{value}");
        assert!(warnings.iter().any(|warning| warning.contains(expected_warning)), "{warnings:?}");
    }
}

#[test]
fn an_oversized_image_hash_is_not_spelled_out() {
    let paint = |len: usize| Kv::Object(vec![("type", Kv::Enum("IMAGE")), ("image", Kv::Object(vec![("hash", Kv::Bytes(vec![0xab; len]))]))]);
    let image_ref = |len: usize| super::paint(&paint(len), &mut |_| {}).and_then(|p| p.get("imageRef").cloned());
    assert_eq!(image_ref(20), Some(Value::String("ab".repeat(20))));
    assert_eq!(image_ref(crate::MAX_IMAGE_HASH), Some(Value::String("ab".repeat(crate::MAX_IMAGE_HASH))));
    assert_eq!(image_ref(crate::MAX_IMAGE_HASH + 1), Some(Value::String(String::new())));
}

#[test]
fn a_pattern_paint_keeps_its_source_layer_and_tiling() {
    let gap = Kv::Object(vec![("x", Kv::Float(4.0)), ("y", Kv::Float(2.0))]);
    let source = Kv::Object(vec![("sessionID", Kv::Uint(13)), ("localID", Kv::Uint(100))]);
    let paint = Kv::Object(vec![
        ("type", Kv::Enum("PATTERN")),
        ("opacity", Kv::Float(0.07)),
        ("sourceNodeId", source),
        ("patternTileType", Kv::Enum("HORIZONTAL_HEXAGONAL")),
        ("scale", Kv::Float(0.5)),
        ("patternSpacing", gap),
        ("horizontalAlignment", Kv::Enum("CENTER")),
    ]);
    let value = super::paint(&paint, &mut |w| panic!("{w}")).unwrap();
    assert_eq!(value["sourceNodeId"], "13:100");
    assert_eq!(value["tileType"], "HORIZONTAL_HEXAGONAL");
    assert_eq!(value["scalingFactor"], 0.5);
    assert_eq!(value["spacing"], json!({"x": 4.0, "y": 2.0}));
    assert_eq!(value["horizontalAlignment"], "CENTER");
    // Without them it is the bare paint, as before.
    let bare = super::paint(&Kv::Object(vec![("type", Kv::Enum("PATTERN"))]), &mut |_| {}).unwrap();
    assert_eq!(bare, json!({"type": "PATTERN"}));
}
