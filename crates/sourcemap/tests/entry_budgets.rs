use craft_sourcemap::{Limits, SourceMap, SourceMapError, parse};
use serde_json::json;

fn with(max_entries: usize) -> Limits {
    Limits { max_entries, ..Limits::default() }
}

fn decode(value: serde_json::Value, max_entries: usize) -> Result<SourceMap, SourceMapError> {
    parse(&serde_json::to_vec(&value).unwrap(), with(max_entries))
}

#[test]
fn entries_up_to_the_limit_are_kept() {
    let value = json!({"version":3,"sources":["a.js","b.js"],"sourcesContent":["a();",null],"names":["x"],"mappings":"AAAAA,CCAA"});
    let map = decode(value.clone(), 5).unwrap();
    assert_eq!(map.sources, vec![Some("a.js".to_string()), Some("b.js".to_string())]);
    assert_eq!(map.sources_content, vec![Some("a();".to_string()), None]);
    assert_eq!(map.names, vec!["x".to_string()]);
    assert_eq!(decode(value, 4), Err(SourceMapError::TooManyEntries(4)));
}

#[test]
fn every_kind_of_entry_counts_once() {
    for value in [
        json!({"version":3,"sources":["a","b","c"],"mappings":""}),
        json!({"version":3,"sourcesContent":["a","b","c"],"mappings":""}),
        json!({"version":3,"names":["a","b","c"],"mappings":""}),
        json!({"version":3,"ignoreList":[0,1,2],"mappings":""}),
        json!({"version":3,"x_google_ignoreList":[0,1,2],"mappings":""}),
        // Entries that are not strings still take a slot each.
        json!({"version":3,"sources":[null, 1, {"a":[1,2,3]}],"mappings":""}),
    ] {
        assert_eq!(decode(value.clone(), 2), Err(SourceMapError::TooManyEntries(2)), "{value}");
        assert!(decode(value.clone(), 3).is_ok(), "{value}");
    }
}

#[test]
fn entries_a_repeated_key_replaces_give_their_slots_back() {
    let json = br#"{"version":3,"mappings":"","sources":["a","b"],"sources":["c"],"names":["x"],"names":[]}"#;
    let map = parse(json, with(2)).unwrap();
    assert_eq!(map.sources, vec![Some("c".to_string())]);
    assert!(map.names.is_empty());
    // `ignoreList` is kept over `x_google_ignoreList`, before or after it.
    let first = br#"{"version":3,"mappings":"","x_google_ignoreList":[0,0],"ignoreList":[0]}"#;
    assert_eq!(parse(first, with(2)).unwrap().ignore_list, vec![0]);
    let after = br#"{"version":3,"mappings":"","ignoreList":[0],"x_google_ignoreList":[0,0,0]}"#;
    assert_eq!(parse(after, with(1)).unwrap().ignore_list, vec![0]);
    // A replaced sections array gives back its sections and what their maps held: without that,
    // the second array would need slots four and five.
    let sections = br#"{"version":3,"sections":[{"offset":{"line":0,"column":0},"map":{"version":3,"sources":["a"],"mappings":""}},{"offset":{"line":1,"column":0},"url":"b.map"}],"sections":[{"offset":{"line":0,"column":0},"url":"c.map"}]}"#;
    assert_eq!(parse(sections, with(3)).unwrap().sections.len(), 1);
    // So does a section's replaced map: one section, then one source instead of two.
    let map = br#"{"version":3,"sections":[{"offset":{"line":0,"column":0},"map":{"version":3,"sources":["a","b"],"mappings":""},"map":{"version":3,"sources":["c"],"mappings":""}}]}"#;
    assert_eq!(parse(map, with(3)).unwrap().sections[0].2.sources, vec![Some("c".to_string())]);
}

#[test]
fn sections_and_their_maps_share_the_budget() {
    let section = |line: u32, name: &str| json!({"offset":{"line":line,"column":0},"map":{"version":3,"sources":[name],"mappings":"AAAA"}});
    let value = json!({"version":3,"sections":[section(0, "a.js"), section(1, "b.js")]});
    // Two sections and one source in each.
    assert_eq!(decode(value.clone(), 4).unwrap().sections.len(), 2);
    assert_eq!(decode(value, 3), Err(SourceMapError::TooManyEntries(3)));
    // Sections without a map count too, so a long list of them is refused early.
    let external = json!({"offset":{"line":0,"column":0},"url":"a.map"});
    let value = json!({"version":3,"sections":[external.clone(), external.clone(), external]});
    assert_eq!(decode(value.clone(), 3).unwrap().sections.len(), 3);
    assert_eq!(decode(value, 2), Err(SourceMapError::TooManyEntries(2)));
    let not_sections = br#"{"version":3,"sections":[0,0,0]}"#;
    assert_eq!(parse(not_sections, with(2)), Err(SourceMapError::TooManyEntries(2)));
    assert_eq!(parse(not_sections, with(3)), Err(SourceMapError::BadSections));
}

