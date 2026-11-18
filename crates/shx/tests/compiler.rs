use craft_shx::{ShxError, compile};

#[test]
fn fixed_shape_bytes() {
    let b = compile("*65,2,A\n010,0\n").unwrap();
    let mut expected = b"AutoCAD-86 shapes 1.0\r\n\x1a".to_vec();
    expected.extend([65, 0, 65, 0, 1, 0, 65, 0, 4, 0, b'A', 0, 0x10, 0]);
    expected.extend(b"EOF");
    assert_eq!(b, expected);
}
#[test]
fn hostile_sources_are_refused() {
    for src in [
        "*1,2,X\n7\n",
        "*1,2,X\n9,(1,1)\n",
        "*99999,1,X\n0\n",
        "*1,1,X\n300,0\n",
        "010,0\n",
        "*1,1,X\n1,0zz,0\n",
        "*1,1,X\n0\n*1,1,Y\n0\n",
        "*--9223372036854775808,1,X\n0",
        "*BIGFONT 1,65535\n*1,1,X\n0",
    ] {
        assert!(compile(src).is_err(), "{src:?}");
    }
    assert_eq!(compile(""), Err(ShxError::Empty));
    assert_eq!(compile(&" ".repeat((8 << 20) + 1)), Err(ShxError::TooLarge));
    let oversized_header = format!("*UNIFONT,6,{}\n10,2,0,0,0,0\n*65,1,A\n0", "n".repeat(65536));
    assert_eq!(compile(&oversized_header), Err(ShxError::TooLarge));
}

#[test]
fn displacement_arc_and_subshape_operands_have_exact_bytes() {
    let source = "; comment\n*+65,99,A\n3,2,4,3,7,66,8,-128,127,9,(1,-2),(0,0),\n10,12,-043,11,1,2,3,4,043,12,-1,2,-3,13,1,2,-3,0,0,0\n";
    let bytes = compile(source).unwrap();
    let program = [3, 2, 4, 3, 7, 66, 8, 128, 127, 9, 1, 254, 0, 0, 10, 12, 0xc3, 11, 1, 2, 3, 4, 0x43, 12, 255, 2, 253, 13, 1, 2, 253, 0, 0, 0];
    let mut record = vec![b'A', 0];
    record.extend(program);
    record.extend(b"EOF");
    assert!(bytes.ends_with(&record));
}

#[test]
fn bigfont_ranges_index_offsets_and_extended_calls_are_encoded() {
    let bytes = compile("*BIGFONT 1,2,128,129,140,141\n*65,1,A\n7,66,7,0,01234,1,2,3,4,0\n").unwrap();
    let signature = b"AutoCAD-86 bigfont 1.0\r\n\x1a";
    let mut expected = signature.to_vec();
    expected.extend([1, 0, 1, 0, 2, 0, 128, 0, 129, 0, 140, 0, 141, 0]);
    let record = [b'A', 0, 7, 66, 7, 0, 0x12, 0x34, 1, 2, 3, 4, 0];
    let offset = (expected.len() + 8) as u32;
    expected.extend([65, 0, record.len() as u8, 0]);
    expected.extend(offset.to_le_bytes());
    expected.extend(record);
    assert_eq!(bytes, expected);
}

#[test]
fn unicode_font_records_and_subshape_numbers_are_encoded() {
    let bytes = compile("*UNIFONT,6,Example\n10,2,0\n0,0,0\n*65,9,A\n7,01234\n").unwrap();
    let mut expected = b"AutoCAD-86 unifont 1.0\r\n\x1a".to_vec();
    expected.extend(2u32.to_le_bytes());
    expected.extend(14u16.to_le_bytes());
    expected.extend(b"Example\0");
    expected.extend([10, 2, 0, 0, 0, 0]);
    expected.extend([65, 0, 6, 0, b'A', 0, 7, 0x12, 0x34, 0]);
    assert_eq!(bytes, expected);
}

#[test]
fn font_records_sorting_names_and_implicit_end_are_preserved() {
    let bytes = compile("*66,1,B\0name\n1\n*0,4,Font\n10,2,0,0\n*65,1,A\n2,0\n").unwrap();
    let mut expected = b"AutoCAD-86 shapes 1.0\r\n\x1a".to_vec();
    expected.extend([0, 0, 66, 0, 3, 0]);
    let records: [(u16, &[u8]); 3] = [(0, b"Font\0\x0a\x02\0\0"), (65, b"A\0\x02\0"), (66, b"Bname\0\x01\0")];
    for (number, record) in records {
        expected.extend(number.to_le_bytes());
        expected.extend((record.len() as u16).to_le_bytes());
    }
    for (_, record) in records {
        expected.extend(record);
    }
    expected.extend(b"EOF");
    assert_eq!(bytes, expected);
}

#[test]
fn invalid_typed_operands_report_source_lines() {
    for body in ["8,128,0", "10,1,128", "11,1,2,3,4,-128", "7,-1", "3,-1", "256", "7,256"] {
        assert!(matches!(compile(&format!("*65,1,A\n{body}")), Err(ShxError::Source(1, _))), "{body}");
    }
    for source in ["*UNIFONT,6,X\n256\n*65,1,A\n0", "*BIGFONT 1,1,-1,2\n*65,1,A\n0", "*0,1,F\n256", "*-1,1,A\n0"] {
        assert!(matches!(compile(source), Err(ShxError::Source(_, _))), "{source}");
    }
    let large_record = format!("*65,1,{}\n0", "x".repeat(65535));
    assert_eq!(compile(&large_record), Err(ShxError::TooLarge));
}

#[test]
fn errors_have_useful_display_messages() {
    for (error, expected) in [
        (ShxError::NotShx, "not an SHX shape or font file"),
        (ShxError::Truncated, "the SHX file is truncated"),
        (ShxError::TooLarge, "the SHX file is too large"),
        (ShxError::Empty, "the SHX file has no shapes"),
        (ShxError::Source(7, "bad operand".into()), "line 7: bad operand"),
    ] {
        assert_eq!(error.to_string(), expected);
    }
}
