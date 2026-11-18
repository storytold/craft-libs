//! The allowlist, the shared refused list and an app's own.

use serde_json::json;

use crate::permissions::{changed, pattern_matches};
use crate::{ALWAYS_DENIED, Manifest, Policy, check_call, check_read_only};

fn manifest(permissions: serde_json::Value) -> Manifest {
    Manifest::from_value(&json!({"id": "x.y", "name": "X", "main": "x.wasm", "permissions": permissions})).unwrap()
}

/// FigCraft's rules, as its engine would state them.
fn figcraft() -> Policy {
    Policy::new(
        &["widget.*", "doc.*", "version.*", "help.*", "library.add*", "command.batch", "collab.*", "view.theme"],
        &["doc.list", "version.list"],
    )
}

#[test]
fn patterns_are_exact_or_prefixes() {
    assert!(pattern_matches("node.*", "node.rename"));
    assert!(!pattern_matches("node.*", "nodes.rename"));
    assert!(pattern_matches("library.add*", "library.add"));
    assert!(pattern_matches("edit.undo", "edit.undo"));
    assert!(!pattern_matches("edit.undo", "edit.undoAll"));
    assert!(pattern_matches("*", "anything"));
}

#[test]
fn every_app_refuses_the_shared_list() {
    let m = manifest(json!({"access": "write"}));
    let none = Policy::default();
    for cmd in ["plugin.install", "plugins.list", "edit.undo", "edit.redo", "file.open", "app.quit", "pref.set", "window.new"] {
        let e = check_call(&m, &none, cmd).unwrap_err();
        assert!(e.contains("isn't available to plugins"), "{cmd}: {e}");
    }
    assert!(ALWAYS_DENIED.contains(&"plugin.*"));
    assert_eq!(check_call(&m, &none, "shape.rectangle"), Ok(()));
    assert_eq!(check_call(&m, &none, "edit.copy"), Ok(()));
}

#[test]
fn an_apps_own_list_and_exceptions_apply() {
    let m = manifest(json!({"access": "write"}));
    let p = figcraft();
    assert!(check_call(&m, &p, "doc.open").is_err());
    assert!(check_call(&m, &p, "library.addServer").is_err());
    assert_eq!(check_call(&m, &p, "doc.list"), Ok(()));
    assert_eq!(check_call(&m, &p, "version.list"), Ok(()));
    // Exceptions never reopen the shared list.
    let lax = Policy::new(&[], &["plugin.list", "file.open"]);
    assert!(check_call(&m, &lax, "plugin.list").is_err());
    assert!(check_call(&m, &lax, "file.open").is_err());
}

#[test]
fn the_manifests_allowlist_applies() {
    let m = manifest(json!({"access": "write", "commands": ["select.*", "swatch.create"]}));
    let p = Policy::default();
    assert_eq!(check_call(&m, &p, "select.all"), Ok(()));
    assert_eq!(check_call(&m, &p, "swatch.create"), Ok(()));
    let e = check_call(&m, &p, "swatch.delete").unwrap_err();
    assert!(e.contains("asked permission for"), "{e}");
    // An allowlist can't reach past the refused lists.
    let greedy = manifest(json!({"access": "write", "commands": ["file.*", "plugin.*"]}));
    assert!(check_call(&greedy, &p, "file.save").is_err());
}

#[test]
fn junk_command_ids_are_refused() {
    let m = manifest(json!({}));
    for cmd in ["", " select.all", "select.all\n", "a\u{0}b", &"x".repeat(129)] {
        assert!(check_call(&m, &Policy::default(), cmd).is_err(), "{cmd:?}");
    }
}

#[test]
fn read_access_refuses_edits() {
    let read = manifest(json!({}));
    let write = manifest(json!({"access": "write"}));
    assert_eq!(check_read_only(&read, "select.all", false), Ok(()));
    assert!(check_read_only(&read, "shape.rectangle", true).unwrap_err().contains("may only read"));
    assert_eq!(check_read_only(&write, "shape.rectangle", true), Ok(()));
}

#[test]
fn a_permission_change_needs_new_consent() {
    let a = manifest(json!({"commands": ["select.*"]}));
    assert!(!changed(&a, &a.clone()));
    assert!(changed(&a, &manifest(json!({"access": "write", "commands": ["select.*"]}))));
    assert!(changed(&a, &manifest(json!({"commands": ["select.*", "node.*"]}))));
}

#[test]
fn exceptions_may_be_patterns() {
    let m = manifest(json!({"access": "write"}));
    // An allowlist app: refuse everything, then allow families and single ids.
    let allow = Policy::new(&["*"], &["select.*", "shape.rectangle"]);
    assert_eq!(check_call(&m, &allow, "select.all"), Ok(()));
    assert_eq!(check_call(&m, &allow, "select.none"), Ok(()));
    assert_eq!(check_call(&m, &allow, "shape.rectangle"), Ok(()));
    assert!(check_call(&m, &allow, "shape.ellipse").is_err());
    assert!(check_call(&m, &allow, "selection.clear").is_err(), "`select.*` doesn't admit `selection.*`");
    assert!(allow.excepts("select.all"));
    assert!(!allow.excepts("doc.open"));
    // Exact exceptions behave as before.
    let p = figcraft();
    assert!(p.excepts("doc.list"));
    assert!(!p.excepts("doc.listAll"));
    assert!(check_call(&m, &p, "doc.listAll").is_err());
}

#[test]
fn pattern_exceptions_never_reopen_the_shared_list() {
    let m = manifest(json!({"access": "write", "commands": ["*"]}));
    for exceptions in [&["*"][..], &["file.*", "plugin*", "edit.*", "app.*", "pref.*", "window.*"][..]] {
        for denied in [&[][..], &["*"][..]] {
            let p = Policy::new(denied, exceptions);
            for cmd in
                ["plugin.install", "plugins.list", "edit.undo", "edit.redo", "file.open", "file.newSequence", "app.quit", "pref.set", "window.new"]
            {
                assert!(p.refuses(cmd), "{exceptions:?} {denied:?} {cmd}");
                assert!(check_call(&m, &p, cmd).unwrap_err().contains("isn't available to plugins"), "{cmd}");
            }
        }
    }
    // A lone `*` exception reopens the app's own list only.
    let p = Policy::new(&["*"], &["*"]);
    assert_eq!(check_call(&m, &p, "edit.copy"), Ok(()));
}
