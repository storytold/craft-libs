use super::*;

fn newer() -> Newer {
    Newer { format: 9, reads_up_to: 8, unknown_changes: Vec::new() }
}

/// What typed reading dropped is named and goes back on saving, where it was.
#[test]
fn dropped_fields_are_named_and_put_back() {
    let file = json!({"name": "A", "format": "app/9", "items": [{"id": 1, "size": 2, "colour": "red"}], "extra": {"x": 1}});
    let known = json!({"name": "A", "items": [{"id": 1, "size": 2}]});
    let mut said = NewerFile::new("App", &newer());
    said.compare(&file, &known, "placeholder", &[]);
    assert_eq!(said.not_understood, ["field `extra`", "field `items[1].colour`"]);
    let mut saved = json!({"name": "A", "items": [{"id": 1, "size": 3}]});
    assert!(said.restore_into(&mut saved).is_empty());
    assert_eq!(saved, json!({"name": "A", "items": [{"id": 1, "size": 3, "colour": "red"}], "extra": {"x": 1}}));
    assert_eq!(said.notice(), "Saved by a newer App (format 9, this version reads up to 8). If something's missing, update.");
}

/// A section that can't be read is left out, kept, and goes back while it is unchanged here.
#[test]
fn unreadable_sections_are_left_out_and_kept() {
    let mut top = match json!({"name": "A", "good": 1, "bad": 2}) {
        Value::Object(m) => m,
        _ => unreachable!(),
    };
    let mut said = NewerFile::new("App", &newer());
    leave_out_unreadable(&mut top, &mut said, "design", |key: &str, _: &Value| if key == "bad" { Err("design file: no") } else { Ok(()) });
    assert!(!top.contains_key("bad"));
    assert_eq!(said.not_understood, ["section `bad` (no)"]);
    let read =
        read_around(
            &mut top,
            &mut said,
            &["name"],
            "design",
            |top: &Map<String, Value>| {
                if top.contains_key("good") { Err("design file: clash") } else { Ok(top.len()) }
            },
        );
    assert_eq!(read, Ok(1));
    assert_eq!(said.not_understood, ["section `bad` (no)", "section `good` (clash)"]);
    let mut saved = json!({"name": "A"});
    said.compare(&saved.clone(), &saved.clone(), "placeholder", &[]);
    assert!(said.restore_into(&mut saved).is_empty());
    assert_eq!(saved, json!({"name": "A", "good": 1, "bad": 2}));
}

/// Notices, warnings and reports name what was left out, and kept values can be searched.
#[test]
fn warnings_report_and_mentions_cover_everything_kept() {
    let mut said = NewerFile::new("App", &Newer { format: 8, reads_up_to: 8, unknown_changes: vec!["curves".into()] });
    assert_eq!(said.notice(), "Saved by a newer App (format 8 with `curves`, which this version doesn't know). If something's missing, update.");
    assert!(!said.needs_container());
    said.keep_section("plugins", json!({"tool": "ruler"}), Some(json!({})), "unknown");
    said.keep_element("shapes", json!({"name": "star", "points": ["tip"]}), "shape `star`".into());
    let file = json!({"shapes": [{"name": "a", "tags": ["deep"], "more": {"x": "nested"}}], "n": 1});
    said.compare(&file, &json!({"shapes": [{"name": "a"}], "n": 1}), "placeholder", &[]);
    assert_eq!(said.warnings()[0], said.notice());
    assert!(said.warnings().contains(&"Not read: section `plugins` (unknown). Saving keeps it.".to_string()));
    let report = said.report();
    assert_eq!(report["format"], 8);
    assert_eq!(report["unknown_changes"], json!(["curves"]));
    assert_eq!(report["notice"], json!(said.notice()));
    for text in ["ruler", "tip", "deep", "nested"] {
        assert!(said.mentions(text), "{text}");
    }
    assert!(!said.mentions("absent"));
    said.parts.push(("blob".into(), Arc::from(&b"x"[..])));
    assert!(said.needs_container());
}

