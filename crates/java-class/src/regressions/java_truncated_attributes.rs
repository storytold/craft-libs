use crate::java_class::parse;
use crate::test_support::java::hello_class;

/// The fixture ends in a class attribute count and a single two-byte SourceFile attribute.
const ATTRIBUTE_TABLE_BYTES: usize = 10;

/// Every cut of the attribute count, header or body leaves a partial class report.
#[test]
fn incomplete_class_attribute_table_reports_a_warning() {
    let data = hello_class();
    let start = data.len() - ATTRIBUTE_TABLE_BYTES;
    for end in start..data.len() {
        let class = parse(&data[..end]).unwrap();
        assert_eq!(class.methods.len(), 4);
        assert!(class.source_file.is_none());
        assert!(class.warnings.iter().any(|warning| warning.contains("class attribute table")), "cut at {}", end - start);
    }
}

/// A complete attribute stays available when a subsequent attribute is cut short.
#[test]
fn completed_class_attributes_survive_a_truncated_record() {
    let mut data = hello_class();
    let start = data.len() - ATTRIBUTE_TABLE_BYTES;
    data[start..start + 2].copy_from_slice(&2u16.to_be_bytes());
    data.extend_from_slice(&[0, 1, 0, 0, 0, 2, 0]);
    let class = parse(&data).unwrap();
    assert_eq!(class.source_file.as_deref(), Some("Hello.java"));
    assert!(class.warnings.iter().any(|warning| warning.contains("class attribute table")));
}

/// Complete, empty and unknown class attributes do not trigger a truncation warning.
#[test]
fn complete_class_attribute_tables_are_accepted() {
    let mut data = hello_class();
    let start = data.len() - ATTRIBUTE_TABLE_BYTES;
    let original = parse(&data).unwrap();
    assert_eq!(original.source_file.as_deref(), Some("Hello.java"));
    assert!(original.warnings.is_empty());
    data[start..start + 2].copy_from_slice(&2u16.to_be_bytes());
    data.extend_from_slice(&[0, 1, 0, 0, 0, 1, 0]);
    assert!(parse(&data).unwrap().warnings.is_empty());
    data.truncate(start + 2);
    data[start..].fill(0);
    assert!(parse(&data).unwrap().warnings.is_empty());
}
