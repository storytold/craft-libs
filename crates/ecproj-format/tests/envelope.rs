use std::collections::BTreeMap;

use craft_ecproj_format::{File, Head, ReadError, SCHEMA_VERSION, check_schema};
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

const PROJECT_MEMBERS: &str = r#"  "schema": 2,
  "settings": {
    "bit_depth": "Bpc8"
  },
  "items": {
    "1": {
      "id": 1,
      "name": "Comp 1"
    },
    "2": {
      "id": 2,
      "name": "clip.mov"
    }
  },
  "next_id": 3
}"#;

#[test]
fn a_file_with_a_saved_view_holds_saved_by_then_the_view_then_the_project() {
    let view = View { active_comp: Some(1), open_comps: vec![1, 2], comp_times: BTreeMap::from([(1, 254_016_000_000)]) };
    let p = project();
    let text = File::new("0.12.0", &view, &p).to_json().unwrap();
    let expected = format!(
        r#"{{
  "savedBy": "0.12.0",
  "activeComp": 1,
  "openComps": [
    1,
    2
  ],
  "compTimes": {{
    "1": 254016000000
  }},
{PROJECT_MEMBERS}"#
    );
    assert_eq!(text, expected);
}

#[test]
fn a_file_with_an_empty_view_holds_saved_by_then_the_project() {
    let p = project();
    let text = File::new("0.12.0", &View::default(), &p).to_json().unwrap();
    assert_eq!(text, format!("{{\n  \"savedBy\": \"0.12.0\",\n{PROJECT_MEMBERS}"));
}

#[test]
fn a_compact_file_holds_the_same_members_on_one_line() {
    let view = View { active_comp: Some(1), open_comps: vec![1, 2], comp_times: BTreeMap::from([(1, 254_016_000_000)]) };
    let p = project();
    let text = File::new("0.12.0", &view, &p).to_json_compact().unwrap();
    assert_eq!(
        text,
        r#"{"savedBy":"0.12.0","activeComp":1,"openComps":[1,2],"compTimes":{"1":254016000000},"schema":2,"settings":{"bit_depth":"Bpc8"},"items":{"1":{"id":1,"name":"Comp 1"},"2":{"id":2,"name":"clip.mov"}},"next_id":3}"#
    );
    assert_eq!(serde_json::from_str::<Project>(&text).unwrap(), p);
    assert_eq!(Head::read(&text).unwrap(), Head { saved_by: Some("0.12.0".into()), schema: 2 });
    let pretty = File::new("0.12.0", &view, &p).to_json().unwrap();
    assert_eq!(serde_json::from_str::<serde_json::Value>(&text).unwrap(), serde_json::from_str::<serde_json::Value>(&pretty).unwrap());
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
fn a_project_saved_before_saved_by_has_a_head_without_it() {
    let text = r#"{"schema": 1, "settings": {}, "items": {}, "next_id": 1}"#;
    assert_eq!(Head::read(text).unwrap(), Head { saved_by: None, schema: 1 });
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
fn a_head_reads_a_saved_by_that_is_not_one_string_as_none() {
    assert_eq!(Head::read(r#"{"savedBy": 12, "schema": 2}"#).unwrap(), Head { saved_by: None, schema: 2 });
    assert_eq!(Head::read(r#"{"savedBy": ["0.12.0"], "schema": 2}"#).unwrap(), Head { saved_by: None, schema: 2 });
    assert_eq!(Head::read(r#"{"savedBy": {"v": "0.12.0"}, "schema": 2}"#).unwrap(), Head { saved_by: None, schema: 2 });
    assert_eq!(Head::read(r#"{"savedBy": "a", "savedBy": "b", "schema": 2}"#).unwrap(), Head { saved_by: None, schema: 2 });
    assert_eq!(Head::read(r#"{"savedBy": "a", "schema": 2, "savedBy": 1, "savedBy": "c"}"#).unwrap(), Head { saved_by: None, schema: 2 });
    assert_eq!(Head::read(r#"{"schema": 2, "savedBy": "0.13.0-dev"}"#).unwrap(), Head { saved_by: Some("0.13.0-dev".into()), schema: 2 });
}

#[test]
fn a_head_reads_the_object_form_only() {
    assert!(matches!(Head::read(r#"[2, {}, {}, 1]"#), Err(ReadError::Json(_))));
    assert!(matches!(Head::read("[1]"), Err(ReadError::Json(_))));
    assert!(matches!(Head::read(r#"["savedBy", "0.12.0"]"#), Err(ReadError::Json(_))));
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
fn a_head_skips_other_members_of_any_type() {
    let text = r#"{"items": [1, {"schema": 9}], "savedBy": null, "schema": 2, "next_id": 1.5}"#;
    assert_eq!(Head::read(text).unwrap(), Head { saved_by: None, schema: 2 });
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

#[test]
fn a_wide_saved_by_array_reads_as_none() {
    let wide = format!("[{}0]", "0,".repeat(1_000_000));
    let text = format!(r#"{{"savedBy": {wide}, "schema": 2, "savedBy": {wide}}}"#);
    assert_eq!(Head::read(&text).unwrap(), Head { saved_by: None, schema: 2 });
    let text = format!(r#"{{"schema": 2, "savedBy": "0.12.0", "savedBy": "{}"}}"#, "9".repeat(1_000_000));
    assert_eq!(Head::read(&text).unwrap(), Head { saved_by: None, schema: 2 });
}

#[test]
fn check_schema_refuses_only_a_newer_schema() {
    assert_eq!(SCHEMA_VERSION, 2);
    assert_eq!(check_schema(0), Ok(()));
    assert_eq!(check_schema(1), Ok(()));
    assert_eq!(check_schema(2), Ok(()));
    for schema in [3, u32::MAX] {
        let Err(e) = check_schema(schema) else { panic!("schema {schema} is newer than 2") };
        assert_eq!((e.schema, e.supported), (schema, 2));
    }
}

#[test]
fn too_new_reads_as_effectcraft_words_it() {
    let Err(e) = check_schema(3) else { panic!("schema 3 is newer than 2") };
    assert_eq!(e.to_string(), "project schema 3 is newer than this version supports (2)");
}
