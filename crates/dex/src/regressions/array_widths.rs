use crate::dex_ops::{Payload, decode, decode_one};

#[test]
fn zero_width_does_not_describe_a_huge_complete_array() {
    let code = [0x0300, 0, 0xffff, 0xffff];
    assert!(decode_one(&code, 0).is_none());
    assert!(!decode(&code, 16).1);
}

#[test]
fn unsupported_element_widths_are_refused_even_for_empty_arrays() {
    for width in [0, 3, 5, 6, 7, 9, 16, u16::MAX] {
        assert!(decode_one(&[0x0300, width, 0, 0], 0).is_none(), "width {width}");
    }
}

#[test]
fn all_primitive_widths_and_odd_byte_padding_still_decode() {
    for width in [1u16, 2, 4, 8] {
        let count = 3u32;
        let bytes = usize::from(width) * count as usize;
        let mut code = vec![0x0300, width, count as u16, 0];
        code.resize(4 + bytes.div_ceil(2), 0x2211);
        let instruction = decode_one(&code, 0).unwrap();
        assert_eq!(instruction.units as usize, code.len());
        match instruction.payload.unwrap() {
            Payload::FillArrayData { element_width, count: decoded, data } => {
                assert_eq!(element_width, width);
                assert_eq!(decoded, count);
                assert_eq!(data.len(), bytes);
                assert_eq!(data[0], 0x11);
            }
            other => panic!("unexpected payload {other:?}"),
        }
        assert!(decode_one(&[0x0300, width, 0, 0], 0).is_some());
        code.pop();
        assert!(decode_one(&code, 0).is_none());
    }
}