/// A kept section or element that was replaced here is reported lost, not written over.
#[test]
fn replaced_sections_and_elements_are_reported_lost() {
    let mut said = NewerFile::new("App", &newer());
    said.keep_section("plugins", json!({"tool": "ruler"}), None, "unknown");
    said.keep_element("shapes", json!({"name": "star"}), "shape `star`".into());
    said.keep_element("shapes", json!({"points": 3}), "an unnamed shape".into());
    let mut saved = json!({"plugins": {"edited": true}, "shapes": [{"name": "star"}]});
    let lost = said.restore_into(&mut saved);
    assert_eq!(lost, ["section `plugins`, replaced by what was edited here", "`shapes` element star, replaced by what was made here"]);
    assert_eq!(saved["shapes"], json!([{"name": "star"}, {"points": 3}]));
    let mut fresh = json!({});
    said.sections.clear();
    assert!(said.restore_into(&mut fresh).is_empty());
    assert_eq!(fresh["shapes"], json!([{"name": "star"}, {"points": 3}]));
}

/// Retained container metadata alone needs a container to survive the save.
#[test]
fn container_metadata_alone_needs_a_container() {
    let mut said = NewerFile::new("App", &newer());
    assert!(!said.needs_container());
    said.container = Some(json!({"layout": 2}));
    assert!(said.needs_container());
}

/// An edit that replaces a retained field keeps the edit and reports the replaced value.
#[test]
fn a_replaced_retained_field_is_reported() {
    let mut said = NewerFile::new("App", &newer());
    said.compare(&json!({"extra": 1}), &json!({}), "placeholder", &[]);
    let mut edited = json!({"extra": 2});
    assert_eq!(said.restore_into(&mut edited), ["field `extra`"]);
    assert_eq!(edited, json!({"extra": 2}));
    let mut same = json!({"extra": 1});
    assert!(said.restore_into(&mut same).is_empty());
}

/// Records are matched by id and by name across long lists, and an index anchor only while the
/// list keeps its length.
#[test]
fn records_match_by_id_name_or_unchanged_index() {
    let records: Vec<Value> = (0..2000).map(|i| json!({"id": i, "extra": i})).collect();
    let known: Vec<Value> = (0..2000).map(|i| json!({"id": i})).collect();
    let mut said = NewerFile::new("App", &newer());
    said.compare(&json!({"items": records}), &json!({"items": known}), "placeholder", &[]);
    let mut saved = json!({"items": (0..2000).rev().map(|i| json!({"id": i})).collect::<Vec<_>>()});
    assert!(said.restore_into(&mut saved).is_empty());
    assert_eq!(saved["items"][0], json!({"id": 1999, "extra": 1999}));
    let mut by_name = NewerFile::new("App", &newer());
    by_name.compare(&json!({"l": [{"name": "a", "x": 1}, [1, 2]]}), &json!({"l": [{"name": "a"}, [1]]}), "placeholder", &[]);
    let mut renamed = json!({"l": [{"name": "a"}]});
    by_name.restore_into(&mut renamed);
    assert_eq!(renamed["l"][0], json!({"name": "a", "x": 1}));
}

/// A section kept whole goes back whole, even when this build's record of it has fields.
#[test]
fn a_section_kept_whole_is_not_compared_field_by_field() {
    let mut said = NewerFile::new("App", &newer());
    said.keep_section("plugins", json!({"mode": "future", "payload": 1}), None, "unreadable");
    let file = json!({"name": "A", "plugins": {"mode": "future", "payload": 1}});
    said.compare(&file, &json!({"name": "A", "plugins": {"mode": "none"}}), "placeholder", &[]);
    let mut saved = json!({"name": "A", "plugins": {"mode": "none"}});
    assert!(said.restore_into(&mut saved).is_empty());
    assert_eq!(saved, file);
}

