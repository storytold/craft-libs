use craft_sqlite::{Error, read_varint, serial_type_len};

#[test]
fn varints_decode_at_each_width() {
    let cases: &[(&[u8], u64)] = &[
        (&[0x7f], 127),
        (&[0x81, 0], 1 << 7),
        (&[0x81, 0x80, 0], 1 << 14),
        (&[0x81, 0x80, 0x80, 0], 1 << 21),
        (&[0x81, 0x80, 0x80, 0x80, 0], 1 << 28),
        (&[0x81, 0x80, 0x80, 0x80, 0x80, 0], 1 << 35),
        (&[0x81, 0x80, 0x80, 0x80, 0x80, 0x80, 0], 1 << 42),
        (&[0x81, 0x80, 0x80, 0x80, 0x80, 0x80, 0x80, 0], 1 << 49),
        (&[0x80, 0xc0, 0x80, 0x80, 0x80, 0x80, 0x80, 0x80, 0], 1 << 56),
    ];
    for &(bytes, expected) in cases {
        assert_eq!(read_varint(bytes), Ok((expected, bytes.len())), "{bytes:x?}");
    }
}

#[test]
fn ninth_byte_keeps_all_bits_and_leaves_trailing_bytes() {
    let bytes = [0xc0, 0x80, 0x80, 0x80, 0x80, 0x80, 0x80, 0x80, 0x80, 0x42];
    assert_eq!(read_varint(&bytes), Ok((0x8000_0000_0000_0080, 9)));
    assert_eq!(read_varint(&[0xff; 9]), Ok((u64::MAX, 9)));
}

#[test]
fn leading_varint_preserves_following_bytes_and_accepts_padding() {
    assert_eq!(read_varint(&[0, 0xff]), Ok((0, 1)));
    assert_eq!(read_varint(&[0x81, 0, 0xff]), Ok((128, 2)));
    assert_eq!(read_varint(&[0x80, 1, 0xff]), Ok((1, 2)));
}

#[test]
fn varint_refuses_every_incomplete_prefix() {
    let bytes = [0xff; 9];
    for len in 0..bytes.len() {
        assert_eq!(read_varint(&bytes[..len]), Err(Error::TruncatedVarint), "length {len}");
    }
}

#[test]
fn fixed_serial_types_have_exact_widths() {
    for (serial_type, width) in (0..).zip([0, 1, 2, 3, 4, 6, 8, 8, 0, 0]) {
        assert_eq!(serial_type_len(serial_type), Ok(width), "code {serial_type}");
    }
}

#[test]
fn reserved_serial_types_are_refused() {
    for serial_type in [10, 11] {
        assert_eq!(serial_type_len(serial_type), Err(Error::ReservedSerialType(serial_type)));
    }
}

#[test]
fn variable_serial_type_widths_are_portable() {
    for (serial_type, width) in [
        (12, 0),
        (13, 0),
        (14, 1),
        (15, 1),
        (12_012, 6_000),
        (12_013, 6_000),
        (u64::MAX - 1, 9_223_372_036_854_775_801),
        (u64::MAX, 9_223_372_036_854_775_801),
    ] {
        assert_eq!(serial_type_len(serial_type), Ok(width), "code {serial_type}");
    }
}
