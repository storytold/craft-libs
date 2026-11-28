use crate::java_class;
use crate::java_ops::{decode, decode_one};

#[test]
fn invokeinterface_requires_a_count_and_zero_reserved_byte() {
    for code in [[0xb9, 0, 1, 0, 0], [0xb9, 0, 1, 1, 1], [0xb9, 0, 1, 255, 255]] {
        assert!(decode_one(&code, 0).is_none(), "{code:?}");
        assert!(!decode(&code, 16).1);
    }
}

#[test]
fn invokedynamic_requires_both_reserved_bytes_to_be_zero() {
    for code in [[0xba, 0, 1, 1, 0], [0xba, 0, 1, 0, 1], [0xba, 0, 1, 255, 255]] {
        assert!(decode_one(&code, 0).is_none(), "{code:?}");
        assert!(!decode(&code, 16).1);
    }
}

#[test]
fn valid_and_truncated_invocations_keep_their_boundaries() {
    for count in [1, 2, 255] {
        let instruction = decode_one(&[0xb9, 0x12, 0x34, count, 0], 0).unwrap();
        assert_eq!(instruction.cp_index, Some(0x1234));
        assert_eq!(instruction.len, 5);
    }
    let instruction = decode_one(&[0xba, 0x12, 0x34, 0, 0], 0).unwrap();
    assert_eq!(instruction.cp_index, Some(0x1234));
    for code in [[0xb9, 0, 1, 1, 0], [0xba, 0, 1, 0, 0]] {
        for len in 1..5 {
            assert!(decode_one(&code[..len], 0).is_none());
        }
    }
    let (instructions, complete) = decode(&[0xb9, 0, 1, 1, 0, 0xba, 0, 2, 0, 0, 0xb1], 16);
    assert!(complete);
    assert_eq!(instructions.len(), 3);
    assert_eq!(instructions[2].pc, 10);
}

fn reference_class() -> java_class::ClassFile {
    let mut class = java_class::parse(&crate::test_support::java::hello_class()).unwrap();
    class.methods.retain(|method| method.name == "run");
    class
}

#[test]
fn malformed_invocations_mark_reference_scans_incomplete() {
    let valid = reference_class();
    let (expected, incomplete) = java_class::references(&valid, "java/io/PrintStream", None, 10);
    assert!(!incomplete);
    assert_eq!(expected.len(), 1);
    for invalid in [[0xb9, 0, 1, 0, 0], [0xb9, 0, 1, 1, 1], [0xba, 0, 1, 1, 0], [0xba, 0, 1, 0, 1]] {
        let mut class = valid.clone();
        let code = &mut class.methods[0].code.as_mut().unwrap().bytecode;
        let later_reference = code.clone();
        *code = invalid.to_vec();
        code.extend_from_slice(&later_reference);
        let (found, incomplete) = java_class::references(&class, "java/io/PrintStream", None, 10);
        assert!(found.is_empty());
        assert!(incomplete, "invalid invocation must not hide an unfinished reference scan: {invalid:?}");
    }
}

#[test]
fn incomplete_scans_keep_earlier_references_and_scan_other_methods() {
    let mut class = reference_class();
    let mut later_method = class.methods[0].clone();
    later_method.name = "later".into();
    class.methods[0].code.as_mut().unwrap().bytecode.extend_from_slice(&[0xba, 0, 1, 1, 0]);
    class.methods.push(later_method);
    let (found, incomplete) = java_class::references(&class, "java/io/PrintStream", None, 10);
    assert!(incomplete);
    assert_eq!(found.len(), 2);
    assert!(found[0].caller.contains(".run:("));
    assert!(found[1].caller.contains(".later:("));
}
