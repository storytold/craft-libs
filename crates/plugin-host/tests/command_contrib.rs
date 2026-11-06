//! `command` and `menu` contributions as `DynCommand`s.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod common;

use common::services::*;
use common::*;
use craft_command::{CommandEntry, DynCommand, Enablement, Menu};
use craft_plugin_host::invoke::CallError;
use craft_plugin_host::lifecycle::Decision;
use craft_plugin_host::store::Source;
use serde_json::{Value, json};

const ID: &str = "org.example.t";

fn contribs() -> Value {
    json!([
        { "kind": "command", "id": "echo", "entry": 0, "label": "Echo It", "menu": ["Plug-ins", "Echo"], "shortcut": "Ctrl+E",
          "enabled": "has-selection",
          "params": {
              "gain": { "type": "number", "min": 0, "max": 2, "default": 1, "label": "Gain" },
              "mode": { "type": "choice", "options": ["soft", "hard"], "default": "soft" },
              "count": { "type": "int" },
              "tint": { "type": "color", "default": [0, 0, 0, 1] },
              "where": { "type": "point2" },
              "opaque": { "type": "json" }
          } },
        { "kind": "command", "id": "always", "entry": 0, "enabled": "always" },
        { "kind": "command", "id": "cached", "entry": 0, "enabled": "cached" },
        { "kind": "command", "id": "plain", "entry": 0 },
        { "kind": "menu", "id": "item", "entry": 0, "label": "Menu Item", "menu": ["Plug-ins"] },
        { "kind": "command", "id": "err", "entry": 4 },
        { "kind": "hook", "id": "watch", "entry": 0, "phase": "after" }
    ])
}

fn load(h: &Harness, optional: Vec<craft_plugin_abi::Permission>) {
    let bytes = echo_pkg(ID, "1.0.0", &echo_wat_v('1'), contribs());
    h.manager.install(&bytes, Source::File).unwrap();
    h.manager.grant(ID, Decision { accept_required: true, optional }).unwrap();
}

fn dynamic(h: &Harness, name: &str) -> std::sync::Arc<DynCommand<Sess, AppError>> {
    match h.registry.resolve(&format!("plugin.{ID}.{name}")) {
        Some(CommandEntry::Dynamic(c)) => c,
        other => panic!("{name}: {other:?}"),
    }
}

#[test]
fn a_command_carries_the_manifest_fields_into_the_registry() {
    let h = Harness::new();
    load(&h, vec![craft_plugin_abi::Permission::UiContribute]);
    let c = dynamic(&h, "echo");
    assert_eq!(&*c.id, "plugin.org.example.t.echo");
    assert_eq!(&*c.label, "Echo It");
    assert_eq!(c.shortcut.as_deref(), Some("Ctrl+E"));
    assert_eq!(c.menu.iter().map(|s| &**s).collect::<Vec<_>>(), vec!["Plug-ins", "Echo"]);
    assert_eq!((&*c.owner.plugin_id, c.owner.generation), (ID, 1));
    assert_eq!(c.enablement, Enablement::HasSelection);
    assert!(c.journal);
    assert!(c.visibility.menu && c.visibility.palette && c.visibility.shortcut);
    // Declarative enablement, default, and the cached start value.
    assert_eq!(dynamic(&h, "always").enablement, Enablement::Always);
    assert_eq!(dynamic(&h, "plain").enablement, Enablement::HasDocument, "absent means has-document");
    assert_eq!(dynamic(&h, "cached").enablement, Enablement::Cached(Ok(())));
    // A command without a label shows its id; one without menu or shortcut is palette only.
    let p = dynamic(&h, "plain");
    assert_eq!(&*p.label, "plain");
    assert!(!p.visibility.menu && !p.visibility.shortcut && p.visibility.palette);
    // The hook is not a command.
    assert!(h.registry.resolve(&format!("plugin.{ID}.watch")).is_none());
}

#[test]
fn a_menu_contribution_needs_ui_contribute_and_a_command_loses_menu_and_shortcut_without_it() {
    let with = Harness::new();
    load(&with, vec![craft_plugin_abi::Permission::UiContribute]);
    let item = dynamic(&with, "item");
    assert!(item.visibility.menu && !item.visibility.palette, "a menu item is menu only");

    let without = Harness::new();
    load(&without, vec![]);
    assert!(without.registry.resolve(&format!("plugin.{ID}.item")).is_none(), "a menu item is all a menu contribution is");
    let e = dynamic(&without, "echo");
    assert!(e.menu.is_empty() && e.shortcut.is_none());
    assert!(!e.visibility.menu && !e.visibility.shortcut && e.visibility.palette);
    let mut views = 0;
    without.registry.for_each(|v| {
        if v.id == format!("plugin.{ID}.echo") {
            views += 1;
            assert!(v.shortcut.is_none() && matches!(v.menu, Menu::Shared(m) if m.is_empty()));
        }
    });
    assert_eq!(views, 1);
}

