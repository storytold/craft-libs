use crate::{MAX_ITEMS, inspect};

const COMPONENT: &[u8] = b"\0asm\x0d\0\x01\0";

/// Only outer sections appear, including sections after embedded binaries.
#[test]
fn lists_component_sections_without_descending_into_embedded_modules() {
    let mut bytes = COMPONENT.to_vec();
    bytes.extend(b"\x00\x04\x03one");
    bytes.extend(b"\x01\x0b\0asm\x01\0\0\0\x01\x01\0");
    bytes.extend(b"\x04\x0e\0asm\x0d\0\x01\0\x00\x04\x03two");
    bytes.extend(b"\x00\x06\x05three");
    let report = inspect(&bytes).unwrap();
    assert!(report.validation.valid, "{:?}", report.validation);
    let layout: Vec<_> = report.sections.iter().map(|s| (s.id, s.name.as_str(), s.offset, s.size)).collect();
    assert_eq!(layout, [(0, "custom:one", 10, 4), (1, "core module", 16, 11), (4, "component", 29, 14), (0, "custom:three", 45, 6)]);
    assert!(report.functions.is_empty());
    assert!(report.parse_error.is_none());
}

/// A short section reports damage without losing the preceding layout.
#[test]
fn truncated_component_keeps_its_complete_prefix() {
    let mut bytes = COMPONENT.to_vec();
    bytes.extend(b"\x00\x04\x03one\x01\x08\0asm");
    let report = inspect(&bytes).unwrap();
    assert_eq!(report.sections.len(), 1);
    assert!(report.parse_error.is_some());
}

/// Component layouts use the same cap as core module layouts.
#[test]
fn component_section_limit_is_disclosed() {
    let mut bytes = COMPONENT.to_vec();
    for _ in 0..=MAX_ITEMS {
        bytes.extend(b"\x00\x02\x01x");
    }
    let report = inspect(&bytes).unwrap();
    assert!(report.validation.valid);
    assert_eq!(report.sections.len(), MAX_ITEMS);
    assert!(report.truncated);
}
