//! The table's model through the dialog: which rows are listed and which count as conflicts.

use std::sync::Arc;

use craft_host::{AppInfo, Host};
use craft_keymap::{Binding, CommandDef, CommandRef, Keymap, Platform, Policy, Scope, Scopes};
use craft_menu::shortcuts::{HoldSetting, Outcome};
use craft_ui::contract::Role;
use craft_ui::testing::trial;
use egui::{Key, Modifiers};

use super::fixture::{self, Rig};

#[trial]
fn hidden_entries_are_not_listed_but_still_conflict() {
    let mut options = fixture::options();
    options.visible = Some(|b| b.command.id != "file.export");
    let mut rig = Rig::new(fixture::keymap(Platform::Mac), options, None);
    assert!(!rig.has("Export…: ⌘E"), "a hidden command is not listed");
    rig.shared.borrow_mut().km.assign(&CommandRef::from("file.save"), Binding::parse("Cmd+E").expect("a chord"), Policy::Keep).expect("assigns");
    rig.pass();
    rig.choose("Filter:", "Conflicts");
    assert!(rig.has("Save: ⌘S") && rig.has("Save: ⌘E"), "the hidden command still clashes with Save");
}

/// Every row that can take another shortcut shows its add button after its chips, in a narrow dialog too: the chips
/// give way to it.
#[trial]
fn every_row_that_can_take_a_shortcut_has_its_add_button() {
    let mut rig = Rig::new(fixture::keymap(Platform::Mac), fixture::options(), None);
    rig.fixture.room(egui::vec2(640.0, 600.0));
    rig.fixture.settle();
    for label in ["Save", "Save As…", "Export…", "Bring to Front", "Loop"] {
        let add = format!("Add a shortcut to {label}");
        assert!(rig.has(&add), "{label:?} has no add button; buttons {:?}", rig.names_with(craft_ui::contract::Role::BUTTON, "Add a shortcut"));
    }
}

/// A query typed in capitals finds the row: the search folds the dotted capital İ to `i`, where a lowercase fold
/// gives `i` plus a combining dot and finds nothing. The English alias finds it too.
#[trial]
fn the_search_folds_dotted_capitals() {
    let (i18n, _) = craft_i18n::I18n::from_owned([("az".to_owned(), String::new(), String::new())], &["az"]);
    let lang = i18n.from_code("az").expect("az is offered");
    let host = Arc::new(Host::builder(AppInfo { id: "trial", name: "Trial", version: "0" }).i18n(Arc::new(i18n)).lang(lang).build());
    let path: Vec<String> = ["Image", "Image Size…"].iter().map(|s| (*s).to_owned()).collect();
    let size = CommandDef::new("image.size", "Şəkil ölçüsü…", Scope::app()).with_path(path).with_alias("Image Size");
    let other = CommandDef::new("file.save", "Saxla", Scope::app());
    let listed = |query: &str| {
        let keymap = Keymap::new(vec![size.clone(), other.clone()], Scopes::new(), Platform::Mac);
        let mut rig = Rig::new(keymap, fixture::options(), None);
        rig.fixture.host(host.clone());
        rig.pass();
        let field = rig.find("Search:");
        rig.fixture.click(&field);
        rig.fixture.type_text(query);
        rig.fixture.idle(2);
        (!rig.names_with(Role::ROW, "Şəkil ölçüsü").is_empty(), !rig.names_with(Role::ROW, "Saxla").is_empty())
    };
    assert_eq!(listed("ŞƏKİL ÖLÇÜSÜ"), (true, false), "capitals with the dotted İ");
    assert_eq!(listed("IMAGE SIZE"), (true, false), "the English alias");
}

/// Types `query` into Search and returns the rows whose names contain `part`.
fn rows_for(rig: &mut Rig, query: &str, part: &str) -> Vec<String> {
    let field = rig.find("Search:");
    rig.fixture.click(&field);
    rig.fixture.key(egui::Key::A, egui::Modifiers::COMMAND);
    rig.fixture.type_text(query);
    rig.fixture.idle(2);
    let field = rig.find("Search:");
    assert_eq!(rig.fixture.tree().value_of(field.node), Some(query), "Search holds only the new query");
    rig.names_with(Role::ROW, part)
}