#[test]
fn reading_stops_at_the_limit() {
    // Everything after the entry that goes over the limit is never read: the broken tail would
    // otherwise be reported as bad JSON.
    let json = br#"{"version":3,"sources":["a","b","c","d" this is not JSON"#;
    assert_eq!(parse(json, with(2)), Err(SourceMapError::TooManyEntries(2)));
    assert!(matches!(parse(json, Limits::default()), Err(SourceMapError::BadJson(_))));
}

#[test]
fn unknown_fields_are_skipped_without_counting() {
    let value = json!({"version":3,"x_vendor":[1,2,3,4,5,6,7,8],"sources":["a.js"],"mappings":"AAAA"});
    assert_eq!(decode(value, 1).unwrap().sources.len(), 1);
}

#[test]
fn values_the_map_does_not_keep_are_still_checked() {
    let deep = format!("{{\"version\":3,\"mappings\":\"\",\"x\":{}{}}}", "[".repeat(200), "]".repeat(200));
    for json in [
        br#"{"version":3,"mappings":"","x":"\ud800"}"#.as_slice(),
        br#"{"version":3,"mappings":"","sources":{"a":"\udc00"}}"#,
        br#"{"version":3,"mappings":"","x":1e400}"#,
        br#"{"version":3,"mappings":"","sources":[{"a":1e400}]}"#,
        b"{\"version\":3,\"mappings\":\"\",\"x\":\"\xff\"}",
        br#"{"version":3,"mappings":"","x":[1,]}"#,
        deep.as_bytes(),
    ] {
        let got = parse(json, Limits::default());
        let full = serde_json::from_slice::<serde_json::Value>(json).unwrap_err().to_string();
        assert_eq!(got, Err(SourceMapError::BadJson(full)), "{}", String::from_utf8_lossy(json));
    }
}

#[test]
fn a_hostile_map_of_empty_names_is_refused_by_default() {
    // About 21 million empty strings fit in the 64 MiB a map may have.
    let limit = Limits::default().max_entries;
    let names = vec!["\"\""; limit + 1].join(",");
    let json = format!("{{\"version\":3,\"mappings\":\"\",\"names\":[{names}]}}").into_bytes();
    assert_eq!(parse(&json, Limits::default()), Err(SourceMapError::TooManyEntries(limit)));
    // The same map is well formed: with room for every name it reads.
    assert_eq!(parse(&json, with(limit + 1)).unwrap().names.len(), limit + 1);
}

#[test]
fn sources_are_read_from_a_map_with_broken_mappings_or_offsets() {
    use craft_sourcemap::parse_sources;
    let bad = br#"{"version":3,"sourceRoot":"src","sources":["a.js"],"sourcesContent":["a();"],"names":["x"],"mappings":"!!"}"#;
    assert_eq!(parse(bad, Limits::default()), Err(SourceMapError::BadMappings(1)));
    let map = parse_sources(bad, Limits::default()).unwrap();
    assert_eq!((map.source_path(0).as_deref(), map.sources_content.len(), map.names.len(), map.lines.len()), (Some("src/a.js"), 1, 1, 0));
    let unordered = br#"{"version":3,"sections":[{"offset":{"line":2,"column":0},"map":{"version":3,"sources":["a.js"],"mappings":"AAAA"}},{"offset":{"line":-1,"column":0},"map":{"version":3,"sources":["b.js"],"mappings":""}},{"offset":{"line":0,"column":0},"map":{"version":3,"sources":["c.js"],"mappings":"!"}}]}"#;
    assert_eq!(parse(unordered, Limits::default()), Err(SourceMapError::BadSections));
    let map = parse_sources(unordered, Limits::default()).unwrap();
    assert_eq!(map.source_inventory(Default::default()).sources, vec!["a.js".to_string(), "b.js".to_string(), "c.js".to_string()]);
    // Past mappings that can't be decoded, the indices aren't known.
    assert_eq!(parse_sources(br#"{"version":3,"sources":["a"],"mappings":"AAAA,!,ACAA"}"#, Limits::default()).unwrap().sources.len(), 1);
    // What a section holds must still be known, and the version and limits still apply.
    for (json, limit, err) in [
        (br#"{"version":3,"sections":[{"offset":{"line":0,"column":0}}]}"#.as_slice(), 9, SourceMapError::BadSections),
        (br#"{"version":3,"sections":{}}"#, 9, SourceMapError::BadSections),
        (br#"{"version":3,"sections":[{"offset":{"line":0,"column":0},"map":{"version":2,"sources":["a"]}}]}"#, 9, SourceMapError::BadVersion),
        (br#"{"version":3,"sources":["a","b"],"mappings":"!"}"#, 1, SourceMapError::TooManyEntries(1)),
        // Mappings that name a source past the end of the list show it is short.
        (br#"{"version":3,"mappings":"AAAA"}"#, 9, SourceMapError::BadMappings(1)),
        (br#"{"version":3,"sources":["a"],"mappings":"AAAA;ACAA,!"}"#, 9, SourceMapError::BadMappings(2)),
        (br#"{"version":3,"sources":["a"],"mappings":"AAAA,ADAA,AEAA"}"#, 9, SourceMapError::BadMappings(3)),
    ] {
        assert_eq!(parse_sources(json, with(limit)), Err(err), "{}", String::from_utf8_lossy(json));
    }
}
