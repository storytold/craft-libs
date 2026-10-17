//! Import of synthetic shortcut sets written for these tests.

use std::collections::BTreeMap;

use craft_keymap::chord::foreign::ReadNote;
use craft_keymap::chord::foreign::{Flavor, ForeignError};
use craft_keymap::import::{
    Failed, FailedChange, MAX_IMPORT_ENTRIES, MAX_IMPORT_TEXT, MAX_KEYS_PER_ENTRY, MAX_SUGGESTIONS, SkipReason, Skipped, UnmatchedReason,
    normalize_label,
};
use craft_keymap::{
    Bindable, Binding, CommandRef, ContextRule, ForeignEntry, ForeignKey, ForeignSet, ImportOptions, ImportReport, Keymap, Kind, MAX_COMMANDS,
    NoneDefined, Overrides, Platform, Scope, Scopes, Start, plan, reserved,
};
use serde_json::json;

fn b(text: &str) -> Binding {
    Binding::parse(text).unwrap_or_else(|e| panic!("{text}: {e}"))
}

fn bindings(texts: &[&str]) -> Vec<Binding> {
    texts.iter().map(|t| b(t)).collect()
}

fn command(id: impl Into<CommandRef>, label: &str, path: &[&str], keys: &[&str]) -> Bindable {
    let mut out = Bindable::new(id, label, Scope::app());
    if !path.is_empty() {
        out = out.with_path(path.iter().map(|s| s.to_string()).collect());
    }
    keys.iter().fold(out, |e, k| e.with_default(b(k)))
}

fn panel(name: &str) -> CommandRef {
    CommandRef::with_params("window.panel", json!({ "panel": name }))
}

fn catalogue() -> Vec<Bindable> {
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
        command("view.guides", "Guides", &["View", "Grids & Guides", "Guides"], &["Cmd+;"]),
        command("view.goToPage", "Go to Page…", &["View", "Go to Page…"], &["Cmd+J"]),
        command("view.zoomIn", "Zoom In", &["View", "Zoom In"], &["Cmd+="]),
        command("object.textFrameOptions", "Text Frame Options…", &["Object", "Text Frame Options…"], &["Cmd+B"]),
        command("type.break", "Break", &["Type", "Breaks", "Break"], &[]),
        command("type.columnBreak", "Insert Break", &["Type", "Column", "Insert Break"], &[]),
        command("type.frameBreak", "Insert Break", &["Type", "Frame", "Insert Break"], &[]),
        command("type.bold", "Bold", &["Type", "Style", "Bold"], &[]),
        command("type.italic", "Italic", &["Type", "Style", "Italic"], &[]),
        command("tool.pen", "Pen Tool", &[], &["P"]).with_kind(Kind::Tool { group: None }).with_alias("Pen"),
        command("tool.type", "Type Tool", &[], &["T"]).with_kind(Kind::Tool { group: None }),
        command("scripts.run", "Run Script", &[], &[]),
    ]
}

fn keymap(platform: Platform) -> Keymap {
    Keymap::new(catalogue(), Scopes::new().with_context("Text"), platform).with_reserved(reserved::system(platform))
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
    ForeignSet { name: Some("My Set".into()), entries }
}

