#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use serde::Deserialize;
use serde_json::json;

use super::*;

/// A stand-in for an app's workbook: fields in declaration order, one of them a list.
#[derive(Debug, PartialEq, Serialize, Deserialize)]
struct Book {
    sheets: Vec<String>,
    active: u32,
    #[serde(default)]
    macros: Vec<Value>,
}

fn book() -> Book {
    Book { sheets: vec!["Sheet1".into(), "Été ✓".into()], active: 1, macros: vec![json!({"v": 1, "name": "Fill"})] }
}

#[test]
fn extensions_are_json_and_scjson_in_any_case() {
    for yes in ["json", "scjson", "JSON", "ScJson"] {
        assert!(is_extension(yes), "{yes}");
    }
    for no in ["", "xlsx", "jsonl", ".json", "scjs"] {
        assert!(!is_extension(no), "{no}");
    }
}

#[test]
fn a_workbook_is_written_pretty_in_field_order() {
    let bytes = write(&book()).unwrap();
    let text = String::from_utf8(bytes).unwrap();
    assert_eq!(
        text,
        "{\n  \"sheets\": [\n    \"Sheet1\",\n    \"Été ✓\"\n  ],\n  \"active\": 1,\n  \"macros\": [\n    {\n      \"name\": \"Fill\",\n      \"v\": 1\n    }\n  ]\n}"
    );
    assert_eq!(read::<Book>(text.as_bytes()).unwrap(), book());
}

#[test]
fn kept_entries_are_appended_as_the_file_held_them() {
    let kept = [json!({"v": 9, "name": "Future", "extra": [1, 2]})];
    let bytes = write_with_kept(&book(), "macros", &kept).unwrap();
    let back: Book = read(&bytes).unwrap();
    assert_eq!(back.macros, [json!({"v": 1, "name": "Fill"}), kept[0].clone()]);
    // Through a JSON value, keys are in map order: `active` before `macros` before `sheets`.
    let text = String::from_utf8(bytes).unwrap();
    assert!(text.find("\"active\"") < text.find("\"macros\"") && text.find("\"macros\"") < text.find("\"sheets\""), "{text}");
    // With nothing kept, the plain writer's bytes.
    assert_eq!(write_with_kept(&book(), "macros", &[]).unwrap(), write(&book()).unwrap());
}

#[test]
fn kept_entries_make_the_list_when_it_is_absent() {
    #[derive(Serialize)]
    struct Bare {
        sheets: Vec<String>,
    }
    let bytes = write_with_kept(&Bare { sheets: vec![] }, "macros", &[json!(1)]).unwrap();
    assert_eq!(serde_json::from_slice::<Value>(&bytes).unwrap(), json!({"sheets": [], "macros": [1]}));
    // A field that is not a list is left as it is.
    let bytes = write_with_kept(&json!({"macros": "text"}), "macros", &[json!(1)]).unwrap();
    assert_eq!(serde_json::from_slice::<Value>(&bytes).unwrap(), json!({"macros": "text"}));
}

#[test]
fn what_is_not_a_workbook_says_so() {
    for bad in [&b""[..], b"{", b"[]", b"{\"sheets\": 7, \"active\": 0}", b"\xff\xfe"] {
        let err = read::<Book>(bad).unwrap_err();
        assert!(matches!(err, Error::Read(_)));
        assert!(err.to_string().starts_with("not a GridCraft JSON workbook: "), "{err}");
    }
    assert_eq!(
        read::<Book>(b"{\"sheets\": 7}").unwrap_err().to_string(),
        "not a GridCraft JSON workbook: invalid type: integer `7`, expected a sequence at line 1 column 12"
    );
}

#[test]
fn a_value_json_cannot_hold_is_a_write_error() {
    use std::collections::BTreeMap;
    let bad: BTreeMap<Vec<u8>, u8> = [(vec![1], 1)].into();
    assert!(matches!(write(&bad), Err(Error::Write(_))));
    assert!(matches!(write_with_kept(&bad, "macros", &[json!(1)]), Err(Error::Write(_))));
}
