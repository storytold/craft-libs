//! An index-map section that points at an external map (`url`) adds no sources here, but its
//! sources are unknown rather than absent: the inventory says how many such sections it saw.

use craft_sourcemap::{InventoryLimits, Limits, SourceMap, parse, parse_sources};
use serde_json::{Value, json};

fn decode(value: Value) -> SourceMap {
    parse(&serde_json::to_vec(&value).unwrap(), Limits::default()).unwrap()
}

/// An external section adds no sources but is counted, so the inventory can say part of the map
/// was never read.
#[test]
fn an_external_section_is_counted_and_adds_no_sources() {
    let map = decode(json!({
        "version": 3,
        "sections": [
            {"offset": {"line": 0, "column": 0}, "map": {"version": 3, "sources": ["a.js"], "mappings": ""}},
            {"offset": {"line": 10, "column": 0}, "url": "vendor.js.map"}
        ]
    }));
    assert_eq!(map.all_sources(), ["a.js"]);
    assert_eq!(map.external_sections, 1);
    let inventory = map.source_inventory(InventoryLimits::default());
    assert_eq!(inventory.sources, ["a.js"]);
    assert!(!inventory.truncated);
    assert_eq!(inventory.external_sections, 1);
}

/// The count reaches an external section inside a nested index map.
#[test]
fn external_sections_are_counted_through_nested_index_maps() {
    let map = decode(json!({
        "version": 3,
        "sections": [
            {"offset": {"line": 0, "column": 0}, "map": {"version": 3, "sections": [
                {"offset": {"line": 0, "column": 0}, "map": {"version": 3, "sources": ["a.js"], "mappings": ""}},
                {"offset": {"line": 4, "column": 0}, "url": "inner.js.map"}
            ]}},
            {"offset": {"line": 20, "column": 0}, "url": "outer.js.map"}
        ]
    }));
    assert_eq!(map.all_sources(), ["a.js"]);
    assert_eq!(map.source_inventory(InventoryLimits::default()).external_sections, 2);
}

/// A map with only inline sections reports nothing unread.
#[test]
fn an_inline_only_map_counts_no_external_sections() {
    let map = decode(json!({
        "version": 3,
        "sections": [{"offset": {"line": 0, "column": 0}, "map": {"version": 3, "sources": ["a.js"], "mappings": ""}}]
    }));
    assert_eq!(map.source_inventory(InventoryLimits::default()).external_sections, 0);
}

/// A map read for its sources alone (its mappings are refused) still says an external section
/// left part of the list unread.
#[test]
fn a_sources_only_read_still_counts_external_sections() {
    let bytes = serde_json::to_vec(&json!({
        "version": 3,
        "sections": [
            {"offset": {"line": 0, "column": 0}, "map": {"version": 3, "sources": ["a.js"]}},
            {"offset": {"line": 8, "column": 0}, "url": "vendor.js.map"}
        ]
    }))
    .unwrap();
    let map = parse_sources(&bytes, Limits::default()).unwrap();
    assert_eq!(map.source_inventory(InventoryLimits::default()).external_sections, 1);
}

/// The count spends no inventory budget: a map with many external sections still retains its
/// inline sources in full.
#[test]
fn many_external_sections_do_not_spend_the_source_budget() {
    let mut sections: Vec<Value> = (0..1_000).map(|i| json!({"offset": {"line": i, "column": 0}, "url": format!("{i}.map")})).collect();
    sections.push(json!({"offset": {"line": 1_000, "column": 0}, "map": {"version": 3, "sources": ["a.js", "b.js"], "mappings": ""}}));
    let map = decode(json!({"version": 3, "sections": sections}));
    let inventory = map.source_inventory(InventoryLimits { max_sources: 2, max_bytes: 16 });
    assert_eq!(inventory.sources, ["a.js", "b.js"]);
    assert!(!inventory.truncated);
    assert_eq!(inventory.external_sections, 1_000);
}
