use crate::bytes::Reader;
use crate::dex::{self, Code, Handler, sleb128};

#[test]
fn rejects_invalid_fifth_byte_sign_extensions() {
    for prefix in [0x80, 0xff] {
        for last in 0x08..=0x77 {
            let mut encoded = vec![prefix; 4];
            encoded.push(last);
            assert_eq!(sleb128(&Reader::new(&encoded), 0), None, "last byte {last:#x}");
        }
    }
}

#[test]
fn retains_signed_32_bit_limits_and_ordinary_values() {
    assert_eq!(sleb128(&Reader::new(&[0xff, 0xff, 0xff, 0xff, 0x07]), 0), Some((i64::from(i32::MAX), 5)));
    assert_eq!(sleb128(&Reader::new(&[0x80, 0x80, 0x80, 0x80, 0x78]), 0), Some((i64::from(i32::MIN), 5)));
    assert_eq!(sleb128(&Reader::new(&[0x9b, 0xf1, 0x59]), 0), Some((-624485, 3)));
    assert_eq!(sleb128(&Reader::new(&[0xe5, 0x8e, 0x26]), 0), Some((624485, 3)));
    assert_eq!(sleb128(&Reader::new(&[0xff, 0x7f]), 0), Some((-1, 2)));
    assert_eq!(sleb128(&Reader::new(&[0x80, 0]), 0), Some((0, 2)));
    assert_eq!(sleb128(&Reader::new(&[0x55, 0x7f, 0x55]), 1), Some((-1, 1)));
}

#[test]
fn retains_valid_five_byte_sign_extensions() {
    assert_eq!(sleb128(&Reader::new(&[0x80, 0x80, 0x80, 0x80, 0]), 0), Some((0, 5)));
    assert_eq!(sleb128(&Reader::new(&[0x81, 0x80, 0x80, 0x80, 0]), 0), Some((1, 5)));
    assert_eq!(sleb128(&Reader::new(&[0xff, 0xff, 0xff, 0xff, 0x7f]), 0), Some((-1, 5)));
}

#[test]
fn rejects_values_longer_than_five_bytes() {
    for (prefix, last) in [(0x80, 0), (0xff, 0x7f)] {
        for length in [6, 10] {
            let mut encoded = vec![prefix; length - 1];
            encoded.push(last);
            assert_eq!(sleb128(&Reader::new(&encoded), 0), None, "{encoded:x?}");
        }
    }
}

#[test]
fn rejects_truncated_values_and_out_of_range_offsets() {
    assert_eq!(sleb128(&Reader::new(&[0x80; 5]), 0), None);
    assert_eq!(sleb128(&Reader::new(&[0x80]), 0), None);
    assert_eq!(sleb128(&Reader::new(&[]), 0), None);
    assert_eq!(sleb128(&Reader::new(&[0]), usize::MAX), None);
}

fn code_with_handler(size: &[u8], payload: &[u8]) -> Code {
    let mut bytes = vec![0; 0x70];
    bytes[..8].copy_from_slice(b"dex\n035\0");
    bytes[36..40].copy_from_slice(&0x70u32.to_le_bytes());
    bytes[40..44].copy_from_slice(&0x1234_5678u32.to_le_bytes());
    // Eight nop instructions and one try entry pointing to the first encoded handler.
    bytes.extend_from_slice(&[1, 0, 0, 0, 0, 0, 1, 0, 0, 0, 0, 0, 8, 0, 0, 0]);
    bytes.extend_from_slice(&[0; 16]);
    bytes.extend_from_slice(&[0, 0, 0, 0, 1, 0, 1, 0]);
    bytes.push(1);
    bytes.extend_from_slice(size);
    bytes.extend_from_slice(payload);
    let len = bytes.len() as u32;
    bytes[32..36].copy_from_slice(&len.to_le_bytes());
    dex::parse(&bytes).unwrap().code(0x70).unwrap()
}

#[test]
fn invalid_sizes_do_not_create_handlers() {
    let sizes: &[&[u8]] = &[
        &[0x80, 0x80, 0x80, 0x80, 0x08],
        &[0x80, 0x80, 0x80, 0x80, 0x77],
        &[0x80, 0x80, 0x80, 0x80, 0x80, 0],
        &[0x80, 0x80, 0x80, 0x80, 0x80, 0x80, 0x80, 0x80, 0x80, 0],
    ];
    for size in sizes {
        // These bytes could be a type/address pair or a catch-all address if size is accepted.
        let code = code_with_handler(size, &[0, 7]);
        assert_eq!(code.tries.len(), 1);
        assert!(code.tries[0].handlers.is_empty(), "invalid size {size:x?}");
    }
}

#[test]
fn valid_catch_all_handlers_are_retained() {
    for size in [&[0][..], &[0x80, 0x80, 0x80, 0x80, 0][..]] {
        let code = code_with_handler(size, &[7]);
        assert_eq!(code.tries.len(), 1);
        assert_eq!(code.tries[0].start, 0);
        assert_eq!(code.tries[0].end, 1);
        assert_eq!(code.tries[0].handlers, vec![Handler { exception: None, address: 7 }]);
    }
}