#[test]
fn manifest_params_become_a_json_schema_the_command_layer_keeps() {
    let h = Harness::new();
    load(&h, vec![]);
    let schema = h.registry.schema(&format!("plugin.{ID}.echo")).unwrap();
    assert_eq!(schema["x-craft-source"], "doc", "the doc is a JSON Schema, so it is kept, not guessed");
    let p = &schema["properties"];
    assert_eq!(p["gain"], json!({"type": "number", "minimum": 0.0, "maximum": 2.0, "default": 1, "description": "Gain"}));
    assert_eq!(p["mode"], json!({"type": "string", "enum": ["soft", "hard"], "default": "soft"}));
    assert_eq!(p["count"], json!({"type": "integer"}));
    assert_eq!(p["tint"]["minItems"], 4);
    assert_eq!(p["where"]["maxItems"], 2);
    assert_eq!(p["opaque"], json!({}));
    assert_eq!(schema["required"], json!(["count", "opaque", "where"]), "params without a default are required");
}

#[test]
fn run_sends_params_to_the_guest_and_returns_its_result() {
    let h = Harness::new();
    load(&h, vec![]);
    let out = h.run(&format!("plugin.{ID}.echo"), &json!({"gain": 0.5, "nested": {"a": [1, 2]}})).unwrap();
    assert_eq!(out["params"], json!({"gain": 0.5, "nested": {"a": [1, 2]}}));
    // Null params arrive as null.
    assert_eq!(h.run(&format!("plugin.{ID}.echo"), &Value::Null).unwrap()["params"], Value::Null);
}

#[test]
fn a_plugin_error_reaches_the_app_error_type_and_is_not_a_fault() {
    let h = Harness::new();
    load(&h, vec![]);
    let err = h.run(&format!("plugin.{ID}.err"), &json!({})).unwrap_err();
    assert!(matches!(&err.0, CallError::Plugin(m) if m == "nope"), "{err:?}");
}

#[test]
fn a_command_resolved_before_unload_reports_not_loaded_instead_of_calling_stale_code() {
    let h = Harness::new();
    load(&h, vec![]);
    let held = dynamic(&h, "echo");
    h.manager.disable(ID).unwrap();
    assert!(h.registry.resolve(&format!("plugin.{ID}.echo")).is_none());
    let err = (held.run)(&mut Sess, &json!({})).unwrap_err();
    assert!(matches!(err.0, CallError::NotLoaded), "{err:?}");
    // Enable again: the old handle works again because it routes by plugin id.
    h.manager.enable(ID).unwrap();
    assert_eq!((held.run)(&mut Sess, &json!({"k": 1})).unwrap()["params"], json!({"k": 1}));
}

#[test]
fn a_command_body_keeps_the_session_and_lends_it_to_the_guests_host_calls() {
    // Entry 7 of the gates guest runs `cmd.execute app.set` and returns. The app serves that call
    // only while the session is lent. The body itself must hold the real session, so the lend
    // happens inside the body and not around it.
    use std::sync::atomic::Ordering;
    let h = Harness::new();
    h.services.require_lend.store(true, Ordering::Relaxed);
    let manifest = json!({
        "schema": 1, "id": ID, "name": "Setter", "version": "1.0.0",
        "abi": { "craft": 1 },
        "requires": { "ops": ["cmd.execute"] },
        "permissions": { "required": ["commands.run:app.*"], "optional": [] },
        "limits": { "memory_mb": 4, "stack_kb": 128 },
        "contributions": [{ "kind": "command", "id": "setter", "entry": 7 }]
    });
    h.manager.install(&package_from(&fixture_text("gates"), &manifest), Source::File).unwrap();
    h.manager.grant(ID, Decision { accept_required: true, optional: vec![] }).unwrap();
    h.run(&format!("plugin.{ID}.setter"), &json!({})).unwrap();
    h.services.recorded(|x| {
        assert_eq!(x.executed.len(), 1, "the guest's cmd.execute reached the app: {:?}", x.executed);
        assert_eq!(x.executed[0].2, "app.set");
    });
    assert_eq!(h.services.lends.load(Ordering::Relaxed), 1);
}
