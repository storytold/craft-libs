//! Import of synthetic shortcut sets written for these tests.

use std::collections::BTreeMap;

use craft_keymap::chord::foreign::{Flavor, ForeignError};
use craft_keymap::import::{FailedChange, MAX_IMPORT_ENTRIES, UnmatchedReason, kys};
use craft_keymap::{
    Binding, CommandDef, CommandKind, CommandRef, ContextRule, ForeignEntry, ForeignKey, ForeignSet, ImportOptions, ImportReport, Keymap,
    MAX_COMMANDS, NotRead, Overrides, Platform, Scope, Scopes, plan, reserved,
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
    remembered.insert(ForeignEntry::new(Some("Scripts"), ["Frobnicate"]).remember_key(), Some(CommandRef::new("scripts.run")));
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

/// A Mac keymap of `catalog` with no reserved chords, for sets read with [`kys::read`].
fn mac_keymap(catalog: Vec<CommandDef>) -> Keymap {
    Keymap::new(catalog, Scopes::new(), Platform::Mac)
}

/// A `.kys` set of `rows`.
fn kys_set(rows: &str) -> ForeignSet {
    kys::read(&format!("<photoshop-keyboard-shortcuts version=\"4\">{rows}</photoshop-keyboard-shortcuts>")).unwrap_or_else(|e| panic!("{e}"))
}

/// The command holds exactly the bindings `texts`, in any order.
fn has_all(km: &Keymap, id: &str, texts: &[&str]) {
    let got = km.bindings(&CommandRef::new(id));
    let want = bindings(texts);
    assert!(got.len() == want.len() && want.iter().all(|w| got.contains(w)), "{id}: {got:?}, wanted {want:?}");
}

fn tool(id: &str, label: &str, keys: &[&str]) -> CommandDef {
    command(id, label, &[], keys).with_kind(CommandKind::Tool { group: None })
}

#[test]
fn a_shared_label_goes_to_the_command_whose_default_key_the_row_carries() {
    // Photoshop names a row by its label alone; both commands are `Levels…` and both have a
    // default, so only the row's keys tell them apart.
    let catalog = vec![command("image.levels", "Levels…", &[], &["Cmd+L"]), command("layer.newLevels", "Levels…", &[], &["Cmd+Alt+L"])];
    let mut km = mac_keymap(catalog);
    let set = kys_set(
        "<command name=\"Levels...\"><shortcut>Cmd+L</shortcut><shortcut>F9</shortcut></command>\
         <command name=\"Levels...\"><shortcut>Opt+Cmd+L</shortcut><shortcut>F10</shortcut></command>\
         <command name=\"Levels...\"><shortcut>F11</shortcut></command>",
    );
    let r = plan(&set, &km, &ImportOptions::default()).apply(&mut km);
    has_all(&km, "image.levels", &["Cmd+L", "F9"]);
    has_all(&km, "layer.newLevels", &["Cmd+Alt+L", "F10"]);
    let unmatched: Vec<(&str, &str, UnmatchedReason)> = r.unmatched.iter().map(|u| (u.item.as_str(), u.keys.as_str(), u.reason)).collect();
    assert_eq!(unmatched, [("Levels...", "F11", UnmatchedReason::Ambiguous)], "a row carrying neither default stays ambiguous");
}

#[test]
fn a_row_with_a_default_and_a_function_key_keeps_both() {
    let mut km = mac_keymap(vec![command("edit.copy", "Copy", &["Edit", "Copy"], &["Cmd+C"])]);
    let set = kys_set("<command name=\"Copy\"><shortcut>F3</shortcut><shortcut>Cmd+C</shortcut></command>");
    let r = plan(&set, &km, &ImportOptions::default()).apply(&mut km);
    has_all(&km, "edit.copy", &["Cmd+C", "F3"]);
    assert!(r.unmatched.is_empty() && r.unreadable.is_empty() && r.failed.is_empty(), "{r:?}");
}

#[test]
fn tool_keys_match_by_name_or_id_and_take_one_letter() {
    let catalog = vec![
        // A command with a tool's label: a tool row never binds it.
        command("edit.move", "Move Tool", &["Edit", "Move Tool"], &[]),
        tool("tool.move", "Move Tool", &["M"]),
        tool("tool.pencil", "Pencil Tool", &[]),
        tool("tool.brush", "Brush Tool", &[]),
        tool("tool.lasso", "Lasso Tool", &["L"]),
    ];
    let mut km = mac_keymap(catalog);
    let set = kys_set(
        "<command name=\"Copy\"><shortcut>Cmd+C</shortcut></command>\
         <tool name=\"Move Tool\">V</tool>\
         <tool name=\"pencil tool\">n</tool>\
         <tool name=\"Renamed\" craft-id=\"tool.brush\">B</tool>\
         <tool name=\"Imaginary Tool\">Q</tool>\
         <tool name=\"Lasso Tool\">Shift+L</tool>",
    );
    let r = plan(&set, &km, &ImportOptions::default()).apply(&mut km);
    // By name, ignoring case, and by id whatever the name says.
    has(&km, "tool.move", &["V"]);
    has(&km, "tool.pencil", &["N"]);
    has(&km, "tool.brush", &["B"]);
    has(&km, "edit.move", &[]);
    // A tool key with a modifier is unreadable and the tool keeps its key.
    has(&km, "tool.lasso", &["L"]);
    let unreadable: Vec<&str> = r.unreadable.iter().map(|u| u.item.as_str()).collect();
    assert_eq!(unreadable, ["Lasso Tool"]);
    let unmatched: Vec<(&str, UnmatchedReason)> = r.unmatched.iter().map(|u| (u.item.as_str(), u.reason)).collect();
    assert_eq!(unmatched, [("Copy", UnmatchedReason::NoMatch), ("Imaginary Tool", UnmatchedReason::NoMatch)]);
}

/// An entry `File > name` whose one key carries undecided codes.
fn coded(name: &str, key: u32, mods: Option<u32>) -> ForeignEntry {
    ForeignEntry::new(Some("File"), [name]).with_key(ForeignKey::code(None, &format!("{key}/{mods:?}"), Some(key), mods))
}

/// The codes of `r` as (code, text).
fn code_texts(r: &ImportReport) -> Vec<(String, Option<String>)> {
    r.codes.iter().map(|c| (c.code.clone(), c.text.clone())).collect()
}

fn pair(code: &str, text: Option<&str>) -> (String, Option<String>) {
    (code.to_string(), text.map(str::to_string))
}

#[test]
fn undecided_codes_decode_by_votes_or_answers() {
    let km = mac_keymap(catalog());

    // Four of five votes agree: Save, four times, votes S and Close votes W; every mods code votes Cmd.
    let mut entries: Vec<ForeignEntry> = (0..4).map(|_| coded("Save", 1, Some(9))).collect();
    entries.push(coded("Close", 1, Some(9)));
    let set = ForeignSet::new(entries);
    let voted = plan(&set, &km, &ImportOptions::default()).report;
    assert_eq!(code_texts(&voted), [pair("key:1", Some("S")), pair("mods:9", Some("Cmd"))]);
    assert_eq!(voted.codes[0].entries, ["File > Save"; 3], "the first three entries that use the code");
    assert!(voted.unreadable.is_empty(), "{:?}", voted.unreadable);

    // Three of four stay undecided and unreadable.
    let mut entries: Vec<ForeignEntry> = (0..3).map(|_| coded("Save", 2, None)).collect();
    entries.push(coded("Close", 2, None));
    let r = plan(&ForeignSet::new(entries), &km, &ImportOptions::default()).report;
    assert!(r.codes.is_empty());
    assert_eq!(r.unreadable.len(), 4);
    assert!(r.unreadable.iter().all(|u| matches!(u.reason, ForeignError::Undecided { key: Some(2), mods: None })));

    // An entry without a matched command votes for nothing.
    let lone = ForeignSet::new(vec![coded("Nothing Like It", 1, Some(9)), coded("Nothing Else", 1, Some(9))]);
    assert!(plan(&lone, &km, &ImportOptions::default()).report.codes.is_empty());

    // An answer wins over the votes.
    let mut answered = ImportOptions::default();
    answered.codes.insert("key:1".into(), Some("D".into()));
    let planned = plan(&set, &km, &answered);
    assert_eq!(code_texts(&planned.report)[0], pair("key:1", Some("D")));
    let save = planned.commands.iter().find(|p| p.command == CommandRef::new("file.save")).expect("Save is in the plan");
    assert!(save.bindings.iter().any(|p| p.binding == b("Cmd+D")), "{:?}", save.bindings);

    // Decoded codes carried as answers stay decoded when too few votes remain.
    let carried = ImportOptions { codes: voted.codes.iter().map(|c| (c.code.clone(), c.text.clone())).collect(), ..ImportOptions::default() };
    let again = plan(&ForeignSet::new(vec![coded("Save", 1, Some(9))]), &km, &carried).report;
    assert_eq!(code_texts(&again), code_texts(&voted));

    // A code answered None skips its keys, whatever the modifier answer is.
    let mut skip = ImportOptions::default();
    skip.codes.insert("key:1".into(), None);
    skip.codes.insert("mods:9".into(), Some("Alt".into()));
    let planned = plan(&set, &km, &skip);
    assert!(planned.commands.is_empty() && planned.report.unreadable.is_empty());
    assert!(code_texts(&planned.report).contains(&pair("key:1", None)));

    // An entry the user answered "No equivalent" is left out and listed.
    let save = || win("File", &["Save"], &[("Default", "Ctrl+Alt+S")]);
    let mut options = windows_options();
    options.remembered.insert(save().remember_key(), None);
    let planned = plan(&ForeignSet::new(vec![save()]), &km, &options);
    let unmatched: Vec<(&str, UnmatchedReason)> = planned.report.unmatched.iter().map(|u| (u.item.as_str(), u.reason)).collect();
    assert_eq!(unmatched, [("File > Save", UnmatchedReason::NoEquivalent)]);
    assert!(planned.commands.is_empty());
}

#[test]
fn the_readers_unread_lines_reach_the_report_in_order() {
    let lines: Vec<NotRead> = (0..=MAX_IMPORT_ENTRIES).map(|n| NotRead::new(Some(n), format!("line {n}"))).collect();
    let set = ForeignSet::new(Vec::new()).with_not_read(lines);
    let r = plan(&set, &mac_keymap(catalog()), &ImportOptions::default()).report;
    assert_eq!(r.not_read.len(), MAX_IMPORT_ENTRIES);
    assert_eq!(r.not_read.first(), Some(&NotRead::new(Some(0), "line 0")));
    assert_eq!(r.not_read.last(), Some(&NotRead::new(Some(MAX_IMPORT_ENTRIES - 1), format!("line {}", MAX_IMPORT_ENTRIES - 1))));
    assert_eq!(r.truncated, 1, "the line past the cap is counted");
}

/// A command's main default key stays its main key, whichever order the set lists its keys in.
#[test]
fn the_main_default_key_among_the_sets_keys_stays_first() {
    let km = keymap(Platform::Mac);
    let set = ForeignSet::new(vec![win("File", &["Save"], &[("Default", "Ctrl+Alt+S"), ("Default", "Ctrl+S")])]);
    let planned = plan(&set, &km, &windows_options());
    let save = planned.commands.iter().find(|p| p.command == CommandRef::new("file.save")).expect("Save is in the plan");
    let keys: Vec<Binding> = save.bindings.iter().map(|p| p.binding.clone()).collect();
    assert_eq!(keys, bindings(&["Cmd+S", "Cmd+Alt+S"]));
}
