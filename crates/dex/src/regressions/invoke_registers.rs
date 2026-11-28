use crate::dex_ops::{decode, decode_one};

#[test]
fn fixed_register_lists_cannot_claim_more_than_five_arguments() {
    for op in [0x24u16, 0x6e, 0x6f, 0x70, 0x71, 0x72, 0xfa, 0xfc] {
        for count in 6..=15 {
            let code = [(count << 12) | op, 1, 0x4321, 2];
            assert!(decode_one(&code, 0).is_none(), "opcode {op:#x}, count {count}");
            assert!(!decode(&code, 16).1);
        }
    }
}

#[test]
fn range_arguments_cannot_leave_the_encoded_register_space() {
    for op in [0x25u16, 0x74, 0x75, 0x76, 0x77, 0x78, 0xfb, 0xfd] {
        assert!(decode_one(&[0x0200 | op, 1, u16::MAX, 2], 0).is_none(), "{op:#x}");
        assert!(decode_one(&[0xff00 | op, 1, u16::MAX - 1, 2], 0).is_none(), "{op:#x}");
    }
}

#[test]
fn zero_and_boundary_register_lists_remain_valid() {
    let fixed = decode_one(&[0x5571, 1, 0x4321], 0).unwrap();
    assert_eq!(fixed.regs, vec![1, 2, 3, 4, 5]);
    assert!(decode_one(&[0x0071, 1, 0], 0).unwrap().regs.is_empty());
    let range = decode_one(&[0x0277, 1, u16::MAX - 1], 0).unwrap();
    assert_eq!(range.regs, vec![65534, 65535]);
    assert!(decode_one(&[0x0077, 1, u16::MAX], 0).unwrap().regs.is_empty());
    let polymorphic = decode_one(&[0x01fb, 1, u16::MAX, 2], 0).unwrap();
    assert_eq!(polymorphic.regs, vec![65535]);
    assert_eq!(polymorphic.index2, Some(2));
}
