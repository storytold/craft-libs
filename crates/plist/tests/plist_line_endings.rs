use craft_plist::{self as plist, Value};

/// Literal line endings normalize before entities, including keys and CDATA.
#[test]
fn xml_line_endings_follow_xml_rules() {
    let xml = "<plist><dict><key>a\rb</key><string>a\r\nb\rc\nd&#13;e<![CDATA[f\r\ng]]></string></dict></plist>";
    for encoding in 0..3 {
        let bytes = match encoding {
            1 => [vec![0xff, 0xfe], xml.encode_utf16().flat_map(u16::to_le_bytes).collect()].concat(),
            2 => [vec![0xfe, 0xff], xml.encode_utf16().flat_map(u16::to_be_bytes).collect()].concat(),
            _ => xml.as_bytes().to_vec(),
        };
        let value = plist::parse_xml(&bytes).unwrap();
        assert_eq!(value.get("a\nb").and_then(Value::as_str), Some("a\nb\nc\nd\ref\ng"));
    }
    assert!(plist::parse_xml(b"<plist><string>a\r\nb</dict></plist>").is_err());
}
