use crate::{WasmReport, inspect};

use crate::test_support::{leb, string};

fn section(module: &mut Vec<u8>, id: u8, body: &[u8]) {
    module.push(id);
    module.extend(leb(body.len()));
    module.extend(body);
}

fn shared_type(functions: usize, imports: usize, imported_tags: usize, defined_tags: usize, params: usize) -> Vec<u8> {
    let mut module = b"\0asm\x01\0\0\0".to_vec();
    let mut ty = vec![1, 0x60];
    ty.extend(leb(params));
    ty.extend(std::iter::repeat_n(0x7f, params));
    ty.push(0);
    section(&mut module, 1, &ty);
    if imports + imported_tags > 0 {
        let mut body = leb(imports + imported_tags);
        for i in 0..imports + imported_tags {
            string(&mut body, "env");
            string(&mut body, "item");
            if i < imports {
                body.extend([0, 0]);
            } else {
                body.extend([4, 0, 0]);
            }
        }
        section(&mut module, 2, &body);
    }
    if functions > 0 {
        let mut body = leb(functions);
        body.extend(std::iter::repeat_n(0, functions));
        section(&mut module, 3, &body);
    }
    if defined_tags > 0 {
        let mut body = leb(defined_tags);
        for _ in 0..defined_tags {
            body.extend([0, 0]);
        }
        section(&mut module, 13, &body);
    }
    if functions > 0 {
        let mut body = leb(functions);
        for _ in 0..functions {
            body.extend([2, 0, 0x0b]);
        }
        section(&mut module, 10, &body);
    }
    module
}

fn derived_bytes(report: &WasmReport) -> usize {
    report.imports.iter().map(|item| item.signature.len()).sum::<usize>()
        + report.tags.iter().map(|item| item.signature.len()).sum::<usize>()
        + report.functions.iter().map(|item| item.signature.len() + item.name.as_ref().map_or(0, String::len)).sum::<usize>()
        + report.exports.iter().map(|item| item.target_name.as_ref().map_or(0, String::len)).sum::<usize>()
}

fn bounded(module: &[u8]) -> WasmReport {
    assert!(module.len() < 1 << 20);
    let report = inspect(module).unwrap();
    assert!(report.parse_error.is_none());
    assert!(derived_bytes(&report) <= 64 << 20, "{} input bytes retain {} derived string bytes", module.len(), derived_bytes(&report));
    assert!(report.truncated);
    assert!(report.warnings.iter().any(|warning| warning.contains("byte budget")), "{:?}", report.warnings);
    report
}

#[test]
fn repeated_function_signatures_have_a_byte_budget() {
    let report = bounded(&shared_type(16_384, 0, 0, 0, 1000));
    assert!(report.validation.valid, "{:?}", report.validation);
    assert!(!report.functions.is_empty());
    assert!(report.functions.len() < 16_384);
    assert!(
        report.functions.iter().enumerate().all(|(index, function)| function.index as usize == index && function.signature.starts_with("(i32, i32"))
    );
    assert!(!report.warnings.iter().any(|warning| warning.contains("function declarations")));
}

#[test]
fn repeated_import_signatures_stay_bounded_even_when_validation_fails() {
    let report = bounded(&shared_type(0, 16_384, 0, 0, 1000));
    assert!(!report.validation.valid);
    assert!(report.validation.error.as_deref().unwrap().contains("effective type size"));
    assert_eq!(report.imported_functions, 16_384);
    assert!(!report.imports.is_empty());
    assert!(report.imports.len() < 16_384);
}

#[test]
fn imported_and_defined_tags_share_the_same_budget_as_functions() {
    for (functions, imports, imported_tags, defined_tags) in [(0, 0, 8192, 0), (0, 0, 0, 16_384), (4096, 4096, 4096, 4096)] {
        let report = bounded(&shared_type(functions, imports, imported_tags, defined_tags, 1000));
        assert_eq!(report.imported_functions as usize, imports);
        assert!(report.tags.len() < imported_tags + defined_tags);
        assert!(report.tags.iter().all(|tag| tag.signature.starts_with("(i32, i32")));
        assert!(report.tags.iter().filter(|tag| !tag.imported).all(|tag| tag.index as usize >= imported_tags));
    }
}

#[test]
fn exported_function_names_cannot_multiply_a_small_name_section() {
    let mut module = shared_type(0, 1, 0, 0, 0);
    let name = "λ".repeat(32_768);
    let mut exports = leb(1280);
    for index in 0..1280 {
        string(&mut exports, &format!("export_{index}"));
        exports.extend([0, 0]);
    }
    section(&mut module, 7, &exports);
    let mut map = vec![1, 0];
    string(&mut map, &name);
    let mut names = vec![4, b'n', b'a', b'm', b'e', 1];
    names.extend(leb(map.len()));
    names.extend(map);
    section(&mut module, 0, &names);
    let report = bounded(&module);
    assert!(report.validation.valid, "{:?}", report.validation);
    assert_eq!(report.exports.len(), 1280);
    assert!(report.exports.iter().any(|export| export.target_name.is_none()));
    assert!(report.exports.iter().filter_map(|export| export.target_name.as_deref()).all(|value| value == name));
}

#[test]
fn ordinary_shared_signatures_and_names_remain_complete() {
    let report = inspect(&shared_type(2, 2, 2, 2, 3)).unwrap();
    assert!(report.validation.valid, "{:?}", report.validation);
    assert!(!report.truncated, "{:?}", report.warnings);
    assert_eq!(report.imported_functions, 2);
    assert_eq!(report.functions.len(), 2);
    assert_eq!(report.tags.len(), 4);
    assert!(report.imports.iter().all(|item| item.signature == "(i32, i32, i32) -> ()"));
    assert!(report.tags.iter().all(|tag| tag.signature == "(i32, i32, i32) -> ()"));
    assert_eq!(report.functions.iter().map(|function| function.index).collect::<Vec<_>>(), [2, 3]);
    assert_eq!(report.tags.iter().map(|tag| tag.index).collect::<Vec<_>>(), [0, 1, 2, 3]);
    assert!(report.tags[..2].iter().all(|tag| tag.imported));
    assert!(report.tags[2..].iter().all(|tag| !tag.imported));
}