/// A kept element goes back only while no record has its id or name.
#[test]
fn a_kept_element_keeps_record_identity() {
    let mut said = NewerFile::new("App", &newer());
    said.keep_element("items", json!({"id": 7, "kind": "future"}), "element 7".into());
    said.keep_element("items", json!({"kind": "anonymous"}), "unnamed".into());
    let mut clash = json!({"items": [{"id": 7, "kind": "made here"}]});
    assert_eq!(said.restore_into(&mut clash).len(), 1);
    assert_eq!(clash, json!({"items": [{"id": 7, "kind": "made here"}, {"kind": "anonymous"}]}));
    let mut free = json!({"items": []});
    assert!(said.restore_into(&mut free).is_empty());
    assert_eq!(free, json!({"items": [{"id": 7, "kind": "future"}, {"kind": "anonymous"}]}));
}

/// Container-only fields are left out at the top level only.
#[test]
fn container_only_fields_are_top_level() {
    let mut said = NewerFile::new("App", &newer());
    let file = json!({"thumbnail": "t", "layer": {"thumbnail": "nested"}});
    said.compare(&file, &json!({"layer": {}}), "placeholder", &["thumbnail"]);
    assert_eq!(said.not_understood, ["field `layer.thumbnail`"]);
    let mut saved = json!({"layer": {}});
    assert!(said.restore_into(&mut saved).is_empty());
    assert_eq!(saved, json!({"layer": {"thumbnail": "nested"}}));
}

/// An element kept on its own is not kept again with its list when typed writing leaves the list out.
#[test]
fn a_kept_element_is_not_restored_twice() {
    let mut said = NewerFile::new("App", &newer());
    said.keep_element("items", json!({"kind": "future"}), "unnamed".into());
    said.compare(&json!({"items": [{"kind": "future"}]}), &json!({}), "placeholder", &[]);
    let mut saved = json!({});
    assert!(said.restore_into(&mut saved).is_empty());
    assert_eq!(saved, json!({"items": [{"kind": "future"}]}));
}

/// A record whose id typed reading drops is matched by its name, and its fields go back.
#[test]
fn a_dropped_id_falls_back_to_the_name() {
    let mut said = NewerFile::new("App", &newer());
    said.compare(&json!({"items": [{"id": 7, "name": "a", "extra": 1}]}), &json!({"items": [{"name": "a"}]}), "placeholder", &[]);
    let mut saved = json!({"items": [{"name": "a"}]});
    assert!(said.restore_into(&mut saved).is_empty());
    assert_eq!(saved, json!({"items": [{"name": "a", "id": 7, "extra": 1}]}));
}

/// Kept elements sharing a list are checked against one index, the ones put back included.
#[test]
fn kept_elements_check_each_other() {
    let mut said = NewerFile::new("App", &newer());
    said.keep_element("items", json!({"id": 1, "v": "first"}), "1".into());
    said.keep_element("items", json!({"id": 1, "v": "again"}), "1 again".into());
    let mut saved = json!({"items": []});
    assert_eq!(said.restore_into(&mut saved).len(), 1);
    assert_eq!(saved, json!({"items": [{"id": 1, "v": "first"}]}));
    // deep nesting is searched without recursion
    let mut deep = json!("needle");
    for _ in 0..20_000 {
        deep = Value::Array(vec![deep]);
    }
    said.keep_section("deep", deep, None, "test");
    assert!(said.mentions("needle"));
    // serde_json drops deep values recursively; leak it rather than overflow the test thread
    std::mem::forget(said);
}

/// A record whose whole identity typed reading drops is matched by its place while the list keeps
/// its length, so an unchanged save keeps its fields.
#[test]
fn a_record_without_a_surviving_identity_is_matched_by_place() {
    let mut said = NewerFile::new("App", &newer());
    said.compare(&json!({"items": [{"id": 7, "extra": 1}]}), &json!({"items": [{}]}), "placeholder", &[]);
    let mut saved = json!({"items": [{}]});
    assert!(said.restore_into(&mut saved).is_empty());
    assert_eq!(saved, json!({"items": [{"id": 7, "extra": 1}]}));
}
