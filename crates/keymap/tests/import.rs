//! Import of synthetic shortcut sets written for these tests.

use std::collections::BTreeMap;

use craft_keymap::chord::foreign::{Flavor, ForeignError};
use craft_keymap::import::{FailedChange, UnmatchedReason};
use craft_keymap::{
    Binding, CommandDef, CommandKind, CommandRef, ContextRule, ForeignEntry, ForeignKey, ForeignSet, ImportOptions, ImportReport, Keymap,
    MAX_COMMANDS, Overrides, Platform, Scope, Scopes, plan, reserved,
};
use serde_json::json;

fn b(text: &str) -> Binding {
    Binding::parse(text).unwrap_or_else(|e| panic!("{text}: {e}"))
}

fn bindings(texts: &[&str]) -> Vec<Binding> {
    texts.iter().map(|t| b(t)).collect()
}

fn command(id: impl Into<CommandRef>, label: &str, path: &[&str], keys: &[&str]) -> CommandDef {
    let mut out = CommandDef::new(id, label, Scope::app());
    if !path.is_empty() {
        out = out.with_path(path.iter().map(|s| s.to_string()).collect());
    }
    keys.iter().fold(out, |e, k| e.with_default(b(k)))
}

fn panel(name: &str) -> CommandRef {
    CommandRef::with_params("window.panel", json!({ "panel": name }))
}

fn catalog() -> Vec<CommandDef> {
    vec![
        command("file.new", "Document…", &["File", "New", "Document…"], &["Cmd+N"]),
        command("dialog.newDocument", "New Document Dialog", &[], &["Cmd+N"]),
        command("file.close", "Close", &["File", "Close"], &["Cmd+W"]),
        command("file.save", "Save", &["File", "Save"], &["Cmd+S"]),
        command("file.exit", "Exit", &["File", "Exit"], &[]),
        command("edit.paste", "Paste", &["Edit", "Paste"], &["Cmd+V"]),
        command("edit.clear", "Clear", &["Edit", "Clear"], &[]),
        command("edit.duplicate", "Duplicate", &["Edit", "Duplicate"], &["Cmd+Alt+Shift+D"]),
        command("edit.checkSpelling", "Check Spelling…", &["Edit", "Proofing", "Check Spelling…"], &[]),
        command("edit.pen", "Pen", &["Edit", "Preferences", "Pen"], &[]),
        command("app.palette", "Command Palette", &[], &["Cmd+Alt+K"]),
        command(panel("layers"), "Layers", &["Window", "Layers"], &["F7"]),
        command(panel("pages"), "Pages", &["Window", "Pages"], &["F12"]),
        command("view.guides", "Guides", &["View", "Grids & Guides", "Guides"], &["Cmd+;"]).toggle(),
        command("view.goToPage", "Go to Page…", &["View", "Go to Page…"], &["Cmd+J"]),
        command("view.zoomIn", "Zoom In", &["View", "Zoom In"], &["Cmd+="]),
        command("object.textFrameOptions", "Text Frame Options…", &["Object", "Text Frame Options…"], &["Cmd+B"]),
        command("type.break", "Break", &["Type", "Breaks", "Break"], &[]),
        command("type.columnBreak", "Insert Break", &["Type", "Column", "Insert Break"], &[]),
        command("type.frameBreak", "Insert Break", &["Type", "Frame", "Insert Break"], &[]),
        command("type.bold", "Bold", &["Type", "Style", "Bold"], &[]),
        command("type.italic", "Italic", &["Type", "Style", "Italic"], &[]),
        command("tool.pen", "Pen Tool", &[], &["P"]).with_kind(CommandKind::Tool { group: None }).with_alias("Pen"),
        command("tool.type", "Type Tool", &[], &["T"]).with_kind(CommandKind::Tool { group: None }),
        command("scripts.run", "Run Script", &[], &[]),
    ]
}

fn keymap(platform: Platform) -> Keymap {
    Keymap::new(catalog(), Scopes::new().with_context("Text"), platform).with_reserved(reserved::system(platform))
}

fn key(flavor: Flavor, context: &str, text: &str) -> ForeignKey {
    ForeignKey::read(Some(context), text, flavor)
}

