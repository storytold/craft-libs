use craft_plist::{Value, parse_binary};

fn single_object(object: &[u8]) -> Vec<u8> {
    let mut data = b"bplist00".to_vec();
    data.extend_from_slice(object);
    let table = data.len() as u64;
    data.push(8);
    data.extend_from_slice(&[0, 0, 0, 0, 0, 0, 1, 1]);
    data.extend_from_slice(&1u64.to_be_bytes());
    data.extend_from_slice(&0u64.to_be_bytes());
    data.extend_from_slice(&table.to_be_bytes());
    data
}

#[test]
fn unsupported_integer_widths_are_not_clamped() {
    for marker in 0x15..=0x1f {
        let mut object = vec![marker];
        object.extend_from_slice(&[0; 16]);
        assert!(parse_binary(&single_object(&object)).is_err(), "accepted {marker:#x}");
    }
}

#[test]
fn extended_lengths_and_uids_do_not_lose_high_bytes() {
    let mut length = vec![0x4f, 0x14];
    length.extend_from_slice(&[0; 16]);
    assert!(parse_binary(&single_object(&length)).is_err());
    for marker in 0x88..=0x8f {
        let mut object = vec![marker, 1];
        object.resize(usize::from(marker & 15) + 2, 0);
        assert!(parse_binary(&single_object(&object)).is_err(), "accepted {marker:#x}");
    }
}

#[test]
fn supported_maximum_widths_keep_their_values() {
    let mut integer = vec![0x14];
    integer.extend_from_slice(&i128::MAX.to_be_bytes());
    assert!(matches!(parse_binary(&single_object(&integer)), Ok(Value::Integer(i)) if i == i128::MAX));
    let mut uid = vec![0x87];
    uid.extend_from_slice(&u64::MAX.to_be_bytes());
    assert!(matches!(parse_binary(&single_object(&uid)), Ok(Value::Uid(i)) if i == u64::MAX));
    let mut empty = vec![0x4f, 0x13];
    empty.extend_from_slice(&0u64.to_be_bytes());
    assert!(matches!(parse_binary(&single_object(&empty)), Ok(Value::Data(d)) if d.is_empty()));
}

#[test]
fn minor_version_compatibility_is_explicit_and_keeps_bounds() {
    use craft_plist::{BinaryOptions, FormatError, Limits, parse_binary_with_options};
    let compat = BinaryOptions { allow_unknown_minor_version: true };
    for minor in [b'0', b'1', b'9', 0xff] {
        let mut data = single_object(&[0x00]);
        data[7] = minor;
        assert!(matches!(parse_binary_with_options(&data, Limits::default(), compat), Ok(Value::Null)));
        assert_eq!(parse_binary(&data).is_ok(), minor == b'0');
        let limited = Limits { max_nodes: 0, ..Limits::default() };
        assert!(matches!(parse_binary_with_options(&data, limited, compat), Err(FormatError::TooLarge(_))));
        data[6] = b'1';
        assert!(matches!(parse_binary_with_options(&data, Limits::default(), compat), Err(FormatError::Unsupported(_))));
    }
    let mut invalid = single_object(&[0x15]);
    invalid[7] = b'1';
    assert!(matches!(parse_binary_with_options(&invalid, Limits::default(), compat), Err(FormatError::Malformed(_))));
    let mut cycle = single_object(&[0xa1, 0]);
    cycle[7] = b'1';
    assert!(matches!(parse_binary_with_options(&cycle, Limits::default(), compat), Err(FormatError::Malformed(_))));
    assert!(parse_binary_with_options(b"bplist01", Limits::default(), compat).is_err());
}
