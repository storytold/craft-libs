use crate::java_ops::{decode, decode_one};

#[test]
fn only_defined_wide_opcodes_are_accepted() {
    for op in 0..=255u8 {
        let valid = matches!(op, 0x15..=0x19 | 0x36..=0x3a | 0x84 | 0xa9);
        let code = [0xc4, op, 0x01, 0x2c, 0xff, 0xfe];
        assert_eq!(decode_one(&code, 0).is_some(), valid, "opcode {op:#x}");
    }
}

#[test]
fn invalid_wide_does_not_invent_a_method_body() {
    let (instructions, complete) = decode(&[0x00, 0xc4, 0xb1, 0, 1, 0xb1], 32);
    assert!(!complete);
    assert_eq!(instructions.len(), 1);
    assert_eq!(instructions[0].name, "nop");
}

#[test]
fn wide_locals_and_signed_increments_are_preserved() {
    for op in [0x15, 0x16, 0x17, 0x18, 0x19, 0x36, 0x37, 0x38, 0x39, 0x3a, 0xa9] {
        let instruction = decode_one(&[0xc4, op, 0xff, 0xff], 0).unwrap();
        assert_eq!(instruction.local, Some(u16::MAX));
        assert_eq!(instruction.len, 4);
        assert!(instruction.wide);
    }
    let instruction = decode_one(&[0xc4, 0x84, 0x01, 0x2c, 0xff, 0xfe], 0).unwrap();
    assert_eq!(instruction.local, Some(300));
    assert_eq!(instruction.int, Some(-2));
    assert_eq!(instruction.len, 6);
    assert!(decode_one(&[0xc4, 0x84, 0x00, 1], 0).is_none());
}
