//! The edit menu of a text part: a right click opens Undo, Cut, Copy, Paste, Delete and Select All over the field;
//! the open menu takes Escape and gives the field back its typing run; a picked row and an Edit command fired from
//! elsewhere (a menu bar, the palette) both reach the focused field through the same command route.

use craft_ui::contract::{Bind, Show as _};
use craft_ui::runtime::commands::{EditCommands, FireSource, fire};
use craft_ui::testing::{Fixture, pointer, trial};
use craft_ui::text::TextField;
use egui::{Id, Key, Modifiers};
use std::cell::{Cell, RefCell};
use std::rc::Rc;

/// A text field holding "hello" with the focus and a typing run.
struct Rig {
    fixture: Fixture,
    text: Rc<RefCell<String>>,
    field: Rc<Cell<Id>>,
}

fn rig() -> Rig {
    let text = Rc::new(RefCell::new("hello".to_owned()));
    let field = Rc::new(Cell::new(Id::NULL));
    let (held, id) = (text.clone(), field.clone());
    let mut fixture = Fixture::new(move |ui| {
        id.set(TextField::new("Name", Bind::bound(&mut *held.borrow_mut())).show(ui).response.id);
    });
    fixture.settle();
    let name = fixture.by_label("Name").expect("the text field");
    fixture.click(&name);
    fixture.settle();
    Rig { fixture, text, field }
}

impl Rig {
    fn right_click_in_the_field(&mut self) {
        let at = self.fixture.by_label("Name").expect("the field").center();
        self.fixture.frame(vec![pointer::secondary(at, true)]);
        self.fixture.frame(vec![pointer::secondary(at, false)]);
        self.fixture.settle();
    }

    fn menu_open(&self) -> bool {
        self.fixture.by_label("Select All").is_some()
    }

    /// What the frames that deliver `events`, one per frame, and the idle frames after them put on the clipboard.
    fn copied_through(&mut self, events: Vec<egui::Event>) -> Vec<String> {
        let mut copied = Vec::new();
        for event in events.into_iter().map(Some).chain(std::iter::repeat_n(None, 8)) {
            self.fixture.frame(event.into_iter().collect());
            copied.extend_from_slice(self.fixture.copied());
        }
        copied
    }

    fn focus_in_the_field(&self) -> bool {
        self.fixture.focused_stop().map(|(id, _)| id) == Some(self.field.get())
    }
}

#[trial]
fn a_right_click_opens_the_edit_menu_and_its_cut_cuts_the_selection() {
    let mut rig = rig();
    rig.fixture.key(Key::A, Modifiers::COMMAND);
    rig.fixture.settle();
    rig.right_click_in_the_field();
    assert!(rig.menu_open(), "a right click in the field opens its edit menu");
    for row in ["Undo", "Cut", "Copy", "Paste"] {
        assert!(rig.fixture.by_label(row).is_some(), "the edit menu has {row}");
    }
    let cut = rig.fixture.by_label("Cut").expect("the Cut row").center();
    let copied = rig.copied_through(vec![pointer::moved(cut), pointer::press(cut), pointer::release(cut)]);
    assert_eq!(copied, ["hello"], "Cut puts the selection on the clipboard");
    rig.fixture.settle();
    assert_eq!(*rig.text.borrow(), "", "and takes it out of the field");
    assert!(!rig.menu_open(), "the pick closes the menu");
    assert!(rig.focus_in_the_field(), "the focus is back in the field");
}

#[trial]
fn escape_closes_the_edit_menu_and_the_field_keeps_its_typing_run() {
    let mut rig = rig();
    rig.fixture.type_text("abc");
    rig.fixture.settle();
    assert_eq!(*rig.text.borrow(), "helloabc", "the typing reaches the field");
    rig.right_click_in_the_field();
    assert!(rig.menu_open(), "the edit menu is open");
    rig.fixture.key(Key::Escape, Modifiers::NONE);
    rig.fixture.settle();
    assert!(!rig.menu_open(), "Escape closes the menu");
    assert!(rig.focus_in_the_field(), "the focus is back in the field");
    assert_eq!(*rig.text.borrow(), "helloabc", "the Escape the menu took leaves the draft alone");
    rig.fixture.type_text("d");
    rig.fixture.settle();
    assert_eq!(*rig.text.borrow(), "helloabcd", "typing goes on in the field");
    rig.fixture.key(Key::Escape, Modifiers::NONE);
    rig.fixture.settle();
    assert_eq!(*rig.text.borrow(), "hello", "the field's own Escape still restores the value from before its run");
}

#[trial]
fn an_edit_command_fired_from_elsewhere_reaches_the_focused_field() {
    let mut rig = rig();
    rig.fixture.key(Key::A, Modifiers::COMMAND);
    rig.fixture.settle();
    assert!(fire(rig.fixture.ctx(), EditCommands::Cut, FireSource::Pointer), "Cut runs while the field has a selection");
    assert_eq!(rig.copied_through(Vec::new()), ["hello"], "the fired Cut cuts the field's selection");
    rig.fixture.settle();
    assert_eq!(*rig.text.borrow(), "");
}

/// The Delete row deletes the field's selection: the command reaches the focused text part as its edit, not the app.
#[trial]
fn the_edit_menus_delete_deletes_the_selection_in_the_field() {
    let mut rig = rig();
    rig.fixture.key(Key::A, Modifiers::COMMAND);
    rig.fixture.settle();
    rig.right_click_in_the_field();
    let delete = rig.fixture.by_label("Delete").expect("the Delete row");
    rig.fixture.click(&delete);
    rig.fixture.settle();
    assert_eq!(*rig.text.borrow(), "", "Delete takes the selection out of the field");
    assert!(rig.focus_in_the_field(), "the focus is back in the field");
}
