use craft_pcraft_format::{Error, Limits, MAX_DEPTH, MAX_GROUP_DEPTH, MAX_MANIFEST_ITEMS, check_manifest, manifest_depth, parse_manifest};

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
fn defaults_are_photocraft_s_load_limits() {
    let limits = Limits::default();
    assert_eq!(limits.max_manifest_bytes, 256 << 20);
    assert_eq!(limits.max_blob_bytes, 1 << 30);
    assert_eq!(limits.max_total_bytes, 16 << 30);
    assert_eq!(MAX_GROUP_DEPTH, 100);
    assert_eq!(limits.max_depth, 367);
    assert_eq!(limits.max_depth, manifest_depth(MAX_GROUP_DEPTH));
    assert_eq!(limits.max_items, MAX_MANIFEST_ITEMS);
    assert_eq!(MAX_MANIFEST_ITEMS, 4 << 20);
}

#[test]
fn manifest_depth_saturates() {
    assert_eq!(manifest_depth(0), 67);
    assert_eq!(manifest_depth(usize::MAX), usize::MAX);
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
fn a_raised_max_depth_stops_at_max_depth_cap() {
    let limits = small(100_000, MAX_MANIFEST_ITEMS);
    assert!(parse_manifest(&nested(MAX_DEPTH), &limits).unwrap().is_array());
    let deeper = parse_manifest(&nested(MAX_DEPTH + 1), &limits).err().map(|e| e.to_string());
    assert_eq!(deeper, Some(format!("limit exceeded: manifest JSON nested deeper than {MAX_DEPTH} levels")));
    assert!(matches!(parse_manifest(&nested(100_000), &limits), Err(Error::LimitExceeded(_))));
}

#[test]
fn items_up_to_max_items_pass_and_one_more_does_not() {
    let limits = small(8, 4);
    assert!(check_manifest(b"[1,2,3,4]", &limits).is_ok());
    assert!(matches!(check_manifest(b"[1,2,3,4,5]", &limits), Err(Error::LimitExceeded(_))));
    assert!(matches!(check_manifest(b"[[[[[]]]]]", &limits), Err(Error::LimitExceeded(_))));
}

#[test]
fn brackets_and_commas_inside_strings_are_not_counted() {
    let limits = small(2, 3);
    assert!(check_manifest(br#"{"a":"[[[{{{,,,,"}"#, &limits).is_ok());
    assert!(check_manifest(br#"{"a":"\"[[[,,,\\"}"#, &limits).is_ok());
    assert!(matches!(check_manifest(br#"{"a":"\\",[[[]]]}"#, &limits), Err(Error::LimitExceeded(_))));
}

#[test]
fn a_manifest_larger_than_max_manifest_bytes_is_refused() {
    let mut limits = Limits::default();
    limits.max_manifest_bytes = 8;
    assert!(check_manifest(b"{\"a\":12}", &limits).is_ok());
    assert!(matches!(check_manifest(b"{\"a\":123}", &limits), Err(Error::LimitExceeded(_))));
    assert!(matches!(parse_manifest(b"{\"a\":123}", &limits), Err(Error::LimitExceeded(_))));
}

#[test]
fn invalid_or_trailing_json_is_a_json_error() {
    let limits = Limits::default();
    assert!(matches!(parse_manifest(b"{\"a\":", &limits), Err(Error::Json(_))));
    assert!(matches!(parse_manifest(b"{} {}", &limits), Err(Error::Json(_))));
    assert!(matches!(parse_manifest(b"", &limits), Err(Error::Json(_))));
}

#[test]
fn messages_read_as_photocraft_s() {
    let too_new = Error::TooNew { found: 2, supported: 1 };
    assert_eq!(too_new.to_string(), "bundle written by a newer version (format 2, this build reads up to 1)");
    assert_eq!(Error::Corrupt("x".into()).to_string(), "corrupt .pcraft bundle: x");
    assert_eq!(Error::Unsupported("x".into()).to_string(), "unsupported .pcraft feature: x");
    assert_eq!(Error::LimitExceeded("x".into()).to_string(), "limit exceeded: x");
    let deep = check_manifest(&nested(368), &Limits::default()).err().map(|e| e.to_string());
    assert_eq!(deep.as_deref(), Some("limit exceeded: manifest JSON nested deeper than 367 levels"));
}
