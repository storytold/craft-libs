//! A command picked from a menu acts where focus was before the menu opened: the pick closes the menu, focus returns
//! to the stop that held it, and only then does the command run. Undo picked from the Edit menu, or found through the
//! Help search, undoes the typing in the text field that held focus. Any other activation that closes a menu returns
//! focus the same way; Escape is no activation and leaves focus on the bar title.

use craft_ui::contract::{Bind, Show as _};
use craft_ui::menu::MenuItem;
use craft_ui::menu::bar::{MenuBar, MenuHelp, MenuTitle};
use craft_ui::menu::search::HelpEntry;
use craft_ui::runtime::commands::EditCommands;
use craft_ui::testing::{Fixture, trial};
use craft_ui::text::TextField;
use egui::Id;
use std::cell::{Cell, RefCell};
use std::rc::Rc;

/// A menu bar with Edit (Undo, Redo) and Help (the search over Undo), then a text field holding "hello" that has
/// focus and the typed run "abc".
struct Rig {
    fixture: Fixture,
    text: Rc<RefCell<String>>,
    field: Rc<Cell<Option<Id>>>,
}

fn rig() -> Rig {
    let text = Rc::new(RefCell::new("hello".to_owned()));
    let field = Rc::new(Cell::new(None));
    let shown = (text.clone(), field.clone());
    let mut fixture = Fixture::new(move |ui| {
        let (text, field) = &shown;
        let titles = vec![
            MenuTitle::new("&Edit", |ui| {
                MenuItem::command(EditCommands::Undo).show(ui);
                MenuItem::command(EditCommands::Redo).show(ui);
            }),
            MenuTitle::new("&Help", |ui| {
                MenuItem::new("&About").show(ui);
            }),
        ];
        let help = vec![HelpEntry::new(EditCommands::Undo, "Undo").with_path(["Edit"])];
        MenuBar::new("Main menu", titles).help(MenuHelp::new(1, &help)).show(ui);
        let mut held = text.borrow_mut();
        field.set(Some(TextField::new("Name", Bind::bound(&mut *held)).show(ui).response.id));
    });
    fixture.settle();
    let name = fixture.by_label("Name").expect("the text field");
    fixture.click(&name);
    fixture.type_text("abc");
    // The typing run ends, so one undo takes back all of it.
    fixture.advance(2.0);
    fixture.settle();
    assert_eq!(*text.borrow(), "helloabc", "the typing reaches the field");
    Rig { fixture, text, field }
}

impl Rig {
    fn click(&mut self, label: &str) {
        let target = self.fixture.by_label(label).unwrap_or_else(|| panic!("{label:?} shows"));
        self.fixture.click(&target);
        self.fixture.settle();
    }

    fn assert_undone_in_the_field(&self, path: &str) {
        assert_eq!(*self.text.borrow(), "hello", "{path}: Undo takes back the typing in the field");
        assert_eq!(self.fixture.focused_stop().map(|(id, _)| id), self.field.get(), "{path}: focus is back in the field");
        assert!(self.fixture.by_label("Redo").is_none() && self.fixture.by_label("About").is_none(), "{path}: the pick closes the menu");
    }
}

#[trial]
fn undo_picked_from_the_edit_menu_undoes_in_the_field_that_held_focus() {
    let mut rig = rig();
    rig.click("Edit");
    rig.click("Undo");
    rig.assert_undone_in_the_field("Edit › Undo");
}

#[trial]
fn opening_help_focuses_its_search_and_down_then_enter_runs_the_first_match() {
    let mut rig = rig();
    rig.click("Help");
    rig.fixture.type_text("und");
    rig.fixture.settle();
    assert_eq!(*rig.text.borrow(), "helloabc", "the typed text lands in the search field, not the text field");
    rig.fixture.key(egui::Key::ArrowDown, egui::Modifiers::NONE);
    rig.fixture.key(egui::Key::Enter, egui::Modifiers::NONE);
    rig.fixture.settle();
    rig.assert_undone_in_the_field("Help › und, Down, Enter");
}

#[trial]
fn a_shortcut_pressed_on_the_focused_bar_runs_once_where_focus_was_and_gives_focus_back() {
    let mut rig = rig();
    rig.fixture.key(egui::Key::F10, egui::Modifiers::NONE);
    rig.fixture.settle();
    assert_ne!(rig.fixture.focused_stop().map(|(id, _)| id), rig.field.get(), "F10 takes the focus to the bar");
    rig.fixture.key(egui::Key::Z, egui::Modifiers::COMMAND);
    rig.fixture.settle();
    rig.assert_undone_in_the_field("F10, then the Undo shortcut");
}