fn numpad_set() -> ForeignSet {
    ForeignSet { name: None, entries: vec![win("Window", &["Layers"], &[("Default", "Num 7")]).at_line(2)] }
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
fn labels_normalise_for_matching() {
    assert_eq!(normalize_label("Check Spelling..."), "check spelling");
    assert_eq!(normalize_label("Check  Spelling…"), "check spelling");
    assert_eq!(normalize_label("Grids && Guides"), "grids guides");
    assert_eq!(normalize_label("SHOW/HIDE Guides"), "guides");
    assert_eq!(normalize_label("Show Hidden Characters"), "hidden characters");
    assert_eq!(normalize_label("Hide "), "hide", "nothing left after the prefix");
    assert_eq!(normalize_label("ÄRGER"), "ärger");
}

#[test]
fn every_match_step_finds_its_command() {
    let (km, r) = import_windows();
    assert_eq!(r.set.as_deref(), Some("My Set"));
    // 1: full path, with `&`, case, spacing and Show/Hide normalised.
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
fn the_set_wins_and_the_report_says_how() {
    let (km, r) = import_windows();
    // Two keys for one command, both kept in order.
    has(&km, "file.close", &["Cmd+W", "Cmd+F4"]);
    // The Text context counts only without a Default key.
    has(&km, "type.bold", &["Cmd+Shift+B"]);
    has(&km, "type.italic", &["Cmd+I"]);
    has(&km, panel("layers"), &["F7"]);
    // First in file order keeps a chord.
    has(&km, "edit.paste", &["Cmd+E"]);
    has(&km, "edit.clear", &[]);
    let skipped: Vec<_> = r.skipped.iter().map(|s| (s.command.id.as_str(), s.shortcut.as_str(), s.kept_by.clone(), s.reason)).collect();
    assert_eq!(
        skipped,
        [("file.exit", "Alt+F4", None, SkipReason::Reserved), ("edit.clear", "Cmd+E", Some(CommandRef::new("edit.paste")), SkipReason::Taken)]
    );
    // Taken from a command outside the set; the twin keeps its shared default.
    has(&km, "app.palette", &[]);
    has(&km, "dialog.newDocument", &["Cmd+N"]);
    let cleared: Vec<_> = r.conflicts_cleared.iter().map(|c| (c.command.id.as_str(), c.label.as_str(), c.shortcut.as_str())).collect();
    assert_eq!(cleared, [("app.palette", "Command Palette", "Cmd+Alt+K")]);
    // None defined clears; an unreadable key leaves the command as it was.
    has(&km, "edit.duplicate", &[]);
    has(&km, "file.save", &["Cmd+S"]);
    // Bindings equal to the defaults leave no override.
    let o = km.overrides();
    assert_eq!(o.get("file.new"), None);
    assert_eq!(o.get(&panel("layers").key()), None);
    assert_eq!(o.get("edit.duplicate"), Some(&[][..]));
    assert_eq!(o.get("file.close"), Some(&["Cmd+W".to_string(), "Cmd+F4".to_string()][..]));

    let changed: Vec<&str> = r.changed.iter().map(|c| c.command.id.as_str()).collect();
    assert_eq!(
        changed,
        [
            "file.close",
            "edit.paste",
            "edit.duplicate",
            "edit.checkSpelling",
            "window.panel",
            "view.guides",
            "view.goToPage",
            "object.textFrameOptions",
            "type.break",
            "type.bold",
            "type.italic",
            "tool.pen",
            "view.zoomIn",
            "scripts.run",
        ]
    );
    assert_eq!(r.imported, changed.len());
    let close = &r.changed[0];
    assert_eq!((close.label.as_str(), close.item.as_str()), ("Close", "File > Close"));
    assert_eq!((close.from.clone(), close.to.clone()), (vec!["Cmd+W".to_string()], vec!["Cmd+W".to_string(), "Cmd+F4".to_string()]));
}

#[test]
fn a_mac_set_reads_its_modifiers_and_scopes_text_keys() {
    let mut km = keymap(Platform::Mac);
    let mac = |menu: &str, path: &[&str], keys: &[(&str, &str)]| entry(Some(menu), path, keys, Flavor::Mac);
    let set = ForeignSet {
        name: Some("Mac".into()),
        entries: vec![
            mac("File", &["Save"], &[("Default", "⇧⌘S")]),
            mac("Edit", &["Paste"], &[("Default", "Cmd+Ctrl+V")]),
            mac("Type", &["Style", "Italic"], &[("Text", "Shift+Cmd+I")]),
            mac("Type", &["Style", "Bold"], &[("Default", "Cmd+B"), ("Tables", "Cmd+Opt+B")]),
        ],
    };
    let text = Scope::new("Text").unwrap();
    let options = ImportOptions {
        contexts: vec![("Default".into(), ContextRule::Home), ("Text".into(), ContextRule::Scope(text))],
        ..ImportOptions::default()
    };
    let r = plan(&set, &km, &options).apply(&mut km);
    has(&km, "file.save", &["Cmd+Shift+S"]);
    has(&km, "edit.paste", &["Cmd+Ctrl+V"]);
    has(&km, "type.italic", &["Text: Cmd+Shift+I"]);
    has(&km, "type.bold", &["Cmd+B"]);
    has(&km, "object.textFrameOptions", &[]);
    assert_eq!(r.conflicts_cleared[0].command.id, "object.textFrameOptions");
    assert_eq!(r.unmatched.len(), 1);
    assert_eq!((r.unmatched[0].keys.as_str(), r.unmatched[0].reason), ("Tables: Cmd+Opt+B", UnmatchedReason::Context));
    assert_eq!(r.changed.iter().find(|c| c.command.id == "type.italic").map(|c| c.to.clone()), Some(vec!["Text: Cmd+Shift+I".to_string()]));
}

#[test]
fn start_and_none_defined_options() {
    let set = ForeignSet { name: None, entries: vec![win("File", &["Close"], &[("Default", "Ctrl+Q")]), win("Edit", &["Duplicate"], &[])] };
    let user = || {
        let mut o = Overrides::new();
        o.insert("file.save", vec!["Cmd+Alt+S".into()]);
        o
    };
    let options = |start, none_defined| ImportOptions {
        contexts: vec![("Default".into(), ContextRule::Home)],
        start,
        none_defined,
        ..ImportOptions::default()
    };

    let mut km = keymap(Platform::Windows).with_overrides(user());
    plan(&set, &km, &options(Start::Current, NoneDefined::Keep)).apply(&mut km);
    has(&km, "file.save", &["Cmd+Alt+S"]);
    has(&km, "file.close", &["Cmd+Q"]);
    has(&km, "edit.duplicate", &["Cmd+Alt+Shift+D"]);

    let mut km = keymap(Platform::Windows).with_overrides(user());
    let r = plan(&set, &km, &options(Start::Defaults, NoneDefined::Clear)).apply(&mut km);
    has(&km, "file.save", &["Cmd+S"]);
    has(&km, "file.close", &["Cmd+Q"]);
    has(&km, "edit.duplicate", &[]);
    assert_eq!(r.imported, 2);
}

#[test]
fn changes_that_find_the_overrides_full_are_reported() {
    let set = ForeignSet {
        name: None,
        entries: vec![
            win("File", &["Close"], &[("Default", "Ctrl+J")]),
            win("File", &["Save"], &[("Default", "Ctrl+Alt+S")]),
            win("Edit", &["Duplicate"], &[]),
        ],
    };
    let full = || {
        let mut o = Overrides::new();
        o.insert("file.close", vec!["Cmd+Q".into()]);
        for n in 1..MAX_COMMANDS {
            o.insert(format!("junk.{n}"), vec!["Cmd+K".into()]);
        }
        assert_eq!(o.len(), MAX_COMMANDS);
        o
    };
    let options = |start| ImportOptions { contexts: vec![("Default".into(), ContextRule::Home)], start, ..ImportOptions::default() };

    // file.close has an entry to rewrite; the other changes each need a new one.
    let mut km = keymap(Platform::Windows).with_overrides(full());
    let r = plan(&set, &km, &options(Start::Current)).apply(&mut km);
    has(&km, "file.close", &["Cmd+J"]);
    has(&km, "view.goToPage", &["Cmd+J"]);
    has(&km, "file.save", &["Cmd+S"]);
    has(&km, "edit.duplicate", &["Cmd+Alt+Shift+D"]);
    let row = |id: &str, label: &str, change, shortcuts: &[&str]| Failed {
        command: id.into(),
        label: label.into(),
        change,
        shortcuts: shortcuts.iter().map(|s| s.to_string()).collect(),
    };
    assert_eq!(
        r.failed,
        [
            row("file.save", "Save", FailedChange::Bind, &["Cmd+Alt+S"]),
            row("edit.duplicate", "Duplicate", FailedChange::Clear, &[]),
            row("view.goToPage", "Go to Page…", FailedChange::Unbind, &["Cmd+J"]),
        ]
    );
    assert_eq!(r.imported, 1);
    assert!(r.conflicts_cleared.is_empty());
    let v = serde_json::to_value(&r).unwrap();
    assert_eq!(
        v["failed"][0],
        json!({"command": {"id": "file.save", "params": null}, "label": "Save", "change": "bind", "shortcuts": ["Cmd+Alt+S"]})
    );

    // Starting from the defaults drops the entries, so everything fits.
    let mut km = keymap(Platform::Windows).with_overrides(full());
    let r = plan(&set, &km, &options(Start::Defaults)).apply(&mut km);
    assert!(r.failed.is_empty(), "{:?}", r.failed);
    has(&km, "file.close", &["Cmd+J"]);
    has(&km, "view.goToPage", &[]);
    has(&km, "file.save", &["Cmd+Alt+S"]);
    has(&km, "edit.duplicate", &[]);
}

#[test]
fn keys_in_unknown_scopes_are_skipped_and_the_rest_apply() {
    let set = ForeignSet { name: None, entries: vec![win("File", &["Close"], &[("Default", "Ctrl+Q"), ("Story", "Ctrl+K"), ("Text", "Ctrl+L")])] };
    let options = ImportOptions {
        contexts: vec![
            ("Default".into(), ContextRule::Home),
            ("Story".into(), ContextRule::Scope(Scope::new("Story").unwrap())),
            ("Text".into(), ContextRule::Scope(Scope::new("Text").unwrap())),
        ],
        ..ImportOptions::default()
    };
    let mut km = keymap(Platform::Windows);
    let r = plan(&set, &km, &options).apply(&mut km);
    has(&km, "file.close", &["Cmd+Q", "Text: Cmd+L"]);
    assert_eq!(
        r.skipped,
        [Skipped {
            command: "file.close".into(),
            item: "File > Close".into(),
            line: None,
            shortcut: "Story: Cmd+K".into(),
            kept_by: None,
            reason: SkipReason::UnknownScope
        }]
    );
    assert_eq!(serde_json::to_value(r.skipped[0].reason).unwrap(), "unknownScope");
}

#[test]
fn suggestions_are_reported_never_applied() {
    let mut km = keymap(Platform::Windows);
    let set = ForeignSet { name: None, entries: vec![win("Edit", &["Chek Spelling Now"], &[("Default", "Ctrl+K")])] };
    let options = ImportOptions { contexts: vec![("Default".into(), ContextRule::Home)], suggest: true, ..ImportOptions::default() };
    let p = plan(&set, &km, &options);
    assert!(p.commands.is_empty());
    let r = p.apply(&mut km);
    assert_eq!(r.suggestions.len(), 1);
    assert_eq!(r.suggestions[0].item, "Edit > Chek Spelling Now");
    assert!(r.suggestions[0].candidates.contains(&CommandRef::new("edit.checkSpelling")), "{:?}", r.suggestions);
    has(&km, "edit.checkSpelling", &[]);
    assert!(km.overrides().is_empty());
    assert_eq!(r.imported, 0);
}

#[test]
fn hostile_sets_are_capped_and_counted() {
    // A full catalogue whose labels share words with every unmatched entry below, so each
    // suggestion scores every command.
    let mut catalogue = catalogue();
    let filler = MAX_COMMANDS - catalogue.len();
    catalogue.extend((0..filler).map(|i| command(format!("filler.c{i}").as_str(), &format!("Filler Command {i}"), &[], &[])));
    let mut km = Keymap::new(catalogue, Scopes::new().with_context("Text"), Platform::Windows).with_reserved(reserved::system(Platform::Windows));
    let long: String = "ä€𝄞 x".repeat(MAX_IMPORT_TEXT);
    // Odd entries have keys and long labels that match nothing, so they reach the suggestions:
    // words in the whole label, or two words and symbols, which score high enough to suggest.
    let wordy = "Filler Command ä€𝄞 ".repeat(MAX_IMPORT_TEXT / 8);
    let symbols = format!("Filler Command {}", "€𝄞 ".repeat(MAX_IMPORT_TEXT));
    let filler_entry = |i: usize| match i % 4 {
        1 => win("Edit", &[&symbols], &[("Default", "Ctrl+F12")]),
        3 => win("Edit", &[&format!("{wordy}{i}")], &[("Default", "Ctrl+F12")]),
        _ => win("Edit", &[&format!("Item {i}")], &[]),
    };
    let mut entries: Vec<ForeignEntry> = (0..MAX_IMPORT_ENTRIES + 500).map(filler_entry).collect();
    let mut big = ForeignEntry::new(Some(long.as_str()), std::iter::repeat_n(long.clone(), 100));
    for _ in 0..MAX_KEYS_PER_ENTRY + 10 {
        let trigger = Err(ForeignError::UnknownKey(long.clone()));
        big = big.with_key(ForeignKey { context: Some(long.clone()), text: long.clone(), trigger, notes: vec![ReadNote::Numpad; 1000] });
    }
    entries.insert(0, big);
    entries.insert(1, win("Edit", &["Paste"], &[("Default", &long), ("Default", "Ctrl+\u{0}"), ("Default", "Win+E")]));
    let set = ForeignSet { name: Some(long.clone()), entries };
    let options = ImportOptions { contexts: vec![("Default".into(), ContextRule::Home)], suggest: true, ..ImportOptions::default() };
    let started = std::time::Instant::now();
    let r = plan(&set, &km, &options).apply(&mut km);
    assert!(started.elapsed() < std::time::Duration::from_secs(60), "{:?}", started.elapsed());
    // Read filler entries are those before the cap, after the two inserted ones.
    let read = MAX_IMPORT_ENTRIES - 2;
    let keyed = read / 2;
    assert!(!r.suggestions.is_empty() && r.suggestions.len() <= MAX_SUGGESTIONS, "{}", r.suggestions.len());
    assert!(r.truncated >= 500 + 10 + keyed + 1 - MAX_SUGGESTIONS, "{}", r.truncated);
    assert_eq!(r.set.as_ref().map(|s| s.chars().count()), Some(MAX_IMPORT_TEXT));
    let row = r.unmatched.iter().find(|u| u.item.starts_with('ä')).unwrap();
    assert!(row.item.chars().count() <= MAX_IMPORT_TEXT * 34, "item of capped segments");
    assert!(row.keys.chars().count() <= (2 * MAX_IMPORT_TEXT + 4) * MAX_KEYS_PER_ENTRY);
    assert_eq!(r.unreadable.len(), 1, "Paste's keys");
    has(&km, "edit.paste", &["Cmd+V"]);
    // Entries past the cap are left out.
    assert_eq!(r.unmatched_without_shortcut, read - keyed);
}

#[test]
fn rows_carry_source_lines_and_approximated_keys() {
    let mut km = keymap(Platform::Windows);
    let d = "Default";
    let set = ForeignSet {
        name: None,
        entries: vec![
            win("Window", &["Layers"], &[(d, "Ctrl+Num 5"), (d, "Num 7")]).at_line(3),
            win("Window", &["Layers"], &[(d, "Shift+F7")]).at_line(4),
            win("Edit", &["Paste"], &[(d, "Ctrl+Num 5")]).at_line(5),
            win("Edit", &["Chek Spelling Now"], &[(d, "Ctrl+K")]).at_line(8),
            win("File", &["Save"], &[(d, "Ctrl+Hyperspace")]).at_line(9),
        ],
    };
    let options = ImportOptions { contexts: vec![(d.into(), ContextRule::Home)], suggest: true, ..ImportOptions::default() };
    let r = plan(&set, &km, &options).apply(&mut km);
    // A keypad key binds the main key and is reported.
    has(&km, panel("layers"), &["Cmd+5", "7", "Shift+F7"]);
    let approximated: Vec<_> = r.approximated.iter().map(|a| (a.item.as_str(), a.line, a.keys.as_str(), a.note)).collect();
    assert_eq!(
        approximated,
        [
            ("Window > Layers", Some(3), "Default: Ctrl+Num 5, Default: Num 7", ReadNote::Numpad),
            ("Edit > Paste", Some(5), "Default: Ctrl+Num 5", ReadNote::Numpad)
        ]
    );
    // Merged entries report the first one's line.
    let layers = r.changed.iter().find(|c| c.command == panel("layers")).unwrap();
    assert_eq!(layers.line, Some(3));
    assert_eq!((r.skipped[0].item.as_str(), r.skipped[0].line), ("Edit > Paste", Some(5)));
    assert_eq!(r.unmatched[0].line, Some(8));
    assert_eq!(r.suggestions[0].line, Some(8));
    assert_eq!(r.unreadable[0].line, Some(9));
}

#[test]
fn report_json_uses_camel_case_names() {
    let mut km = keymap(Platform::Windows);
    let mut options = windows_options();
    options.suggest = true;
    let r = plan(&windows_set(), &km, &options).apply(&mut km);
    let v = serde_json::to_value(&r).unwrap();
    let names = |v: &serde_json::Value| v.as_object().unwrap().keys().cloned().collect::<Vec<_>>();
    let mut top = names(&v);
    top.sort();
    assert_eq!(
        top,
        [
            "approximated",
            "changed",
            "conflictsCleared",
            "failed",
            "imported",
            "set",
            "skipped",
            "suggestions",
            "truncated",
            "unmatched",
            "unmatchedWithoutShortcut",
            "unreadable"
        ]
    );
    let sorted = |v: &serde_json::Value| {
        let mut n = names(v);
        n.sort();
        n
    };
    assert_eq!(sorted(&v["changed"][0]), ["command", "from", "item", "label", "line", "to"]);
    assert_eq!(sorted(&v["conflictsCleared"][0]), ["command", "label", "shortcut"]);
    assert_eq!(sorted(&v["skipped"][1]), ["command", "item", "keptBy", "line", "reason", "shortcut"]);
    assert_eq!(v["skipped"][1]["keptBy"], json!({"id": "edit.paste", "params": null}));
    assert_eq!(v["skipped"][0]["reason"], "reserved");
    assert_eq!(sorted(&v["unmatched"][0]), ["item", "keys", "line", "reason"]);
    let reasons: Vec<&str> = v["unmatched"].as_array().unwrap().iter().filter_map(|u| u["reason"].as_str()).collect();
    assert_eq!(reasons, ["noMatch", "ambiguous", "context", "tool", "noMatch"]);
    assert_eq!(sorted(&v["unreadable"][0]), ["item", "keys", "line", "reason"]);
    assert_eq!(sorted(&v["suggestions"][0]), ["candidates", "item", "line"]);
    let approximated = ImportReport { approximated: plan(&numpad_set(), &km, &options).report.approximated, ..ImportReport::default() };
    let a = serde_json::to_value(&approximated).unwrap();
    assert_eq!(sorted(&a["approximated"][0]), ["item", "keys", "line", "note"]);
    assert_eq!(a["approximated"][0]["note"], "numpad");
    assert_eq!(v["changed"][0]["from"], json!(["Cmd+W"]));
}