/// The Match Case box, if the dialog shows one.
fn match_case_box(rig: &Rig) -> Option<craft_ui::testing::Target> {
    let tree = rig.fixture.tree();
    rig.fixture.by_role(Role::CHECK_BOX).into_iter().find(|node| tree.name_of(node.node) == Some("Match Case"))
}

/// Without `Options::match_case` there is no Match Case box, and the search ignores case.
#[trial]
fn match_case_is_not_offered_by_default() {
    let mut rig = Rig::mac();
    assert!(match_case_box(&rig).is_none(), "Match Case is opt-in");
    assert_eq!(rows_for(&mut rig, "save", "Save").len(), 2, "Save and Save As… match in any case");
}

/// Offered, Match Case starts off and the search ignores case; ticked, a name must contain the query with its exact case.
#[trial]
fn ticking_match_case_needs_the_exact_case() {
    let mut rig = Rig::tuned(Platform::Mac, |o| o.match_case = true);
    let check = match_case_box(&rig).expect("the dialog offers Match Case");
    assert_eq!(rig.fixture.tree().toggled_of(check.node), Some(craft_behavior::Mixed::Same(false)), "Match Case starts off");
    assert_eq!(rows_for(&mut rig, "save", "Save").len(), 2, "off, the search ignores case");
    let check = match_case_box(&rig).expect("the dialog offers Match Case");
    rig.fixture.click(&check);
    rig.fixture.settle();
    let check = match_case_box(&rig).expect("the dialog offers Match Case");
    assert_eq!(rig.fixture.tree().toggled_of(check.node), Some(craft_behavior::Mixed::Same(true)), "the click ticks it");
    let rows = rows_for(&mut rig, "save", "Save");
    assert!(rows.is_empty(), "\"save\" is written with a capital S in both names: {rows:?}");
    assert_eq!(rows_for(&mut rig, "Save", "Save").len(), 2, "\"Save\" finds Save and Save As…");
}

/// Ticked, only the command's name counts: its id, menu path and shortcut no longer find it.
#[trial]
fn match_case_matches_command_names_only() {
    let mut rig = Rig::tuned(Platform::Mac, |o| o.match_case = true);
    for query in ["file.export", "File", "Cmd+E"] {
        assert_eq!(rows_for(&mut rig, query, "Export").len(), 1, "off, {query:?} finds Export…");
    }
    let check = match_case_box(&rig).expect("the dialog offers Match Case");
    rig.fixture.click(&check);
    rig.fixture.settle();
    for query in ["file.export", "File", "Cmd+E"] {
        assert!(rows_for(&mut rig, query, "Export").is_empty(), "{query:?} is not in Export…'s name");
    }
    assert_eq!(rows_for(&mut rig, "Export", "Export").len(), 1);
}

/// Ticked, a query the search folds away entirely (a zero-width space) still needs a name that holds it, so it lists
/// nothing rather than every command.
#[trial]
fn with_match_case_a_query_that_folds_away_lists_nothing() {
    let mut rig = Rig::tuned(Platform::Mac, |o| o.match_case = true);
    assert_eq!(rows_for(&mut rig, "\u{200B}", "Save").len(), 2, "off, a folded-away query lists every command");
    let check = match_case_box(&rig).expect("the dialog offers Match Case");
    rig.fixture.click(&check);
    rig.fixture.settle();
    let rows = rows_for(&mut rig, "\u{200B}", "Save");
    assert!(rows.is_empty(), "no name holds a zero-width space: {rows:?}");
}

/// The name of the control that holds the keyboard focus.
fn focused(rig: &Rig) -> Option<String> {
    let tree = rig.fixture.tree();
    tree.name_of(craft_ui::testing::access::focused_key(tree)).map(str::to_owned)
}

/// Presses Tab until the control named `name` holds the focus.
fn tab_to(rig: &mut Rig, name: &str) {
    for _ in 0..40 {
        if focused(rig).as_deref() == Some(name) {
            return;
        }
        rig.press(Modifiers::NONE, Key::Tab);
    }
    panic!("Tab never reached {name:?}; the focus is on {:?}", focused(rig));
}

