//! The table's model through the dialog: which rows are listed and which count as conflicts.

use std::sync::Arc;

use craft_host::{AppInfo, Host};
use craft_keymap::{Binding, CommandDef, CommandRef, Keymap, Platform, Policy, Scope, Scopes};
use craft_ui::contract::Role;
use craft_ui::testing::trial;

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
