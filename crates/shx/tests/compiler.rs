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
