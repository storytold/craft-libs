//! Original, generated project fragments exercise record ownership and preservation.
use craft_fcproj_format::{FormatError, MIGRATIONS, SCHEMA_VERSION, migrate_with, records::ProjectRecords};
use serde_json::{Value, json};

fn project() -> Value {
    json!({
        "format": "filmcraft.project", "schema_version": SCHEMA_VERSION, "view": {"tabs":[12]},
        "generator": "test", "extra": [null, {}],
        "project": {
            "root": {"id":0,"name":"Cut","children":[
                {"Item":12},{"Bin":{"id":19,"name":"Cut","children":[{"Item":4}]}}
            ]},
            "items": {
                "4":{"id":4,"media":{"path":"../offline/shot.mov"},"label":"A"},
                "12":{"id":12,"sequence":{"nested":[4],"duration":123}},
                "9":{"id":9,"name":"Unfiled","extra":{}}
            },
            "next_id":20, "transcripts":{}, "new_project_field":{"value":null}
        }
    })
}

fn split(doc: &Value) -> ProjectRecords {
    ProjectRecords::split(&serde_json::to_vec(doc).unwrap()).unwrap()
}

#[test]
fn changing_one_sequence_preserves_all_other_records_and_the_view() {
    let original = project();
    let mut changed = original.clone();
    changed["project"]["items"]["12"]["sequence"]["duration"] = json!(124);
    let before = split(&original);
    let after = split(&changed);
    assert_eq!(before.envelope, after.envelope);
    assert_eq!(before.bins, after.bins);
    assert_eq!(before.items.keys().filter(|id| before.items[id] != after.items[id]).copied().collect::<Vec<_>>(), [12]);
    assert_eq!(before.into_document().unwrap(), original);
    assert_eq!(after.into_document().unwrap(), changed);
}

#[test]
fn legacy_uses_the_same_migrations_and_future_versions_are_refused() {
    let mut bare = project()["project"].take();
    bare["version"] = json!(1);
    assert_eq!(split(&bare).into_document().unwrap(), migrate_with(MIGRATIONS, bare, 1).unwrap());
    let mut doc = project();
    doc["schema_version"] = json!(SCHEMA_VERSION + 1);
    assert!(matches!(ProjectRecords::split(&serde_json::to_vec(&doc).unwrap()), Err(FormatError::TooNew { .. })));
}

#[test]
fn incomplete_or_conflicting_records_cannot_be_reassembled() {
    let make = || split(&project());
    let mut missing = make();
    missing.items.remove(&9); // Unfiled; a bin traversal alone cannot detect the lost item.
    assert!(missing.into_document().is_err());
    let mut extra = make();
    extra.items.insert(11, json!({"id":11}));
    assert!(extra.into_document().is_err());
    let mut cycle = make();
    cycle.bins.get_mut(&19).unwrap()["children"] = json!([{"Bin":0}]);
    assert!(cycle.into_document().is_err());
    let mut mismatch = make();
    mismatch.items.get_mut(&4).unwrap()["id"] = json!(19);
    assert!(mismatch.into_document().is_err());
    let mut detached = make();
    detached.bins.get_mut(&0).unwrap()["children"] = json!([]);
    assert!(detached.into_document().is_err());
}

#[test]
fn conversion_refuses_ambiguous_bins_and_dangling_membership() {
    let mut doc = project();
    doc["project"]["root"]["children"][1]["Bin"]["id"] = json!(0);
    assert!(ProjectRecords::split(&serde_json::to_vec(&doc).unwrap()).is_err());
    doc["project"]["root"]["children"] = json!([{"Item":100}]);
    assert!(ProjectRecords::split(&serde_json::to_vec(&doc).unwrap()).is_err());
}