fn entry(menu: Option<&str>, path: &[&str], keys: &[(&str, &str)], flavor: Flavor) -> ForeignEntry {
    keys.iter().fold(ForeignEntry::new(menu, path.iter().copied()), |e, (c, t)| e.with_key(key(flavor, c, t)))
}

fn win(menu: &str, path: &[&str], keys: &[(&str, &str)]) -> ForeignEntry {
    entry(Some(menu), path, keys, Flavor::Windows)
}

/// An InDesign-like Windows set, one entry per match rule or apply rule.
fn windows_set() -> ForeignSet {
    let d = "Default";
    let entries = vec![
        win("File", &["New", "Document..."], &[(d, "Ctrl+N")]),
        win("File", &["Close"], &[(d, "Ctrl+W"), (d, "Ctrl+F4")]),
        win("File", &["Save"], &[(d, "Ctrl+Hyperspace")]),
        win("File", &["Exit"], &[(d, "Alt+F4")]),
        win("Edit", &["Paste"], &[(d, "Ctrl+E")]),
        win("Edit", &["Clear"], &[(d, "Ctrl+E")]),
        win("Edit", &["Duplicate"], &[]),
        win("Edit", &["Duplicate"], &[]),
        win("Edit", &["Spelling", "Check Spelling..."], &[(d, "Ctrl+Alt+K"), ("Text", "F7")]),
        win("Edit", &["Frobnicate Everything"], &[(d, "Ctrl+F12")]),
        win("Edit", &["Never Heard Of It"], &[]),
        win("Window", &["Layers"], &[(d, "F7")]),
        win("Window", &["Pages"], &[(d, "Shift+F12")]),
        win("View", &["Grids & Guides", "SHOW/HIDE   Guides"], &[(d, "Ctrl+Alt+;")]),
        win("Layout", &["Go To Page"], &[(d, "Ctrl+Alt+J")]),
        win("Object", &["Text Frame", "Options"], &[(d, "Ctrl+Alt+B")]),
        win("Type", &["Insert", "Break"], &[(d, "Ctrl+Alt+Enter")]),
        win("Type", &["Insert Break"], &[(d, "Shift+Enter")]),
        win("Type", &["Style", "Bold"], &[("Text", "Shift+Ctrl+B")]),
        win("Type", &["Style", "Italic"], &[(d, "Ctrl+I"), ("Table", "Ctrl+Alt+I")]),
        win("Tools", &["Pen"], &[(d, "Shift+P")]).tool(),
        win("Tools", &["Zoom In"], &[(d, "Z")]).tool(),
        entry(None, &["Zoom In"], &[(d, "Ctrl+Alt+I")], Flavor::Windows),
        entry(None, &["Somewhere", "Zoom In"], &[(d, "Ctrl+F11")], Flavor::Windows),
        win("Scripts", &["Frobnicate"], &[(d, "Ctrl+Alt+R")]),
    ];
    ForeignSet::new(entries).with_name("My Set")
}

fn windows_options() -> ImportOptions {
    let mut remembered = BTreeMap::new();
    remembered.insert(ForeignEntry::new(Some("Scripts"), ["Frobnicate"]).remember_key(), CommandRef::new("scripts.run"));
    ImportOptions {
        contexts: vec![("default".into(), ContextRule::Home), ("TEXT".into(), ContextRule::HomeWithoutHomeEntry)],
        remembered,
        ..ImportOptions::default()
    }
}

fn import_windows() -> (Keymap, ImportReport) {
    let mut km = keymap(Platform::Windows);
    let report = plan(&windows_set(), &km, &windows_options()).apply(&mut km);
    (km, report)
}

fn has(km: &Keymap, id: impl Into<CommandRef>, texts: &[&str]) {
    let c = id.into();
    assert_eq!(km.bindings(&c), bindings(texts), "{}", c.key());
}

