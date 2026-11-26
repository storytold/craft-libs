//! Integration tests of craft-ecproj-format: envelope.

use std::collections::BTreeMap;

use craft_ecproj_format::{File, Head, ReadError};
use serde::{Deserialize, Serialize};

#[derive(Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
struct View {
    #[serde(skip_serializing_if = "Option::is_none")]
    active_comp: Option<u64>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    open_comps: Vec<u64>,
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    comp_times: BTreeMap<u64, i64>,
}

#[derive(Debug, PartialEq, Serialize, Deserialize)]
struct Settings {
    bit_depth: String,
}

#[derive(Debug, PartialEq, Serialize, Deserialize)]
struct Item {
    id: u64,
    name: String,
}

#[derive(Debug, PartialEq, Serialize, Deserialize)]
struct Project {
    schema: u32,
    settings: Settings,
    items: BTreeMap<u64, Item>,
    next_id: u64,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    render_queue: Vec<u64>,
}

fn project() -> Project {
    Project {
        schema: 2,
        settings: Settings { bit_depth: "Bpc8".into() },
        items: BTreeMap::from([(1, Item { id: 1, name: "Comp 1".into() }), (2, Item { id: 2, name: "clip.mov".into() })]),
        next_id: 3,
        render_queue: vec![],
    }
}

#[test]
fn a_written_file_reads_back_as_the_project_and_its_head() {
    let p = project();
    let view = View { active_comp: Some(1), ..View::default() };
    let text = File::new("0.12.0", &view, &p).to_json().unwrap();
    assert_eq!(serde_json::from_str::<Project>(&text).unwrap(), p);
    assert_eq!(Head::read(&text).unwrap(), Head { saved_by: Some("0.12.0".into()), schema: 2 });
}

#[test]
fn a_head_needs_a_schema() {
    assert!(Head::read(r#"{"savedBy": "0.12.0", "items": {}}"#).is_err());
    assert!(Head::read(r#"{"savedBy": "0.12.0", "schema": "2"}"#).is_err());
    assert!(Head::read(r#"{"savedBy": "0.13.0", "schema": null, "schema": "3"}"#).is_err());
    assert!(Head::read("[]").is_err());
    assert!(Head::read(r#"["0.12.0", 2]"#).is_err());
    assert!(Head::read(r#"{"schema": 2, "schema": 2}"#).is_err());
    assert!(Head::read(r#"{"schema": 2} {}"#).is_err());
    assert!(Head::read(r#"{"savedBy": "0.12.0", "schema": 2"#).is_err());
    assert!(Head::read("").is_err());
    assert!(Head::read("2").is_err());
}

#[test]
fn a_head_refuses_a_newer_schema() {
    let Err(ReadError::TooNew { too_new, saved_by, .. }) = Head::read(r#"{"savedBy": "0.13.0", "schema": 3}"#) else {
        panic!("schema 3 is newer than 2")
    };
    assert_eq!((too_new.schema, too_new.supported), (3, 2));
    assert_eq!(saved_by.as_deref(), Some("0.13.0"));
    let Err(e) = Head::read(r#"{"schema": 4294967295, "savedBy": 1}"#) else { panic!("schema u32::MAX is newer than 2") };
    assert_eq!(e.to_string(), "project schema 4294967295 is newer than this version supports (2)");
    let ReadError::TooNew { saved_by, .. } = e else { panic!("{e:?} is not TooNew") };
    assert_eq!(saved_by, None);
    assert_eq!(Head::read(r#"{"savedBy": "0.12.0", "schema": 2}"#).unwrap(), Head { saved_by: Some("0.12.0".into()), schema: 2 });
}

#[test]
fn a_deeply_nested_saved_by_is_skipped_like_other_members() {
    let deep = format!("{}{}", "[".repeat(10_000), "]".repeat(10_000));
    let text = format!(r#"{{"savedBy": {deep}, "schema": 2, "items": {deep}}}"#);
    assert_eq!(Head::read(&text).unwrap(), Head { saved_by: None, schema: 2 });
    let deep_map = format!("{}0{}", r#"{"v": "#.repeat(10_000), "}".repeat(10_000));
    let text = format!(r#"{{"savedBy": "0.12.0", "savedBy": {deep_map}, "schema": 2}}"#);
    assert_eq!(Head::read(&text).unwrap(), Head { saved_by: None, schema: 2 });
}
