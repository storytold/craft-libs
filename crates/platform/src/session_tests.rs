//! Reopen last session.

use crate::session::*;

fn doc(path: &str) -> SessionDocument {
    SessionDocument { path: path.to_string(), ..Default::default() }
}

#[test]
// After quit_started, closing documents doesn't change to_save(true, ..); quit_cancelled makes to_save return the open session again.
fn quit_keeps_the_session_from_its_start() {
    let two = Session { documents: vec![doc("/a"), doc("/b")], active: Some("/a".into()) };
    let one = Session { documents: vec![doc("/b")], active: None };
    let mut keeper = SessionKeeper::default();
    keeper.quit_started(|| two.clone());
    keeper.quit_started(|| one.clone());
    assert_eq!(keeper.to_save(true, one.clone()), two);
    assert_eq!(keeper.to_save(false, one.clone()), one);
    keeper.quit_cancelled();
    assert_eq!(keeper.to_save(true, one.clone()), one);
}

// The stored session is untrusted settings: entries without a usable path and non-text fields
// are skipped rather than failing the whole session, a missing view is `Null`, a huge list is
// cut at MAX_DOCUMENTS, and anything that isn't a session reads as none.
#[test]
fn malformed_entries_are_skipped_and_the_list_capped() {
    let value = serde_json::json!({
        "documents": [{"path": 5}, {"path": ""}, "x", {"path": "/a.pdf", "window": 3}, {"path": "/b.pdf", "view": {"page": 7}}],
        "active": 3,
    });
    let session = Session::from_json(&value);
    let read: Vec<(&str, Option<&str>, &serde_json::Value)> =
        session.documents.iter().map(|d| (d.path.as_str(), d.window.as_deref(), &d.view)).collect();
    assert_eq!(read, [("/a.pdf", None, &serde_json::Value::Null), ("/b.pdf", None, &serde_json::json!({"page": 7}))]);
    assert_eq!(session.active, None);
    let many: Vec<serde_json::Value> = (0..1000).map(|i| serde_json::json!({"path": format!("/{i}.pdf")})).collect();
    assert_eq!(Session::from_json(&serde_json::json!({"documents": many})).documents.len(), MAX_DOCUMENTS);
    assert_eq!(Session::from_json(&serde_json::json!("garbage")), Session::default());
}

// TOML has no null, so a document without view state must not write one: the typed settings save
// would fail and lose the whole session. Left out, it reads back as `Null` both ways.
#[test]
fn a_null_view_is_left_out_and_reads_back_as_null() {
    let session = Session { documents: vec![doc("/a.pdf"), SessionDocument { view: serde_json::json!({"page": 2}), ..doc("/b.pdf") }], active: None };
    let written = serde_json::to_value(&session).unwrap();
    assert_eq!(written["documents"][0].get("view"), None, "{written}");
    assert_eq!(written["documents"][1]["view"], serde_json::json!({"page": 2}));
    assert_eq!(serde_json::from_value::<Session>(written.clone()).unwrap(), session);
    assert_eq!(Session::from_json(&written), session);
}
