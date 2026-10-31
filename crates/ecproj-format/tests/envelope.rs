use std::collections::BTreeMap;

use craft_ecproj_format::{File, Head, SCHEMA_VERSION, TooNew, check_schema, saved_by};
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
fn a_written_file_reads_back_as_the_project_and_its_head() {
    let p = project();
    let view = View { active_comp: Some(1), ..View::default() };
    let text = File::new("0.12.0", &view, &p).to_json().unwrap();
    assert_eq!(serde_json::from_str::<Project>(&text).unwrap(), p);
    assert_eq!(Head::read(&text).unwrap(), Head { saved_by: Some("0.12.0".into()), schema: 2 });
    assert_eq!(saved_by(&text).as_deref(), Some("0.12.0"));
}

#[test]
fn a_project_saved_before_saved_by_has_a_head_without_it() {
    let text = r#"{"schema": 1, "settings": {}, "items": {}, "next_id": 1}"#;
    assert_eq!(Head::read(text).unwrap(), Head { saved_by: None, schema: 1 });
    assert_eq!(saved_by(text), None);
}

#[test]
fn a_head_needs_a_schema() {
    assert!(Head::read(r#"{"savedBy": "0.12.0", "items": {}}"#).is_err());
    assert!(Head::read(r#"{"savedBy": "0.12.0", "schema": "2"}"#).is_err());
    assert!(Head::read("[2]").is_err());
    assert!(Head::read(r#"["0.12.0", 2]"#).is_err());
    assert!(Head::read(r#"{"schema": 2, "schema": 2}"#).is_err());
    assert!(Head::read(r#"{"savedBy": "a", "savedBy": "b", "schema": 2}"#).is_err());
    assert!(Head::read(r#"{"schema": 2} {}"#).is_err());
}

#[test]
fn a_head_skips_other_members_of_any_type() {
    let text = r#"{"items": [1, {"schema": 9}], "savedBy": null, "schema": 2, "next_id": 1.5}"#;
    assert_eq!(Head::read(text).unwrap(), Head { saved_by: None, schema: 2 });
}

#[test]
fn saved_by_is_none_unless_it_is_a_string_in_a_json_object() {
    assert_eq!(saved_by(r#"{"savedBy": 12, "schema": 2}"#), None);
    assert_eq!(saved_by(r#"{"savedBy": null, "schema": 2}"#), None);
    assert_eq!(saved_by(r#"["savedBy", "0.12.0"]"#), None);
    assert_eq!(saved_by(r#"["0.12.0"]"#), None);
    assert_eq!(saved_by(r#"{"savedBy": "0.12.0"} []"#), None);
    assert_eq!(saved_by(r#"{"savedBy": "0.12.0", "schema": 2"#), None);
    assert_eq!(saved_by(""), None);
    assert_eq!(saved_by(r#"{"schema": 2, "savedBy": "0.13.0-dev"}"#).as_deref(), Some("0.13.0-dev"));
    assert_eq!(saved_by(r#"{"savedBy": "0.13.0", "schema": null, "schema": "3"}"#).as_deref(), Some("0.13.0"));
}

#[test]
fn check_schema_refuses_only_a_newer_schema() {
    assert_eq!(SCHEMA_VERSION, 2);
    assert_eq!(check_schema(0), Ok(()));
    assert_eq!(check_schema(1), Ok(()));
    assert_eq!(check_schema(2), Ok(()));
    assert_eq!(check_schema(3), Err(TooNew { schema: 3, supported: 2 }));
    assert_eq!(check_schema(u32::MAX), Err(TooNew { schema: u32::MAX, supported: 2 }));
}

#[test]
fn too_new_reads_as_effectcraft_words_it() {
    let Err(e) = check_schema(3) else { panic!("schema 3 is newer than 2") };
    assert_eq!(e.to_string(), "project schema 3 is newer than this version supports (2)");
}
