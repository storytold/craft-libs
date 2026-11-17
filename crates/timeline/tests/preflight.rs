use craft_timeline::preflight::{json_depth, xml_depth};

#[test]
fn xml_skips_markup_inside_quotes_comments_and_cdata() {
    let input = br#"<?xml version="1.0"?><a x="<fake><fake>" y='>'><!-- <fake> --><![CDATA[<fake>]]><b><leaf/></b></a>"#;
    assert_eq!(xml_depth(input), 2);
    assert_eq!(xml_depth(b"<self-closing/>"), 0);
}

#[test]
fn lexical_scans_do_not_claim_to_validate_malformed_documents() {
    assert_eq!(xml_depth(b"</a></b><a>"), 1);
    assert_eq!(xml_depth(b"<a><!-- unterminated <b>"), 1);
    assert_eq!(json_depth(b"]]}{"), 1);
    assert_eq!(json_depth(br#"{"unterminated[[["#), 1);
}

#[test]
fn json_ignores_brackets_and_escaped_quotes_in_strings() {
    assert_eq!(json_depth(br#"{"x":[{"s":"[\"{]}"}]}"#), 3);
    assert_eq!(json_depth(br#""[[[""#), 0);
}

#[test]
fn deep_inputs_are_scanned_iteratively_without_a_tree() {
    let depth = 10_000;
    let xml = "<x>".repeat(depth) + &"</x>".repeat(depth);
    assert_eq!(xml_depth(xml.as_bytes()), depth);
    let json = "[".repeat(depth) + &"]".repeat(depth);
    assert_eq!(json_depth(json.as_bytes()), depth);
}
