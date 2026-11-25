use craft_sourcemap as sourcemap;
use craft_sourcemap::{Limits, SourceMapError};
use serde_json::{Value, json};

fn decode(value: Value, limits: Limits) -> Result<sourcemap::SourceMap, SourceMapError> {
    sourcemap::parse(&serde_json::to_vec(&value).unwrap(), limits)
}

fn map(mappings: &str) -> Value {
    json!({"version":3,"sources":["input.js"],"names":[],"mappings":mappings})
}

#[test]
fn empty_lines_spend_the_default_budget_even_with_no_segments() {
    let limits = Limits { max_segments: 0, ..Limits::default() };
    assert_eq!(decode(map(&";".repeat(1_000_000)), limits), Err(SourceMapError::TooManyLines(1_000_000)));
}

#[test]
fn exact_and_zero_line_limits_include_the_final_empty_line() {
    let limits = Limits { max_lines: 3, max_segments: 0, ..Limits::default() };
    let parsed = decode(map(";;"), limits).unwrap();
    assert_eq!(parsed.lines.len(), 3);
    assert_eq!(parsed.segments, 0);
    assert_eq!(decode(map(";;;"), limits), Err(SourceMapError::TooManyLines(3)));
    assert_eq!(decode(map(""), Limits { max_lines: 0, ..limits }), Err(SourceMapError::TooManyLines(0)));
    assert_eq!(decode(map(""), Limits { max_lines: 1, ..limits }).unwrap().lines.len(), 1);
}

#[test]
fn nested_sections_share_one_generated_line_budget() {
    let value = json!({"version":3,"sections":[
        {"offset":{"line":0,"column":0},"map":map(";")},
        {"offset":{"line":2,"column":0},"map":{"version":3,"sections":[
            {"offset":{"line":0,"column":0},"map":map(";")}
        ]}}
    ]});
    let limits = Limits { max_lines: 3, max_segments: 0, ..Limits::default() };
    assert_eq!(decode(value.clone(), limits), Err(SourceMapError::TooManyLines(3)));
    let parsed = decode(value, Limits { max_lines: 4, ..limits }).unwrap();
    assert_eq!(parsed.sections[0].2.lines.len(), 2);
    assert_eq!(parsed.sections[1].2.sections[0].2.lines.len(), 2);
}

#[test]
fn external_sections_do_not_consume_undecoded_lines() {
    let value = json!({"version":3,"sections":[
        {"offset":{"line":0,"column":0},"url":"other.map"},
        {"offset":{"line":2,"column":0},"map":map("")}
    ]});
    let parsed = decode(value, Limits { max_lines: 1, max_segments: 0, ..Limits::default() }).unwrap();
    assert!(parsed.sections[0].2.lines.is_empty());
    assert_eq!(parsed.sections[1].2.lines.len(), 1);
}

#[test]
fn positions_after_empty_lines_keep_their_coordinates() {
    let parsed = decode(map("AAAA;;AACA"), Limits { max_lines: 3, max_segments: 2, ..Limits::default() }).unwrap();
    assert_eq!(parsed.lines.len(), 3);
    assert!(parsed.original_positions(1, 0).is_empty());
    let position = &parsed.original_positions(2, 0)[0];
    assert_eq!(position.source.as_deref(), Some("input.js"));
    assert_eq!((position.line, position.column), (1, 0));
}

#[test]
fn line_and_segment_limits_are_independent() {
    let limits = Limits { max_lines: 10, max_segments: 1, ..Limits::default() };
    assert!(matches!(decode(map("A,A"), limits), Err(SourceMapError::TooManySegments(_))));
    assert_eq!(decode(map(";"), Limits { max_lines: 1, max_segments: 0, ..limits }), Err(SourceMapError::TooManyLines(1)));
    assert_eq!(decode(map(";"), Limits { max_lines: 2, max_segments: 0, ..limits }).unwrap().segments, 0);
}
