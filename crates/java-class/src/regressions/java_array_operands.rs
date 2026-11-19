use crate::java_ops::{decode, decode_one};

#[test]
fn invalid_primitive_array_types_stop_before_the_allocation() {
    for atype in (0..=u8::MAX).filter(|value| !(4..=11).contains(value)) {
        assert!(decode_one(&[0xbc, atype], 0).is_none(), "atype {atype}");
        let (instructions, complete) = decode(&[0, 0xbc, atype, 0xb1], 100);
        assert!(!complete, "atype {atype}");
        assert_eq!(instructions.len(), 1);
        assert_eq!(instructions[0].name, "nop");
    }
}

#[test]
fn zero_dimension_multidimensional_allocations_are_not_complete_bytecode() {
    assert!(decode_one(&[0xc5, 0, 1, 0], 0).is_none());
    let (instructions, complete) = decode(&[0, 0xc5, 0, 1, 0, 0xb1], 100);
    assert!(!complete);
    assert_eq!(instructions.len(), 1);
    assert_eq!(instructions[0].name, "nop");
}

#[test]
fn every_defined_primitive_type_and_nonzero_dimension_remains_decodable() {
    for atype in 4..=11 {
        let (instructions, complete) = decode(&[0xbc, atype, 0xb1], 100);
        assert!(complete);
        assert_eq!(instructions.len(), 2);
        assert_eq!(instructions[0].int, Some(i32::from(atype)));
        assert_eq!(instructions[0].len, 2);
    }
    for dimensions in 1..=u8::MAX {
        let instruction = decode_one(&[0, 0xc5, 0x12, 0x34, dimensions], 1).unwrap();
        assert_eq!(instruction.pc, 1);
        assert_eq!(instruction.cp_index, Some(0x1234));
        assert_eq!(instruction.int, Some(i32::from(dimensions)));
        assert_eq!(instruction.len, 4);
    }
}

#[test]
fn incomplete_array_operands_remain_rejected() {
    for instruction in [&[0xbc, 10][..], &[0xc5, 0, 1, 2][..]] {
        for end in 1..instruction.len() {
            assert!(decode_one(&instruction[..end], 0).is_none());
        }
    }
}
