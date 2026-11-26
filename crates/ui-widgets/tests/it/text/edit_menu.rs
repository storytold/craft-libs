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
    /// A right click well after the click that focused the field: egui counts presses of any button into one click
    /// run, and a second press of a run selects the word under it.
    fn right_click_in_the_field(&mut self) {
        self.fixture.advance(1.0);
        let at = self.fixture.by_label("Name").expect("the field").center();
        self.fixture.frame(vec![pointer::secondary(at, true)]);
        self.fixture.frame(vec![pointer::secondary(at, false)]);
        self.fixture.settle();
    }

    fn menu_open(&self) -> bool {
        self.fixture.by_label("Select All").is_some()
    }

    /// A primary click at `at`, well after any other click.
    fn click_at(&mut self, at: egui::Pos2) {
        self.fixture.advance(1.0);
        self.fixture.click_at(at);
        self.fixture.settle();
    }

    /// A point just inside the field's start edge, in the room before its text.
    fn padding_start(&self) -> egui::Pos2 {
        let field = self.fixture.by_label("Name").expect("the field").rect;
        egui::pos2(field.left() + 2.0, field.center().y)
    }

    fn type_text(&mut self, text: &str) {
        self.fixture.type_text(text);
        self.fixture.settle();
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

/// A click in the field around its text (its padding) places the caret at the nearest character, as a click on the
/// text does: here before the first one.
#[trial]
fn a_click_in_the_room_around_the_text_places_the_caret() {
    let mut rig = rig();
    rig.fixture.key(Key::A, Modifiers::COMMAND);
    rig.fixture.settle();
    let at = rig.padding_start();
    rig.click_at(at);
    rig.type_text("X");
    assert_eq!(*rig.text.borrow(), "Xhello", "the click put the caret before the first character");
}

/// A click outside the edit menu closes it and goes no further, so the field keeps the selection it had; the next
/// click in the field places a caret, here in the room before its text (the LightCraft click-back case).
#[trial]
fn a_click_after_the_edit_menu_closes_places_a_caret() {
    let mut rig = rig();
    rig.fixture.key(Key::A, Modifiers::COMMAND);
    rig.fixture.settle();
    rig.right_click_in_the_field();
    let at = rig.padding_start();
    rig.click_at(at);
    assert!(!rig.menu_open(), "the first click closes the menu");
    assert!(rig.focus_in_the_field(), "and the focus is back in the field");
    rig.click_at(at);
    rig.type_text("X");
    assert_eq!(*rig.text.borrow(), "Xhello", "the second click placed the caret before the first character");
}

/// Return in an edit menu the pointer opened, before the pointer lit a row, runs no row (it ran the first one, Undo,
/// and took back the typing): it closes the menu and the field keeps its typing run, as a native menu does.
#[trial]
fn return_in_the_edit_menu_with_no_row_lit_only_closes_it() {
    let mut rig = rig();
    rig.fixture.type_text("abc");
    rig.fixture.settle();
    rig.right_click_in_the_field();
    assert!(rig.menu_open(), "the edit menu is open");
    rig.fixture.key(Key::Enter, Modifiers::NONE);
    rig.fixture.settle();
    assert!(!rig.menu_open(), "Return closes the menu");
    assert_eq!(*rig.text.borrow(), "helloabc", "and runs none of its rows");
    assert!(rig.focus_in_the_field(), "the focus is back in the field");
    rig.fixture.type_text("d");
    rig.fixture.settle();
    assert_eq!(*rig.text.borrow(), "helloabcd", "typing goes on in the field");
}

/// Return in an edit menu the pointer opened runs the row the pointer lit, not the first row the menu focused.
#[trial]
fn return_in_the_edit_menu_runs_the_row_the_pointer_lit() {
    let mut rig = rig();
    rig.fixture.type_text("abc");
    rig.fixture.settle();
    rig.right_click_in_the_field();
    let select_all = rig.fixture.by_label("Select All").expect("the Select All row").center();
    rig.fixture.frame(vec![pointer::moved(select_all)]);
    rig.fixture.settle();
    rig.fixture.key(Key::Enter, Modifiers::NONE);
    rig.fixture.settle();
    assert!(!rig.menu_open(), "Return closes the menu");
    assert_eq!(*rig.text.borrow(), "helloabc", "Undo, the first row, did not run");
    rig.fixture.type_text("x");
    rig.fixture.settle();
    assert_eq!(*rig.text.borrow(), "x", "Select All ran: typing replaced the whole text");
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

/// Cut, Copy and Delete need a selection; Paste, Undo and Select All do not.
#[trial]
fn the_edit_menu_offers_cut_copy_and_delete_only_with_a_selection() {
    let mut rig = rig();
    let selection_rows = ["Cut", "Copy", "Delete"];
    let always_rows = ["Paste", "Undo", "Select All"];
    let disabled = |rig: &Rig, row: &str| {
        let target = rig.fixture.by_label(row).unwrap_or_else(|| panic!("the edit menu has {row}"));
        rig.fixture.tree().is_disabled_key(target.node)
    };
    rig.right_click_in_the_field();
    assert!(rig.menu_open(), "the edit menu is open");
    for row in selection_rows {
        assert!(disabled(&rig, row), "{row} is disabled while the selection is empty");
    }
    for row in always_rows {
        assert!(!disabled(&rig, row), "{row} stays enabled without a selection");
    }
    rig.fixture.key(Key::Escape, Modifiers::NONE);
    rig.fixture.settle();
    rig.fixture.key(Key::A, Modifiers::COMMAND);
    rig.fixture.settle();
    rig.right_click_in_the_field();
    assert!(rig.menu_open(), "the edit menu opens again");
    for row in selection_rows {
        assert!(!disabled(&rig, row), "{row} is enabled with a selection");
    }
}

/// A Shift-click in the room around the text extends the selection from its anchor, as a Shift-click on the text does,
/// rather than collapsing it at the caret the press places.
#[trial]
fn a_shift_click_in_the_room_around_the_text_keeps_the_anchor() {
    let mut rig = rig();
    // Put the caret at the end of "hello" with a plain click, then extend with Shift into the room before the text.
    let field = rig.fixture.by_label("Name").expect("the field").rect;
    rig.fixture.advance(1.0);
    rig.fixture.click_at(egui::pos2(field.right() - 4.0, field.center().y));
    rig.fixture.settle();
    let at = rig.padding_start();
    rig.fixture.advance(1.0);
    rig.fixture.hold(Modifiers::SHIFT).click_at(at);
    rig.fixture.hold(Modifiers::NONE).settle();
    rig.type_text("X");
    assert_eq!(*rig.text.borrow(), "X", "the Shift-click selected from the end back to the first character");
}
