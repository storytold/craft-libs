use craft_sqlite::{
    BtreePageHeader, Error, PayloadKind, btree_cell_offset, decode_signed_be_int, local_payload_len, page_size_from_raw, parse_btree_page_header,
    read_varint, serial_type_len, split_record, usable_page_size,
};

#[test]
fn record_split_excludes_the_length_encoding() {
    assert_eq!(split_record(&[3, 1, 15, 42, b'A']), Ok((&[1, 15][..], &[42, b'A'][..])));
    assert_eq!(split_record(&[1]), Ok((&[][..], &[][..])));
    assert_eq!(split_record(&[3, 0, 9]), Ok((&[0, 9][..], &[][..])));

    let mut payload = [0; 129];
    payload[..3].copy_from_slice(&[0x81, 0, 1]);
    payload[128] = 42;
    let (serial_types, body) = split_record(&payload).unwrap();
    assert_eq!(serial_types.len(), 126);
    assert_eq!(serial_types[0], 1);
    assert_eq!(body, [42]);
}

#[test]
fn record_split_limits_a_serial_type_to_its_header() {
    let (serial_types, body) = split_record(&[2, 0x81, 0]).unwrap();
    assert_eq!(body, [0]);
    assert_eq!(read_varint(serial_types), Err(Error::TruncatedVarint));

    let (serial_types, _) = split_record(&[3, 0x81, 0]).unwrap();
    assert_eq!(read_varint(serial_types), Ok((128, 2)));
}

#[test]
fn record_split_rejects_incomplete_and_out_of_range_lengths() {
    for payload in [&[][..], &[0x80][..]] {
        assert_eq!(split_record(payload), Err(Error::TruncatedVarint));
    }
    for (payload, length) in [(&[0][..], 0), (&[0x80, 1][..], 1), (&[2][..], 2), (&[0xff; 9][..], u64::MAX)] {
        assert_eq!(split_record(payload), Err(Error::InvalidRecordHeaderLength(length)));
    }
}

#[test]
fn local_payload_preserves_inline_and_overflow_boundaries() {
    // Original literal cases calculated from SQLite file format section 1.6, including
    // the 512-byte page with 32 reserved bytes and an odd usable size with one reserved byte.
    for (usable, minimum, table_max, index_max) in [
        (480, 35, 445, 94),
        (511, 39, 476, 102),
        (512, 39, 477, 102),
        (1_024, 103, 989, 230),
        (2_048, 232, 2_013, 487),
        (4_096, 489, 4_061, 1_002),
        (8_192, 1_003, 8_157, 2_030),
        (16_384, 2_031, 16_349, 4_086),
        (32_768, 4_087, 32_733, 8_198),
        (65_536, 8_199, 65_501, 16_422),
    ] {
        for (kind, maximum) in [(PayloadKind::TableLeaf, table_max), (PayloadKind::Index, index_max)] {
            for (payload, expected) in [(0, 0), (maximum, maximum), (maximum + 1, minimum)] {
                assert_eq!(local_payload_len(usable, u64::from(payload), kind), Some(expected), "usable {usable}, payload {payload}, {kind:?}");
            }
            // A complete overflow-page span returns to the upper local boundary; one
            // further byte must fall back to the minimum rather than exceed the maximum.
            let payload = u64::from(maximum) + u64::from(usable) - 4;
            assert_eq!(local_payload_len(usable, payload, kind), Some(maximum), "usable {usable}, {kind:?}");
            assert_eq!(local_payload_len(usable, payload + 1, kind), Some(minimum), "usable {usable}, {kind:?}");
        }
    }
    assert_eq!(local_payload_len(512, 600, PayloadKind::TableLeaf), Some(92));
}

#[test]
fn local_payload_refuses_unsupported_usable_sizes() {
    for usable in [0, 4, 35, 479, 65_537, u32::MAX] {
        for kind in [PayloadKind::TableLeaf, PayloadKind::Index] {
            assert_eq!(local_payload_len(usable, 0, kind), None);
            assert_eq!(local_payload_len(usable, u64::MAX, kind), None);
        }
    }
}

#[test]
fn local_payload_keeps_full_width_payload_lengths() {
    // Literal remainders for the largest wire length catch host-size truncation and
    // overflow in the minimum-plus-modulo calculation without allocating a payload.
    for (usable, table_local, index_local) in [(480, 323, 35), (511, 93, 93), (512, 255, 39), (32_768, 4_095, 4_095), (65_536, 8_199, 8_199)] {
        assert_eq!(local_payload_len(usable, u64::MAX, PayloadKind::TableLeaf), Some(table_local));
        assert_eq!(local_payload_len(usable, u64::MAX, PayloadKind::Index), Some(index_local));
    }
}

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

