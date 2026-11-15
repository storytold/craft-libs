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
