use craft_plist::{Value, parse_binary};

fn single_object(object: &[u8], offset: u8) -> Vec<u8> {
    let mut data = b"bplist00".to_vec();
    data.extend_from_slice(object);
    let table = data.len() as u64;
    data.push(offset);
    data.extend_from_slice(&[0, 0, 0, 0, 0, 0, 1, 1]);
    data.extend_from_slice(&1u64.to_be_bytes());
    data.extend_from_slice(&0u64.to_be_bytes());
    data.extend_from_slice(&table.to_be_bytes());
    data
}

#[test]
fn binary_values_cannot_consume_the_offset_table_or_trailer() {
    for object in [&[0x54, b'x'][..], &[0x43, 0x42], &[0x13], &[0x87], &[0x62, 0, b'x']] {
        assert!(parse_binary(&single_object(object, 8)).is_err(), "accepted object {object:02x?}");
    }
}

#[test]
fn binary_offsets_cannot_point_inside_the_file_header() {
    assert!(parse_binary(&single_object(&[0x00], 0)).is_err());
    assert!(matches!(parse_binary(&single_object(&[0x00], 8)), Ok(Value::Null)));
}

#[test]
fn objects_ending_at_the_table_boundary_are_valid() {
    assert!(matches!(parse_binary(&single_object(&[0x51, b'x'], 8)), Ok(Value::String(s)) if s == "x"));
}
