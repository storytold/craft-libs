//! Pattern tiles: Figma keeps the layer a pattern paint tiles on the internal canvas, and it is
//! kept like the library copies the pages use, with what it holds.

#![allow(clippy::unwrap_used, clippy::panic)]

use super::tests::{arr, g, node, o, s};
use super::*;

/// The ids of every layer under `v`.
fn ids(v: &Value, out: &mut Vec<String>) {
    if let Some(id) = v["id"].as_str() {
        out.push(id.to_string());
    }
    for c in v["children"].as_array().into_iter().flatten() {
        ids(c, out);
    }
}

#[test]
fn the_layer_a_pattern_tiles_is_kept_from_the_internal_canvas() {
    let pattern = o(vec![("type", Kv::Enum("PATTERN")), ("sourceNodeId", g(30)), ("patternTileType", Kv::Enum("RECTANGULAR"))]);
    let nodes = vec![
        node(0, None, "DOCUMENT", "Document", vec![]),
        node(1, Some((0, "!")), "CANVAS", "Page", vec![]),
        node(2, Some((0, "~")), "CANVAS", "Internal Only Canvas", vec![("internalOnly", Kv::Bool(true))]),
        node(30, Some((2, "!")), "FRAME", "Group 1", vec![]),
        node(31, Some((30, "!")), "RECTANGLE", "Grid line", vec![]),
        node(32, Some((2, "\"")), "FRAME", "Unused", vec![]),
        node(10, Some((1, "!")), "RECTANGLE", "CTA", vec![("fillPaints", arr(vec![pattern]))]),
    ];
    let msg = o(vec![("nodeChanges", arr(nodes)), ("blobs", arr(vec![]))]);
    let out = to_rest(&msg, "Fixture", &Options::default()).json;
    let mut kept = Vec::new();
    for context in out[FIG_LIBRARY_CONTEXTS_FIELD].as_array().into_iter().flatten() {
        ids(context, &mut kept);
    }
    assert!(kept.contains(&"1:30".to_string()) && kept.contains(&"1:31".to_string()), "the tile and what it holds: {kept:?}");
    assert!(!kept.contains(&"1:32".to_string()), "nothing else from the internal canvas: {kept:?}");
    assert_eq!(super::tests::named(&out, "CTA")["fills"][0]["sourceNodeId"], "1:30");
}

#[test]
fn without_patterns_the_internal_canvas_keeps_nothing() {
    let nodes = vec![
        node(0, None, "DOCUMENT", "Document", vec![]),
        node(1, Some((0, "!")), "CANVAS", "Page", vec![]),
        node(2, Some((0, "~")), "CANVAS", "Internal Only Canvas", vec![("internalOnly", Kv::Bool(true))]),
        node(30, Some((2, "!")), "FRAME", "Group 1", vec![]),
        node(10, Some((1, "!")), "RECTANGLE", "CTA", vec![("fillPaints", arr(vec![s("not a paint")]))]),
    ];
    let msg = o(vec![("nodeChanges", arr(nodes)), ("blobs", arr(vec![]))]);
    let out = to_rest(&msg, "Fixture", &Options::default()).json;
    assert!(out.get(FIG_LIBRARY_CONTEXTS_FIELD).is_none(), "{}", out[FIG_LIBRARY_CONTEXTS_FIELD]);
}
