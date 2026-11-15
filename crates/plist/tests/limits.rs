use craft_plist::{Limits, Value, parse_binary, parse_binary_with_limits};
fn plist(objects: &[&[u8]]) -> Vec<u8> {
    let mut b = b"bplist00".to_vec();
    let mut offsets = Vec::new();
    for obj in objects {
        offsets.push(b.len() as u8);
        b.extend_from_slice(obj);
    }
    let table = b.len() as u64;
    b.extend_from_slice(&offsets);
    b.extend_from_slice(&[0, 0, 0, 0, 0, 0, 1, 1]);
    b.extend_from_slice(&(objects.len() as u64).to_be_bytes());
    b.extend_from_slice(&0u64.to_be_bytes());
    b.extend_from_slice(&table.to_be_bytes());
    b
}
#[test]
fn languages_and_every_truncation() {
    let b = plist(&[&[0xd1, 1, 2], b"\x5eAppleLanguages", &[0xa1, 3], b"\x55ja-JP"]);
    let p = parse_binary(&b).unwrap();
    assert_eq!(p.get("AppleLanguages").unwrap().as_array().unwrap()[0].as_str(), Some("ja-JP"));
    for n in 0..b.len() {
        assert!(parse_binary(&b[..n]).is_err(), "prefix {n}");
    }
}
#[test]
fn cycles_and_repeated_references() {
    assert!(parse_binary(&plist(&[&[0xa1, 0]])).is_err());
    let b = plist(&[&[0xa3, 1, 1, 1], b"\x51x"]);
    assert_eq!(parse_binary(&b).unwrap(), Value::Array(vec![Value::String("x".into()); 3]));
    assert!(parse_binary_with_limits(&b, Limits { max_nodes: 3, ..Limits::default() }).is_err());
    assert!(parse_binary_with_limits(&b, Limits { max_expanded_bytes: 2, ..Limits::default() }).is_err());
}
#[test]
fn explicit_limits_and_version() {
    let b = plist(&[&[0xa1, 1], b"\x51x"]);
    for l in [
        Limits { max_input_bytes: b.len() - 1, ..Limits::default() },
        Limits { max_objects: 1, ..Limits::default() },
        Limits { max_depth: 1, ..Limits::default() },
        Limits { max_nodes: 0, ..Limits::default() },
    ] {
        assert!(parse_binary_with_limits(&b, l).is_err());
    }
    let mut future = b;
    future[7] = b'1';
    assert!(parse_binary(&future).is_err());
}
#[test]
fn utf16_and_lossy_expansion_spend_budget() {
    let b = plist(&[&[0x62, 0xd8, 0x3d, 0xde, 0x00]]);
    assert_eq!(parse_binary(&b).unwrap().as_str(), Some("😀"));
    assert!(parse_binary_with_limits(&b, Limits { max_expanded_bytes: 3, ..Limits::default() }).is_err());
    let bad = plist(&[&[0x51, 0xff]]);
    assert_eq!(parse_binary(&bad).unwrap().as_str(), Some("�"));
    assert!(parse_binary_with_limits(&bad, Limits { max_expanded_bytes: 2, ..Limits::default() }).is_err());
}