/// By keyboard alone, with Match Case ticked: a chord recorded by Search by key finds the command bound to it, and a
/// shortcut typed into Search still does not.
#[trial]
fn with_match_case_ticked_search_by_key_still_finds_the_binding() {
    let mut rig = Rig::tuned(Platform::Mac, |o| o.match_case = true);
    tab_to(&mut rig, "Match Case");
    rig.press(Modifiers::NONE, Key::Space);
    let check = match_case_box(&rig).expect("the dialog offers Match Case");
    assert_eq!(rig.fixture.tree().toggled_of(check.node), Some(craft_behavior::Mixed::Same(true)), "Space ticks Match Case");

    tab_to(&mut rig, "Search by key");
    rig.press(Modifiers::NONE, Key::Space);
    rig.press(Modifiers::COMMAND, Key::E);
    let rows = rig.names_with(Role::ROW, "Export");
    assert_eq!(rows.len(), 1, "the recorded chord finds Export…: {rows:?}");
    let check = match_case_box(&rig).expect("the dialog offers Match Case");
    assert_eq!(rig.fixture.tree().toggled_of(check.node), Some(craft_behavior::Mixed::Same(true)), "Match Case stays ticked");

    // Typing the recorded text over itself makes it a typed query.
    tab_to(&mut rig, "Search:");
    let recorded = rig.fixture.tree().value_of(rig.find("Search:").node).expect("Search holds the chord").to_owned();
    rig.press(Modifiers::COMMAND, Key::A);
    rig.fixture.type_text(&recorded);
    rig.fixture.idle(2);
    assert_eq!(rig.fixture.tree().value_of(rig.find("Search:").node), Some(recorded.as_str()), "the text is unchanged");
    let rows = rig.names_with(Role::ROW, "Export");
    assert!(rows.is_empty(), "the same text typed is not in Export…'s name: {rows:?}");

    rig.press(Modifiers::COMMAND, Key::A);
    rig.fixture.type_text("Cmd+E");
    rig.fixture.idle(2);
    let rows = rig.names_with(Role::ROW, "Export");
    assert!(rows.is_empty(), "a typed shortcut is not in Export…'s name: {rows:?}");
}

/// With holds on, Summarize… writes a hold binding in its command's line as its trigger text; with holds off the
/// summary leaves it out.
#[trial]
fn the_summary_lists_hold_bindings() {
    let mut km = fixture::keymap(Platform::Mac);
    km.set_holds(true);
    km.assign(&CommandRef::from("file.save"), Binding::parse("Hold K").expect("a hold"), Policy::Keep).expect("assigns");
    let mut rig = Rig::with_holds(km, fixture::options(), HoldSetting::new(true, 0.5, 0.2..=2.0));
    let summarize = |rig: &mut Rig| {
        rig.click("Summarize…");
        rig.outcomes()
            .into_iter()
            .rev()
            .find_map(|o| match o {
                Outcome::Summary { text, html, .. } => Some((text, html)),
                _ => None,
            })
            .expect("a summary")
    };

    let (text, html) = summarize(&mut rig);
    assert!(text.lines().any(|line| line.contains("Save") && line.contains("Hold K")), "Save's line has its hold: {text}");
    assert!(html.contains("Hold K"), "the HTML has the hold: {html}");

    rig.click("Hold Shortcuts");
    let (text, html) = summarize(&mut rig);
    assert!(!text.contains("Hold K") && !html.contains("Hold K"), "with holds off the summary has no hold: {text}");
}

/// The summary written without a `Ui` (a command's file export) is the one Summarize… writes under the same host.
#[trial]
fn the_summary_without_a_ui_is_the_one_summarize_writes() {
    let host = Arc::new(Host::builder(AppInfo { id: "trial", name: "Trial", version: "0" }).build());
    let km = fixture::keymap(Platform::Mac);
    let mut rig = Rig::new(km.clone(), fixture::options(), None);
    rig.fixture.host(host.clone());
    rig.pass();
    rig.click("Summarize…");
    let text = rig
        .outcomes()
        .into_iter()
        .rev()
        .find_map(|o| match o {
            Outcome::Summary { text, .. } => Some(text),
            _ => None,
        })
        .expect("a summary");
    let (title, _) = text.split_once('\n').expect("a title line");
    assert_eq!(craft_menu::shortcuts::summary::text_for(&km, &host, title), text);
}
