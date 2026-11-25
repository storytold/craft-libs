use crate::test_support::formats::wasm_module;
use crate::{MAX_ITEMS, inspect};

use crate::test_support::{leb, string};

fn custom(module: &mut Vec<u8>, name: &str, data: &[u8]) {
    let mut body = Vec::new();
    string(&mut body, name);
    body.extend(data);
    module.push(0);
    module.extend(leb(body.len()));
    module.extend(body);
}

fn names(module: &mut Vec<u8>, entries: &[(usize, &str)]) {
    let mut map = leb(entries.len());
    for (index, name) in entries {
        map.extend(leb(*index));
        string(&mut map, name);
    }
    let mut body = vec![1];
    body.extend(leb(map.len()));
    body.extend(map);
    custom(module, "name", &body);
}

fn producers(fields: usize, values: usize) -> Vec<u8> {
    let mut body = leb(fields);
    for _ in 0..fields {
        string(&mut body, "language");
        body.extend(leb(values));
        for _ in 0..values {
            string(&mut body, "fixture");
            string(&mut body, "1");
        }
    }
    body
}

#[test]
fn repeated_name_sections_share_a_limit_but_can_update_retained_functions() {
    let mut bytes = b"\0asm\x01\0\0\0\x01\x04\x01\x60\0\0\x03\x02\x01\0\x0a\x04\x01\x02\0\x0b".to_vec();
    let entries: Vec<_> = (0..MAX_ITEMS).map(|index| (index, "kept")).collect();
    names(&mut bytes, &entries);
    let complete = inspect(&bytes).unwrap();
    assert!(complete.validation.valid);
    assert!(!complete.truncated);
    assert_eq!(complete.functions[0].name.as_deref(), Some("kept"));
    names(&mut bytes, &[(0, "updated"), (MAX_ITEMS, "omitted")]);
    let report = inspect(&bytes).unwrap();
    assert!(report.validation.valid);
    assert!(report.parse_error.is_none());
    assert!(report.truncated);
    assert_eq!(report.functions[0].name.as_deref(), Some("updated"));
}

#[test]
fn producer_values_are_bounded_across_repeated_sections() {
    let mut bytes = b"\0asm\x01\0\0\0".to_vec();
    let section = producers(1, 64);
    for _ in 0..MAX_ITEMS / 64 + 1 {
        custom(&mut bytes, "producers", &section);
    }
    let report = inspect(&bytes).unwrap();
    assert!(report.validation.valid);
    assert!(report.parse_error.is_none());
    assert!(report.truncated);
    assert_eq!(report.producers.iter().map(|field| field.values.len()).sum::<usize>(), MAX_ITEMS);
    assert_eq!(report.producers[0].values[0].name, "fixture");
}

#[test]
fn empty_producer_fields_still_spend_the_global_field_limit() {
    let mut bytes = b"\0asm\x01\0\0\0".to_vec();
    let section = producers(64, 0);
    for _ in 0..MAX_ITEMS / 64 + 1 {
        custom(&mut bytes, "producers", &section);
    }
    let report = inspect(&bytes).unwrap();
    assert!(report.validation.valid);
    assert_eq!(report.producers.len(), MAX_ITEMS);
    assert!(report.truncated);
}

#[test]
fn existing_per_section_limits_are_reported_as_truncation() {
    for (fields, values) in [(65, 0), (1, 65)] {
        let mut bytes = b"\0asm\x01\0\0\0".to_vec();
        custom(&mut bytes, "producers", &producers(fields, values));
        let report = inspect(&bytes).unwrap();
        assert!(report.truncated, "{fields} fields with {values} values");
    }
    let mut bytes = b"\0asm\x01\0\0\0".to_vec();
    let mut subsections = vec![0, 2, 1, b'm'];
    // Name subsection IDs must increase, including unknown extension IDs.
    for id in 20..84 {
        subsections.extend([id, 0]);
    }
    custom(&mut bytes, "name", &subsections);
    let report = inspect(&bytes).unwrap();
    assert!(report.truncated);
    assert_eq!(report.module_name.as_deref(), Some("m"));
}

#[test]
fn ordinary_names_and_producers_remain_complete() {
    let report = inspect(&wasm_module()).unwrap();
    assert!(report.validation.valid);
    assert!(!report.truncated);
    assert_eq!(report.functions[0].name.as_deref(), Some("add"));
    assert_eq!(report.module_name.as_deref(), Some("demo"));
    assert_eq!(report.producers.len(), 2);
    assert_eq!(report.producers[1].values[0].name, "rustc");
    assert_eq!(report.exports.iter().find(|export| export.name == "add").unwrap().target_name.as_deref(), Some("add"));
}
