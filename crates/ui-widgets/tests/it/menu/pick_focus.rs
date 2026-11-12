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

/// The command of the scope around the target button in the bar-keys trial.
#[derive(Clone, Copy, Debug, PartialEq, Eq, craft_ui::contract::CommandSet)]
#[command(prefix = "barKeys", scope = "trial.barKeys")]
#[non_exhaustive]
pub enum BarKeysCommand {
    #[command(label = "Toggle", default = "Cmd+Alt+P")]
    Toggle,
}

#[trial]
fn a_shortcut_pressed_on_the_focused_bar_runs_once_where_focus_was_and_gives_focus_back() {
    use craft_ui::contract::ShowContainer as _;
    use craft_ui::runtime::commands::Commands as _;
    let count = Rc::new(Cell::new(0));
    let target = Rc::new(Cell::new(None));
    let (seen, stop) = (count.clone(), target.clone());
    let mut fixture = Fixture::new(move |ui| {
        let titles = vec![MenuTitle::new("&Edit", |ui| {
            MenuItem::command(EditCommands::Undo).show(ui);
        })];
        MenuBar::new("Main menu", titles).show(ui);
        craft_ui::layout_components::Stack::new().command_scope(<BarKeysCommand as craft_ui::contract::CommandSet>::SCOPE).show(ui, |ui, cx| {
            while cx.take_command::<BarKeysCommand>().is_some() {
                seen.set(seen.get() + 1);
            }
            stop.set(Some(craft_ui::buttons::Button::new("Target").show(ui).id));
        });
    });
    fixture.settle();
    craft_ui::runtime::focus::request(fixture.ctx(), craft_ui::runtime::focus::FocusTarget::Id(target.get().expect("the button ran")));
    fixture.settle();
    fixture.key(egui::Key::F10, egui::Modifiers::NONE);
    fixture.settle();
    assert_ne!(fixture.focused_stop().map(|(id, _)| id), target.get(), "F10 takes the focus to the bar");
    fixture.key(egui::Key::P, egui::Modifiers::COMMAND | egui::Modifiers::ALT);
    fixture.settle();
    assert_eq!(count.get(), 1, "the shortcut runs once, in the scope of the stop the bar took the focus from");
    assert_eq!(fixture.focused_stop().map(|(id, _)| id), target.get(), "and the focus is back on that stop");
}

/// A bar whose Help searches the target scope's Toggle, then the Target button inside that scope; returns the count
/// of Toggle runs and the button's id.
fn scoped_rig() -> (Fixture, Rc<Cell<u32>>, Rc<Cell<Option<Id>>>) {
    use craft_ui::contract::ShowContainer as _;
    use craft_ui::runtime::commands::Commands as _;
    let count = Rc::new(Cell::new(0));
    let target = Rc::new(Cell::new(None));
    let (seen, stop) = (count.clone(), target.clone());
    let mut fixture = Fixture::new(move |ui| {
        let titles = vec![MenuTitle::new("&Help", |ui| {
            MenuItem::new("&About").show(ui);
        })];
        let help = vec![HelpEntry::new(BarKeysCommand::Toggle, "Toggle")];
        MenuBar::new("Main menu", titles).help(MenuHelp::new(0, &help)).show(ui);
        craft_ui::layout_components::Stack::new().command_scope(<BarKeysCommand as craft_ui::contract::CommandSet>::SCOPE).show(ui, |ui, cx| {
            while cx.take_command::<BarKeysCommand>().is_some() {
                seen.set(seen.get() + 1);
            }
            stop.set(Some(craft_ui::buttons::Button::new("Target").show(ui).id));
        });
    });
    fixture.settle();
    craft_ui::runtime::focus::request(fixture.ctx(), craft_ui::runtime::focus::FocusTarget::Id(target.get().expect("the button ran")));
    fixture.settle();
    (fixture, count, target)
}

#[trial]
fn opening_help_focuses_its_search_and_down_then_enter_runs_the_first_match() {
    let (mut fixture, count, target) = scoped_rig();
    let help = fixture.by_label("Help").expect("the Help title");
    fixture.click(&help);
    fixture.settle();
    fixture.type_text("tog");
    fixture.settle();
    assert!(fixture.by_label("Toggle").is_some(), "the typed text lands in the search field and finds Toggle");
    fixture.key(egui::Key::ArrowDown, egui::Modifiers::NONE);
    fixture.settle();
    fixture.key(egui::Key::Enter, egui::Modifiers::NONE);
    fixture.settle();
    assert_eq!(count.get(), 1, "Down then Enter runs the first match once");
    assert!(fixture.by_label("About").is_none(), "and the menu closes");
    assert_eq!(fixture.focused_stop().map(|(id, _)| id), target.get(), "with the focus back on the stop that held it");
}

/// The Help search edits as any text field does inside the open menu: Backspace removes a letter, and Left moves the
/// caret, so the next letter lands before the last one.
#[trial]
fn the_help_search_takes_its_editing_keys() {
    let (mut fixture, _, _) = scoped_rig();
    let help = fixture.by_label("Help").expect("the Help title");
    fixture.click(&help);
    fixture.settle();
    fixture.type_text("tog");
    fixture.settle();
    let field_reads = |fixture: &Fixture, text: &str| fixture.texts().iter().any(|run| run.text == text);
    fixture.key(egui::Key::Backspace, egui::Modifiers::NONE);
    fixture.settle();
    assert!(field_reads(&fixture, "to"), "Backspace removes the last letter");
    fixture.key(egui::Key::ArrowLeft, egui::Modifiers::NONE);
    fixture.settle();
    fixture.type_text("x");
    fixture.settle();
    assert!(field_reads(&fixture, "txo"), "Left moves the caret before the last letter");
}
