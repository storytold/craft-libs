//! Integration tests of craft-pcraft-format: manifest size and depth limits.
use craft_pcraft_format::{Error, Limits, check_manifest, check_manifest_with_cancel, parse_manifest};

#[test]
fn cancellation_stops_manifest_validation_after_it_starts() {
    let polls = std::cell::Cell::new(0);
    let cancelled = || {
        polls.set(polls.get() + 1);
        polls.get() >= 3
    };
    let json = format!(r#"{{"padding":"{}"}}"#, "x".repeat(256 * 1024));
    assert!(matches!(check_manifest_with_cancel(json.as_bytes(), &Limits::default(), &cancelled), Err(Error::Cancelled)));
    assert_eq!(polls.get(), 3);
    for limits in [Limits::default(), small(1, 1)] {
        assert_eq!(
            check_manifest_with_cancel(json.as_bytes(), &limits, &|| false).map_err(|e| e.to_string()),
            check_manifest(json.as_bytes(), &limits).map_err(|e| e.to_string())
        );
    }
}

fn nested(depth: usize) -> Vec<u8> {
    let mut json = "[".repeat(depth).into_bytes();
    json.extend(std::iter::repeat_n(b']', depth));
    json
}

fn small(max_depth: usize, max_items: usize) -> Limits {
    let mut limits = Limits::default();
    limits.max_depth = max_depth;
    limits.max_items = max_items;
    limits
}

#[test]
fn nesting_up_to_max_depth_passes_and_one_more_level_does_not() {
    let limits = Limits::default();
    let at = limits.max_depth;
    assert!(check_manifest(&nested(at), &limits).is_ok());
    assert!(matches!(check_manifest(&nested(at + 1), &limits), Err(Error::LimitExceeded(_))));
}

#[test]
fn a_manifest_deeper_than_serde_json_s_128_levels_parses() {
    let value = parse_manifest(&nested(300), &Limits::default()).unwrap();
    assert!(value.is_array());
}

#[test]
fn brackets_and_commas_inside_strings_are_not_counted() {
    let limits = small(2, 3);
    assert!(check_manifest(br#"{"a":"[[[{{{,,,,"}"#, &limits).is_ok());
    assert!(check_manifest(br#"{"a":"\"[[[,,,\\"}"#, &limits).is_ok());
    assert!(matches!(check_manifest(br#"{"a":"\\",[[[]]]}"#, &limits), Err(Error::LimitExceeded(_))));
}
