use super::*;
#[test]
fn mapping_indices_must_refer_to_declared_entries() {
    for mappings in ["ACAA", "AAAAC"] {
        let json = serde_json::json!({"version":3,"sources":["source.js"],"names":["name"],"mappings":mappings});
        assert!(matches!(parse(json.to_string().as_bytes(), Limits::default()), Err(SourceMapError::BadMappings(1))));
    }
    let valid = br#"{"version":3,"sources":[null],"names":["name"],"mappings":"AAAAA"}"#;
    assert!(parse(valid, Limits::default()).is_ok());
}

#[test]
fn lookup_keeps_coincident_mappings_and_stops_at_an_unmapped_segment() {
    let map = parse(br#"{"version":3,"sources":["a","b"],"mappings":"EAAA,ACAA,E"}"#, Limits::default()).unwrap();
    assert!(map.original_positions(0, 1).is_empty());
    let matches = map.original_positions(0, 3);
    assert_eq!(matches.iter().map(|p| p.source_index).collect::<Vec<_>>(), [0, 1]);
    assert!(map.original_positions(0, 4).is_empty());
}

#[test]
fn relative_sources_keep_the_url_root_delimiter() {
    for (root, expected) in [
        ("webpack:///", "webpack:///src/main.js"),
        ("file:///", "file:///src/main.js"),
        ("https://host/scripts/", "https://host/scripts/src/main.js"),
        ("src", "src/src/main.js"),
    ] {
        let json = serde_json::json!({"version":3,"sourceRoot":root,"sources":["src/main.js"],"mappings":"AAAA"});
        let map = parse(json.to_string().as_bytes(), Limits::default()).unwrap();
        assert_eq!(map.all_sources(), [expected]);
        assert_eq!(map.original_positions(0, 0)[0].source.as_deref(), Some(expected));
    }
}

#[test]
fn decodes_and_looks_up() {
    // Generated "a;b" from two lines of src.js.
    let m = br#"{"version":3,"sources":["src.js"],"names":["foo"],"mappings":"AAAAA,EAAE;AACA","sourcesContent":["x\ny"]}"#;
    let map = parse(m, Limits::default()).unwrap();
    assert_eq!(map.lines.len(), 2);
    let p = map.original_positions(0, 1);
    assert_eq!(p[0].source.as_deref(), Some("src.js"));
    assert_eq!(p[0].name.as_deref(), Some("foo"));
    assert_eq!((p[0].line, p[0].column), (0, 0));
    let q = map.original_positions(0, 2);
    assert_eq!(q[0].column, 2);
    let r = map.original_positions(1, 0);
    assert_eq!(r[0].line, 1);
    assert!(map.original_positions(5, 0).is_empty());
    assert_eq!(map.source_content(0), Some("x\ny"));
}

#[test]
fn indexed_maps() {
    let m = br#"{"version":3,"sections":[{"offset":{"line":0,"column":0},"map":{"version":3,"sources":["a.js"],"names":[],"mappings":"AAAA"}},{"offset":{"line":1,"column":0},"map":{"version":3,"sources":["b.js"],"names":[],"mappings":"AAAA"}}]}"#;
    let map = parse(m, Limits::default()).unwrap();
    assert_eq!(map.original_positions(1, 3)[0].source.as_deref(), Some("b.js"));
    assert_eq!(map.all_sources(), vec!["a.js", "b.js"]);
}

#[test]
fn validates_index_map_shape_and_offsets() {
    let inline = serde_json::json!({"version":3,"mappings":"A"});
    for sections in [
        serde_json::json!({}),
        serde_json::json!([{"map":inline}]),
        serde_json::json!([{"offset":{"line":-1,"column":0},"map":inline}]),
        serde_json::json!([{"offset":{"line":0,"column":4294967296_u64},"map":inline}]),
        serde_json::json!([{"offset":{"line":0,"column":0}}]),
        serde_json::json!([{"offset":{"line":0,"column":0},"map":inline,"url":"later.map"}]),
        serde_json::json!([{"offset":{"line":0,"column":2},"map":inline},{"offset":{"line":0,"column":1},"map":inline}]),
    ] {
        let json = serde_json::json!({"version":3,"sections":sections}).to_string();
        assert_eq!(parse(json.as_bytes(), Limits::default()), Err(SourceMapError::BadSections));
    }
}

#[test]
fn an_external_section_ends_the_previous_inline_mapping() {
    let inline = serde_json::json!({"version":3,"sources":["first.js"],"mappings":"AAAA"});
    let sections = serde_json::json!([
        {"offset":{"line":0,"column":0},"map":inline},
        {"offset":{"line":0,"column":4},"url":"external.map"},
        {"offset":{"line":1,"column":0},"map":inline}
    ]);
    let json = serde_json::json!({"version":3,"sections":sections}).to_string();
    let map = parse(json.as_bytes(), Limits::default()).unwrap();
    assert_eq!(map.original_positions(0, 3)[0].source.as_deref(), Some("first.js"));
    assert!(map.original_positions(0, 4).is_empty());
    assert_eq!(map.original_positions(1, 0)[0].generated_column, 0);
}

#[test]
fn vlq_signed_extremes_and_overflow() {
    for (field, value) in [
        ("B", Some(i64::from(i32::MIN))),
        ("+/////D", Some(i64::from(i32::MAX))),
        ("//////D", Some(-i64::from(i32::MAX))),
        ("ggggggE", None),
        ("ggggggggggA", Some(0)),
    ] {
        assert_eq!(vlq(field.as_bytes(), &mut 0), value, "{field}");
    }
    let map = parse(br#"{"version":3,"mappings":"+/////D,C,B"}"#, Limits::default()).unwrap();
    assert_eq!(map.lines[0].iter().map(|s| s.generated_column).collect::<Vec<_>>(), [0, 2147483647, 2147483648]);
    assert!(parse(br#"{"version":3,"mappings":"B"}"#, Limits::default()).is_err());
}

#[test]
fn hostile_mappings_do_not_panic() {
    for m in ["", ";;;", "A", "gggggggggggggggggggggggggg", "AAAA,", "////", "AAAAAAAAAAAAAAA", "D", "AD,AA", "+", "!!"] {
        let j = format!(r#"{{"version":3,"sources":[],"names":[],"mappings":"{m}"}}"#);
        if let Ok(map) = parse(j.as_bytes(), Limits::default()) {
            let _ = map.original_positions(0, 0);
            let _ = map.original_positions(u32::MAX, u32::MAX);
        }
    }
    let tight = Limits { max_segments: 1, ..Limits::default() };
    assert!(parse(br#"{"version":3,"sources":[],"names":[],"mappings":"A,A"}"#, tight).is_err());
    assert!(parse(b"[]", Limits::default()).is_err());
}
