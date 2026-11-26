use craft_sourcemap::{InventoryLimits, Limits, SourceMap, parse};
use serde_json::{Value, json};

fn decode(value: Value) -> SourceMap {
    parse(&serde_json::to_vec(&value).unwrap(), Limits::default()).unwrap()
}

#[test]
fn all_sources_bounds_expanded_path_bytes_before_collecting() {
    let root = "r".repeat(2 * 1024 * 1024);
    let bytes = serde_json::to_vec(&json!({
        "version": 3, "sourceRoot": root, "sources": vec!["/x.js"; 40], "mappings": ""
    }))
    .unwrap();
    let map = parse(&bytes, Limits::default()).unwrap();
    let sources = map.all_sources();
    assert!(sources.iter().map(String::len).sum::<usize>() <= 64 * 1024 * 1024);
    assert_eq!(sources.len(), 31);
    assert!(sources.iter().all(|source| source.ends_with("//x.js")));
}

#[test]
fn repeated_large_roots_stop_before_relative_and_absolute_looking_paths_expand() {
    for source in ["x.js", "/x.js", "https://example.test/x.js"] {
        let bytes = serde_json::to_vec(&json!({
            "version": 3, "sourceRoot": "r".repeat(1024 * 1024),
            "sources": vec![source; 10_000], "mappings": ""
        }))
        .unwrap();
        assert!(bytes.len() < 2 * 1024 * 1024);
        let map = parse(&bytes, Limits::default()).unwrap();
        let inventory = map.source_inventory(InventoryLimits { max_sources: 5000, max_bytes: 2 * 1024 * 1024 });
        assert_eq!(inventory.sources.len(), 1);
        assert_eq!(inventory.sources[0].len(), 1024 * 1024 + 1 + source.len());
        assert!(inventory.sources[0].ends_with(source));
        assert!(inventory.truncated);
    }
}

#[test]
fn byte_limits_preserve_complete_utf8_identities_at_the_exact_boundary() {
    let map = decode(json!({"version": 3, "sourceRoot": "é/", "sources": ["東京.js", "東京.js", null], "mappings": ""}));
    let expected = "é/東京.js";
    let limits = InventoryLimits { max_sources: 2, max_bytes: 2 * expected.len() };
    let exact = map.source_inventory(limits);
    assert_eq!(exact.sources, [expected, expected]);
    assert!(!exact.truncated);
    let short = map.source_inventory(InventoryLimits { max_bytes: limits.max_bytes - 1, ..limits });
    assert_eq!(short.sources, [expected]);
    assert!(short.truncated);
    let too_small = map.source_inventory(InventoryLimits { max_bytes: expected.len() - 1, ..limits });
    assert!(too_small.sources.is_empty());
    assert!(too_small.truncated);
}

#[test]
fn entry_limits_count_empty_paths_but_not_null_sources() {
    let map = decode(json!({"version": 3, "sourceRoot": "", "sources": [null, "", "", null], "mappings": ""}));
    let exact = map.source_inventory(InventoryLimits { max_sources: 2, max_bytes: 0 });
    assert_eq!(exact.sources, ["", ""]);
    assert!(!exact.truncated);
    let one = map.source_inventory(InventoryLimits { max_sources: 1, max_bytes: 0 });
    assert_eq!(one.sources, [""]);
    assert!(one.truncated);
    let zero = map.source_inventory(InventoryLimits { max_sources: 0, max_bytes: 0 });
    assert!(zero.sources.is_empty());
    assert!(zero.truncated);
    let absent = decode(json!({"version": 3, "sources": [null], "mappings": ""}));
    assert!(!absent.source_inventory(InventoryLimits { max_sources: 0, max_bytes: 0 }).truncated);
}

#[test]
fn indexed_sections_share_entry_and_byte_limits_in_source_order() {
    let map = decode(json!({"version": 3, "sourceRoot": "outer", "sections": [
        {"offset": {"line": 0, "column": 0}, "map": {
            "version": 3, "sourceRoot": "src", "sources": [null, "/first.js"], "mappings": ""
        }},
        {"offset": {"line": 1, "column": 0}, "url": "external.map"},
        {"offset": {"line": 2, "column": 0}, "map": {"version": 3, "sections": [
            {"offset": {"line": 0, "column": 0}, "map": {
                "version": 3, "sourceRoot": "file:///", "sources": ["second.js", "third.js"], "mappings": ""
            }}
        ]}}
    ]}));
    let expected = ["src//first.js", "file:///second.js", "file:///third.js"];
    let complete = map.source_inventory(InventoryLimits { max_sources: 3, max_bytes: expected.iter().map(|s| s.len()).sum() });
    assert_eq!(complete.sources, expected);
    assert!(!complete.truncated);
    let bytes = map.source_inventory(InventoryLimits { max_sources: 3, max_bytes: expected[0].len() + expected[1].len() });
    assert_eq!(bytes.sources, expected[..2]);
    assert!(bytes.truncated);
    let entries = map.source_inventory(InventoryLimits { max_sources: 1, max_bytes: 100 });
    assert_eq!(entries.sources, expected[..1]);
    assert!(entries.truncated);
}

#[test]
fn an_oversized_path_stops_the_prefix_without_shortening_or_skipping_identities() {
    let map = decode(json!({"version": 3, "sources": ["a.js", "b".repeat(40), "c.js"], "mappings": ""}));
    let inventory = map.source_inventory(InventoryLimits { max_sources: 3, max_bytes: 8 });
    assert_eq!(inventory.sources, ["a.js"]);
    assert!(inventory.truncated);
    assert_eq!(map.source_path(1).unwrap(), "b".repeat(40));
}
