use craft_plist::{self as plist, Value};

/// Quoted delimiters belong to attributes, including on empty elements.
#[test]
fn quoted_tag_delimiters_preserve_values() {
    for quote in ['\'', '"'] {
        let xml = format!(
            "<plist note={quote}a>b{quote}><dict><key note={quote}/>界{quote}>key</key><string note={quote}>/{quote}>value</string><key>empty</key><string note={quote}>/{quote}/></dict></plist>"
        );
        let value = plist::parse_xml(xml.as_bytes()).unwrap();
        assert_eq!(value.get("key").and_then(Value::as_str), Some("value"));
        assert_eq!(value.get("empty").and_then(Value::as_str), Some(""));
    }
}

/// A quote that never closes must not turn later markup into a value.
#[test]
fn unterminated_attribute_quotes_are_rejected() {
    for xml in ["<plist><string note=\"oops>value</string></plist>", "<plist><string note='oops>value</string></plist>"] {
        assert!(plist::parse_xml(xml.as_bytes()).is_err(), "{xml}");
    }
}
