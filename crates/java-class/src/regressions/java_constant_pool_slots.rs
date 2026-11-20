use crate::{FormatError, java_class};

fn class_with_pool(count: u16, pool: &[u8]) -> Vec<u8> {
    let mut bytes = vec![0xca, 0xfe, 0xba, 0xbe, 0, 0, 0, 52];
    bytes.extend_from_slice(&count.to_be_bytes());
    bytes.extend_from_slice(pool);
    bytes.extend_from_slice(&[0; 14]);
    bytes
}

#[test]
fn a_wide_constant_needs_its_reserved_slot() {
    for tag in [5, 6] {
        let mut pool = vec![tag];
        pool.extend_from_slice(&[0; 8]);
        let bytes = class_with_pool(2, &pool);
        assert!(matches!(java_class::parse(&bytes), Err(FormatError::Malformed(message)) if message.contains("second slot")));
    }
}

#[test]
fn the_largest_pool_cannot_overflow_on_a_trailing_wide_constant() {
    for tag in [5, 6] {
        let mut pool = [3, 0, 0, 0, 0].repeat(usize::from(u16::MAX) - 2);
        pool.push(tag);
        pool.extend_from_slice(&[0; 8]);
        let bytes = class_with_pool(u16::MAX, &pool);
        assert!(matches!(java_class::parse(&bytes), Err(FormatError::Malformed(message)) if message.contains("second slot")));
    }
}

#[test]
fn a_wide_constant_with_two_remaining_slots_is_retained() {
    for tag in [5, 6] {
        for count in [3, u16::MAX] {
            let mut pool = [3, 0, 0, 0, 0].repeat(usize::from(count) - 3);
            pool.push(tag);
            pool.extend_from_slice(&[0; 8]);
            let parsed = java_class::parse(&class_with_pool(count, &pool)).unwrap();
            assert_eq!(parsed.pool.len(), usize::from(count));
            assert!(matches!(parsed.pool.last(), Some(java_class::Const::Unusable)));
            assert!(matches!(parsed.pool[usize::from(count) - 2], java_class::Const::Long(_) | java_class::Const::Double(_)));
        }
    }
}

#[test]
fn ordinary_final_constants_and_existing_classes_still_parse() {
    let parsed = java_class::parse(&class_with_pool(2, &[3, 0, 0, 0, 7])).unwrap();
    assert_eq!(parsed.pool, vec![java_class::Const::Unusable, java_class::Const::Integer(7)]);
    let parsed = java_class::parse(&crate::test_support::java::hello_class()).unwrap();
    assert!(!parsed.methods.is_empty());
}
