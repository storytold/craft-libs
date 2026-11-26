use craft_sourcemap::{Limits, parse};
use serde_json::json;

#[test]
fn indexed_lookup_reports_columns_in_the_generated_file() {
    let json = json!({"version":3,"sections":[{"offset":{"line":3,"column":11},"map":{"version":3,"sources":["one.js"],"mappings":"IAAA;IACA"}}]});
    let map = parse(json.to_string().as_bytes(), Limits::default()).unwrap();
    assert!(map.original_positions(3, 14).is_empty());
    let first = map.original_positions(3, 17);
    assert_eq!(first[0].generated_column, 15);
    assert_eq!((first[0].line, first[0].column), (0, 0));
    assert_eq!(map.original_positions(4, 4)[0].generated_column, 4);
}

#[test]
fn nested_section_columns_are_added_only_on_each_first_line() {
    for (inner_line, query_line, query_column) in [(0, 3, 18), (1, 4, 7)] {
        let json = json!({"version":3,"sections":[{"offset":{"line":3,"column":11},"map":{"version":3,"sections":[{"offset":{"line":inner_line,"column":3},"map":{"version":3,"sources":["nested.js"],"mappings":"IAAA"}}]}}]});
        let map = parse(json.to_string().as_bytes(), Limits::default()).unwrap();
        let positions = map.original_positions(query_line, query_column);
        assert_eq!(positions[0].generated_column, query_column);
        assert_eq!(positions[0].source.as_deref(), Some("nested.js"));
    }
}