#[test]
fn page_size_and_usable_validation() {
    assert_eq!(page_size_from_raw(512), Ok(512));
    assert_eq!(page_size_from_raw(4096), Ok(4096));
    assert_eq!(page_size_from_raw(1), Ok(65_536));
    assert_eq!(page_size_from_raw(0), Err(Error::InvalidPageSize(0)));
    assert_eq!(page_size_from_raw(256), Err(Error::InvalidPageSize(256)));
    assert_eq!(page_size_from_raw(1000), Err(Error::InvalidPageSize(1000)));

    assert_eq!(usable_page_size(512, 32), Ok(480));
    assert_eq!(usable_page_size(512, 33), Err(Error::UsablePageSizeTooSmall(479)));
    assert_eq!(usable_page_size(100, 150), Err(Error::UsablePageSizeTooSmall(0)));
    assert_eq!(usable_page_size(512, 0), Ok(512));

    for err in [
        Error::InvalidPageSize(0),
        Error::UsablePageSizeTooSmall(479),
        Error::TruncatedPage,
        Error::UnsupportedBtreePageType(0x01),
        Error::InvalidCellPointer { offset: 109, min: 110, usable: 512 },
        Error::CellIndexOutOfBounds { index: 1, cells: 1 },
        Error::InvalidIntegerWidth(0),
    ] {
        assert!(!err.to_string().is_empty());
    }
}

#[test]
fn signed_be_integers_decode_with_sign_extension() {
    assert_eq!(decode_signed_be_int(&[0x7f]), Ok(127));
    assert_eq!(decode_signed_be_int(&[0x80]), Ok(-128));
    assert_eq!(decode_signed_be_int(&[0x80, 0x00, 0x01]), Ok(-8_388_607));
    assert_eq!(decode_signed_be_int(&[0xff, 0xff, 0xff, 0xff, 0xff, 0xfe]), Ok(-2));
    assert_eq!(decode_signed_be_int(&[0x7f, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff]), Ok(i64::MAX));
    assert_eq!(decode_signed_be_int(&[0x80, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00]), Ok(i64::MIN));
    assert_eq!(decode_signed_be_int(&[]), Err(Error::InvalidIntegerWidth(0)));
    assert_eq!(decode_signed_be_int(&[0; 9]), Err(Error::InvalidIntegerWidth(9)));
}

#[test]
fn btree_page_header_and_cell_pointer_bounds() {
    let mut page = [0u8; 512];
    page[100] = 0x0d;
    page[103..105].copy_from_slice(&1u16.to_be_bytes());
    page[108..110].copy_from_slice(&400u16.to_be_bytes());

    let hdr = parse_btree_page_header(&page, true).unwrap();
    assert_eq!(hdr, BtreePageHeader { kind: 0x0d, header_offset: 100, cells: 1, pointers_offset: 108, pointers_end: 110 });
    assert_eq!(btree_cell_offset(&page, &hdr, 0), Ok(400));
    assert_eq!(btree_cell_offset(&page, &hdr, 1), Err(Error::CellIndexOutOfBounds { index: 1, cells: 1 }));

    // Pointer inside header/pointer array is rejected.
    page[108..110].copy_from_slice(&109u16.to_be_bytes());
    assert_eq!(btree_cell_offset(&page, &hdr, 0), Err(Error::InvalidCellPointer { offset: 109, min: 110, usable: 512 }));

    // Unsupported page kind and truncated pages are rejected.
    page[0] = 0x01;
    assert_eq!(parse_btree_page_header(&page, false), Err(Error::UnsupportedBtreePageType(0x01)));
    assert_eq!(parse_btree_page_header(&[], false), Err(Error::TruncatedPage));
    assert_eq!(parse_btree_page_header(&[0x0d, 0, 0, 0], false), Err(Error::TruncatedPage));

    // Interior page with pointer array exceeding usable page is rejected.
    page[0] = 0x05;
    page[3..5].copy_from_slice(&300u16.to_be_bytes());
    assert_eq!(parse_btree_page_header(&page, false), Err(Error::TruncatedPage));

    // Cell pointer >= usable is rejected.
    page[0] = 0x0d;
    page[3..5].copy_from_slice(&1u16.to_be_bytes());
    page[8..10].copy_from_slice(&512u16.to_be_bytes());
    let small_hdr = parse_btree_page_header(&page, false).unwrap();
    assert_eq!(btree_cell_offset(&page, &small_hdr, 0), Err(Error::InvalidCellPointer { offset: 512, min: 10, usable: 512 }));
}
