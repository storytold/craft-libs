//! Integration tests of craft-keymap: editor.

use craft_keymap::{Binding, CommandDef, CommandRef, EditSession, Keymap, Platform, Policy, Scope, Scopes};

fn b(text: &str) -> Binding {
    Binding::parse(text).unwrap_or_else(|e| panic!("{text}: {e}"))
}

fn keymap(platform: Platform) -> Keymap {
    let catalog = vec![
        CommandDef::new("file.save", "Save", Scope::app()).with_default(b("Cmd+S")).with_path(vec!["File".into(), "Save".into()]),
        CommandDef::new("edit.copy", "Copy", Scope::app()).with_default(b("Cmd+C")).with_alias("Kopieren"),
        CommandDef::new("view.zoomIn", "Zoom In", Scope::app()).with_default(b("Cmd+=")),
        CommandDef::new("tool.brush", "Brush", Scope::app()).with_default(b("B")).with_path(vec!["Tools".into(), "Paint".into()]),
    ];
    Keymap::new(catalog, Scopes::new(), platform)
}

fn keys(km: &Keymap, id: &str) -> Vec<String> {
    km.bindings(&CommandRef::new(id)).iter().map(|b| b.to_string()).collect()
}

#[test]
fn edits_undo_redo_and_cancel() {
    let mut km = keymap(Platform::Windows);
    let mut session = EditSession::begin(&km);
    let save = CommandRef::new("file.save");

    let assigned = session.edit(&mut km, |km| km.assign(&save, b("Cmd+Shift+S"), Policy::Steal));
    assert!(assigned.is_ok());
    session.edit(&mut km, |km| km.clear(&CommandRef::new("edit.copy")));
    assert_eq!(keys(&km, "file.save"), ["Cmd+S", "Cmd+Shift+S"]);
    assert!(keys(&km, "edit.copy").is_empty());
    assert!(session.is_modified(&km));

    // An edit that changes nothing is no undo step.
    session.edit(&mut km, |km| km.reset(&CommandRef::new("tool.brush")));

    assert!(session.undo(&mut km));
    assert_eq!(keys(&km, "edit.copy"), ["Cmd+C"]);
    assert!(session.undo(&mut km));
    assert_eq!(keys(&km, "file.save"), ["Cmd+S"]);
    assert!(!session.undo(&mut km));
    assert!(!session.is_modified(&km));

    assert!(session.redo(&mut km));
    assert_eq!(keys(&km, "file.save"), ["Cmd+S", "Cmd+Shift+S"]);
    // A new edit drops what could be redone.
    session.edit(&mut km, |km| km.clear(&CommandRef::new("tool.brush")));
    assert!(!session.redo(&mut km));

    session.cancel(&mut km);
    assert!(!session.is_modified(&km));
    assert_eq!(keys(&km, "file.save"), ["Cmd+S"]);
    assert_eq!(keys(&km, "tool.brush"), ["B"]);
    assert!(!session.can_undo());
}