#[test]
fn every_match_step_finds_its_command() {
    let (km, r) = import_windows();
    assert_eq!(r.set.as_deref(), Some("My Set"));
    // 1: full path, with `&`, case and spacing normalized, and Show/Hide dropped for a toggle.
    has(&km, "view.guides", &["Cmd+Alt+;"]);
    // 2: joined segments `Text Frame Options` in Object.
    has(&km, "object.textFrameOptions", &["Cmd+Alt+B"]);
    // 2 is ambiguous (two `Insert Break` in Type), 3 finds `Break`.
    has(&km, "type.break", &["Cmd+Alt+Enter"]);
    // 3: last segment in the menu, under another submenu.
    has(&km, "edit.checkSpelling", &["Cmd+Alt+K"]);
    // 4: the label anywhere; also with no menu for a one-segment path.
    has(&km, "view.goToPage", &["Cmd+Alt+J"]);
    has(&km, "view.zoomIn", &["Cmd+Alt+I"]);
    // 0: a remembered answer.
    has(&km, "scripts.run", &["Cmd+Alt+R"]);
    // A command with parameters, by its path.
    has(&km, panel("pages"), &["Shift+F12"]);
    // A tool entry matches the tool, not the command with the same label.
    has(&km, "tool.pen", &["Shift+P"]);
    has(&km, "edit.pen", &[]);

    let unmatched: Vec<(&str, UnmatchedReason)> = r.unmatched.iter().map(|u| (u.item.as_str(), u.reason)).collect();
    assert_eq!(
        unmatched,
        [
            ("Edit > Frobnicate Everything", UnmatchedReason::NoMatch),
            ("Type > Insert Break", UnmatchedReason::Ambiguous),
            ("Type > Style > Italic", UnmatchedReason::Context),
            ("Tools > Zoom In", UnmatchedReason::Tool),
            ("Somewhere > Zoom In", UnmatchedReason::NoMatch),
        ]
    );
    assert_eq!(r.unmatched[2].keys, "Table: Ctrl+Alt+I");
    assert_eq!(r.unmatched_without_shortcut, 1);
    assert_eq!(r.unreadable.len(), 1);
    assert_eq!(r.unreadable[0].item, "File > Save");
    assert_eq!(r.unreadable[0].keys, "Default: Ctrl+Hyperspace");
    assert!(matches!(r.unreadable[0].reason, ForeignError::UnknownKey(_)));
    assert!(r.suggestions.is_empty(), "not asked for");
    assert_eq!(r.truncated, 0);
}

#[test]
fn a_failed_change_keeps_its_shortcut_when_a_later_change_frees_room() {
    // Save's change finds the overrides full. Close's change then frees an entry, and Zoom In
    // takes Cmd+S. Removing Cmd+S from Save would fit now, but Save's import failed, so Save
    // keeps it and Zoom In stays as it was.
    let set = ForeignSet::new(vec![
        win("File", &["Save"], &[("Default", "Ctrl+Alt+S")]),
        win("File", &["Close"], &[("Default", "Ctrl+W")]),
        win("View", &["Zoom In"], &[("Default", "Ctrl+S")]),
    ]);
    let mut o = Overrides::new();
    o.insert("file.close", vec!["Cmd+Q".into()]);
    o.insert("view.zoomIn", vec!["Cmd+Alt+Z".into()]);
    for n in 2..MAX_COMMANDS {
        o.insert(format!("junk.{n}"), vec!["Cmd+K".into()]);
    }
    assert_eq!(o.len(), MAX_COMMANDS);
    let options = ImportOptions { contexts: vec![("Default".into(), ContextRule::Home)], ..ImportOptions::default() };
    let mut km = keymap(Platform::Windows).with_overrides(o);
    let r = plan(&set, &km, &options).apply(&mut km);
    has(&km, "file.save", &["Cmd+S"]);
    has(&km, "file.close", &["Cmd+W"]);
    has(&km, "view.zoomIn", &["Cmd+Alt+Z"]);
    let failed: Vec<(&str, FailedChange)> = r.failed.iter().map(|f| (f.command.id.as_str(), f.change)).collect();
    assert_eq!(failed, [("file.save", FailedChange::Bind), ("view.zoomIn", FailedChange::Bind)]);
    assert!(r.conflicts_cleared.is_empty(), "{:?}", r.conflicts_cleared);
    assert_eq!(r.imported, 1);
}
