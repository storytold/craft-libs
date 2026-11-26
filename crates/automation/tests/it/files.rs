//! `ui.pick` at the protocol boundary.

use std::path::PathBuf;

use craft_automation::files::pick_method;
use craft_host::{Files, NoPickFiles, PickRequest, Picked, ScriptedFiles};
use craft_ui::testing::trial;
use serde_json::{Value, json};

fn call(files: &dyn Files, params: Value) -> Result<Value, String> {
    pick_method(files, "ui.pick", &params).expect("ui.pick is the pick method")
}

fn path(p: &str) -> PathBuf {
    PathBuf::from(p)
}

/// Each field reaches the service as its `Picked`; with picks on screen the oldest is answered; a bad request is
/// refused with the field list and answers nothing; a service that takes no automated answers says so.
#[trial]
fn ui_pick_answers_the_next_picker_and_refuses_bad_params() {
    let answered = Ok(json!({"answered": "next"}));
    let files = ScriptedFiles::new();
    assert_eq!(call(&files, json!({"files": ["/w/a.pdf", "/w/b.pdf"]})), answered);
    assert_eq!(call(&files, json!({"folder": "/w/out"})), answered);
    assert_eq!(call(&files, json!({"save": "/w/c.pdf"})), answered);
    assert_eq!(call(&files, json!({"cancel": true, "files": null})), answered, "a null field is omitted");
    let picked: Vec<Picked> = (0..4)
        .map(|_| {
            let id = files.pick(PickRequest::open("document"));
            files.take_answer(id).expect("a queued answer comes at once")
        })
        .collect();
    assert_eq!(
        picked,
        [
            Picked::Paths(vec![path("/w/a.pdf"), path("/w/b.pdf")]),
            Picked::Paths(vec![path("/w/out")]),
            Picked::Paths(vec![path("/w/c.pdf")]),
            Picked::Cancelled
        ]
    );

    let held = ScriptedFiles::held();
    let oldest = held.pick(PickRequest::open("first"));
    let newest = held.pick(PickRequest::open("second"));
    assert_eq!(call(&held, json!({"files": ["/w/first.pdf"]})), answered);
    assert_eq!(held.take_answer(oldest), Some(Picked::Paths(vec![path("/w/first.pdf")])), "the oldest open pick is answered");
    assert_eq!(held.take_answer(newest), None);
    assert_eq!(held.open(), 1);

    for bad in [
        json!({"files": ["/w/a.pdf"], "save": "/w/b.pdf"}),
        json!({}),
        json!({"files": []}),
        json!({"files": ["/w/a.pdf", 3]}),
        json!({"save": 3}),
        json!({"cancel": false}),
        json!({"path": "/w/a.pdf"}),
    ] {
        let error = call(&files, bad.clone()).expect_err("refused");
        assert!(error.contains("files, folder, save, cancel"), "{bad}: {error}");
    }
    assert_eq!(call(&files, json!({"folder": "/w/after"})), answered);
    let id = files.pick(PickRequest::folder("export"));
    assert_eq!(files.take_answer(id), Some(Picked::Paths(vec![path("/w/after")])), "no refused request queued an answer");

    let unsupported = call(&NoPickFiles::default(), json!({"cancel": true})).expect_err("a service without automated answers refuses");
    assert!(!unsupported.contains("files, folder, save, cancel"), "an unsupported service is not a field-list refusal: {unsupported}");
    assert!(pick_method(&files, "ui.click", &json!({})).is_none());
}
