//! `MissingPluginRef`: what a document keeps for a contribution whose plugin is not installed.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod common;

use common::services::*;
use common::*;
use craft_plugin_host::lifecycle::Decision;
use craft_plugin_host::missing::{MissingPluginRef, RefStatus};
use craft_plugin_host::store::Source;
use serde_json::{Value, json};

const ID: &str = "org.example.t";

fn doc_ref(version: &str, contribution: &str) -> MissingPluginRef {
    serde_json::from_value(json!({
        "plugin_id": ID, "contribution_id": contribution, "plugin_version": version,
        "params": { "bands": 4, "curve": [0.1, 0.9], "nested": { "k": null, "list": [{"a": 1}] } },
        "x_future": { "kept": true }
    }))
    .unwrap()
}

fn install(h: &Harness, version: &str, contribs: Value) {
    h.manager.install(&echo_pkg(ID, version, &echo_wat_v('1'), contribs), Source::File).unwrap();
    h.manager.grant(ID, Decision { accept_required: true, optional: vec![] }).unwrap();
}

#[test]
fn a_reference_survives_a_save_and_load_unchanged_including_fields_this_release_does_not_know() {
    let original = json!({
        "plugin_id": ID, "contribution_id": "echo", "plugin_version": "1.0.0",
        "params": { "z": 1, "a": [1, 2, {"deep": "x"}], "n": null, "f": 0.1 },
        "x_future": { "kept": true }, "another": [1]
    });
    let r: MissingPluginRef = serde_json::from_value(original.clone()).unwrap();
    assert_eq!(r.extra["x_future"], json!({"kept": true}));
    assert_eq!(serde_json::to_value(&r).unwrap(), original);
    // Through text too, which is what a project file is.
    let text = serde_json::to_string(&r).unwrap();
    assert_eq!(serde_json::from_str::<MissingPluginRef>(&text).unwrap(), r);
    // The three named fields and params are required: a record that lacks one is refused, not repaired.
    for key in ["plugin_id", "contribution_id", "plugin_version", "params"] {
        let mut broken = original.clone();
        broken.as_object_mut().unwrap().remove(key);
        assert!(serde_json::from_value::<MissingPluginRef>(broken).is_err(), "missing {key}");
    }
}

#[test]
fn the_status_follows_the_plugin_through_install_disable_update_and_remove() {
    let h = Harness::new();
    let r = doc_ref("1.0.0", "echo");
    let before = serde_json::to_value(&r).unwrap();

    assert_eq!(h.manager.ref_status(&r), RefStatus::MissingPlugin);
    assert_eq!(r.marker(&h.manager.ref_status(&r)).unwrap(), "missing plugin: org.example.t 1.0.0");

    install(&h, "1.0.0", std_contribs());
    let ok = h.manager.ref_status(&r);
    assert_eq!(ok, RefStatus::Available { installed_version: "1.0.0".into(), version_differs: false });
    assert!(!ok.is_missing() && r.marker(&ok).is_none());

    // A document saved by an older plugin version still resolves, and says so.
    let old = doc_ref("0.9.0", "echo");
    assert_eq!(h.manager.ref_status(&old), RefStatus::Available { installed_version: "1.0.0".into(), version_differs: true });

    // A reference to a contribution the plugin does not have.
    let gone = doc_ref("1.0.0", "removed_effect");
    assert_eq!(h.manager.ref_status(&gone), RefStatus::MissingContribution);
    assert!(gone.marker(&h.manager.ref_status(&gone)).unwrap().contains("has no removed_effect"));

    h.manager.disable(ID).unwrap();
    assert_eq!(h.manager.ref_status(&r), RefStatus::MissingPlugin, "a disabled plugin is missing to the document");
    h.manager.enable(ID).unwrap();
    assert!(!h.manager.ref_status(&r).is_missing());

    h.manager.remove(ID).unwrap();
    assert_eq!(h.manager.ref_status(&r), RefStatus::MissingPlugin);
    assert_eq!(serde_json::to_value(&r).unwrap(), before, "no host path rewrote the record");

    // Installing the plugin again brings the same reference back to life with its params intact.
    install(&h, "1.0.0", std_contribs());
    assert!(!h.manager.ref_status(&r).is_missing());
    assert_eq!(r.params["nested"]["list"], json!([{"a": 1}]));
}

#[test]
fn a_contribution_of_a_newer_kind_is_reported_as_needing_a_newer_host() {
    let h = Harness::new();
    let mut c = std_contribs();
    c.as_array_mut().unwrap().push(json!({ "kind": "teleport", "id": "zap", "entry": 99 }));
    install(&h, "1.0.0", c);
    let r = doc_ref("1.0.0", "zap");
    let status = h.manager.ref_status(&r);
    assert_eq!(status, RefStatus::UnsupportedKind { installed_version: "1.0.0".into() });
    assert!(status.is_missing());
    assert_eq!(r.marker(&status).unwrap(), "missing plugin: zap needs a newer host");
}
