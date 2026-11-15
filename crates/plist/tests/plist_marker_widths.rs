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
