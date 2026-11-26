use craft_sourcemap::{Limits, SourceMap, parse};
use serde_json::{Value, json};

fn decode(value: Value) -> SourceMap {
    parse(&serde_json::to_vec(&value).unwrap(), Limits::default()).unwrap()
}

#[test]
fn root_prefix_precedes_absolute_looking_sources() {
    for (root, source, expected) in [
        ("webpack:///", "/src/main.js", "webpack:////src/main.js"),
        ("https://example.test/build", "/src/main.js", "https://example.test/build//src/main.js"),
        ("src", "https://example.test/main.js", "src/https://example.test/main.js"),
        ("src/", "//example.test/main.js", "src///example.test/main.js"),
        ("sources", "node:internal/main", "sources/node:internal/main"),
    ] {
        let map = decode(json!({"version": 3, "sourceRoot": root, "sources": [source], "mappings": "AAAA"}));
        assert_eq!(map.original_positions(0, 0)[0].source.as_deref(), Some(expected), "root {root:?}, source {source:?}");
        assert_eq!(map.all_sources(), [expected]);
    }
}

#[test]
fn empty_root_keeps_the_same_identity_as_an_absent_or_null_root() {
    for source in ["src/main.js", "/src/main.js", "https://example.test/main.js"] {
        let mut value = json!({"version": 3, "sources": [source], "mappings": "AAAA"});
        assert_eq!(decode(value.clone()).all_sources(), [source]);
        value["sourceRoot"] = Value::Null;
        assert_eq!(decode(value.clone()).all_sources(), [source]);
        value["sourceRoot"] = json!("");
        let map = decode(value);
        assert_eq!(map.all_sources(), [source]);
        assert_eq!(map.original_positions(0, 0)[0].source.as_deref(), Some(source));
    }
}

#[test]
fn each_source_keeps_its_index_and_embedded_content() {
    let map = decode(json!({
        "version": 3, "sourceRoot": "https://example.test/originals/",
        "sources": ["/first.js", null, "https://other.test/third.js"],
        "sourcesContent": ["first();", null, "third();"], "mappings": "AAAA,CEAA"
    }));
    let first = map.original_positions(0, 0);
    let third = map.original_positions(0, 1);
    assert_eq!(first[0].source_index, 0);
    assert_eq!(third[0].source_index, 2);
    assert_eq!(first[0].source.as_deref(), Some("https://example.test/originals//first.js"));
    assert_eq!(third[0].source.as_deref(), Some("https://example.test/originals/https://other.test/third.js"));
    assert_eq!(map.source_content(0), Some("first();"));
    assert_eq!(map.source_content(2), Some("third();"));
    assert_eq!(map.source_path(1), None);
    assert_eq!(map.source_path(3), None);
}

#[test]
fn indexed_maps_use_each_embedded_maps_own_root() {
    let map = decode(json!({
        "version": 3, "sourceRoot": "outer", "sections": [
            {"offset": {"line": 0, "column": 3}, "map": {
                "version": 3, "sourceRoot": "file:///", "sources": ["/first.js"], "mappings": "AAAA"
            }},
            {"offset": {"line": 1, "column": 0}, "map": {
                "version": 3, "sources": ["second.js"], "mappings": "AAAA"
            }}
        ]
    }));
    assert_eq!(map.original_positions(0, 3)[0].source.as_deref(), Some("file:////first.js"));
    assert_eq!(map.original_positions(1, 0)[0].source.as_deref(), Some("second.js"));
    assert_eq!(map.all_sources(), ["file:////first.js", "second.js"]);
}

#[test]
fn relative_sources_still_preserve_url_root_delimiters() {
    for (root, expected) in [("webpack:///", "webpack:///main.js"), ("file:///", "file:///main.js"), ("src", "src/main.js"), ("src/", "src/main.js")]
    {
        let map = decode(json!({"version": 3, "sourceRoot": root, "sources": ["main.js"], "mappings": "AAAA"}));
        assert_eq!(map.all_sources(), [expected]);
    }
}
