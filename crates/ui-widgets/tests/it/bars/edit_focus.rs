//! A toolbar's Undo and Redo act in the focused text field while it has a step for them, and otherwise go on to the
//! app's command, whose state then enables the button; the press keeps the focus in the field either way. A menu's
//! edit commands act in the focused field too.

use craft_ui::bars::{BarItem, BarItems};
use craft_ui::contract::{Bind, Props as _, Show as _};
use craft_ui::display::Text;
use craft_ui::runtime::commands::{self, CommandRef, CommandSet as _, CommandState, EditCommands, FireSource};
use craft_ui::runtime::frame;
use craft_ui::testing::{Fixture, trial};
use craft_ui::text::TextField;
use egui::{Id, Key, Modifiers, Vec2};
use std::cell::{Cell, RefCell};
use std::rc::Rc;

/// The label the command's definition gives its toolbar button.
fn label_of(command: EditCommands) -> String {
    let id = CommandRef::from(command).id;
    let defs = EditCommands::defs().expect("the edit commands define");
    defs.into_iter().find(|def| def.command.id == id).map(|def| def.label).expect("the edit command has a definition")
}

/// A title, then a text field holding "hello" over a toolbar with Undo and Redo; the field is the first stop, so Tab
/// focuses it.
struct Rig {
    fixture: Fixture,
    text: Rc<RefCell<String>>,
    field: Rc<Cell<Option<Id>>>,
    /// A menu pick the next frame fires, and whether in the UI (after the title, before the field, as the window's own
    /// menu bar runs) or before it (as a host dispatches a system menu before the app's UI).
    menu: Rc<Cell<Option<(EditCommands, bool)>>>,
}

/// The rig with the field focused and the app's Undo and Redo in `app_can_undo`'s state.
fn rig(app_can_undo: bool) -> Rig {
    let text = Rc::new(RefCell::new("hello".to_owned()));
    let field = Rc::new(Cell::new(None));
    let menu = Rc::new(Cell::new(None));
    let (held, id, pick) = (text.clone(), field.clone(), menu.clone());
    let mut fixture = Fixture::new(move |ui| {
        let (pick, ctx) = (pick.take(), ui.ctx().clone());
        let fire = |in_ui| {
            if let Some((command, _)) = pick.filter(|&(_, at)| at == in_ui) {
                commands::fire(&ctx, command, FireSource::Menu);
            }
        };
        fire(false);
        Text::new("Title").show(ui);
        fire(true);
        id.set(Some(TextField::new("Name", Bind::bound(&mut *held.borrow_mut())).id_salt("name").show(ui).response.id));
        BarItems::new(vec![BarItem::new(EditCommands::Undo), BarItem::new(EditCommands::Redo)]).show(ui);
    });
    for command in [EditCommands::Undo, EditCommands::Redo] {
        fixture.command_state(command, CommandState::new().enabled(app_can_undo));
    }
    fixture.room(Vec2::new(600.0, 200.0)).settle();
    fixture.key(Key::Tab, Modifiers::NONE);
    fixture.settle();
    let rig = Rig { fixture, text, field, menu };
    assert!(rig.focus_in_the_field(), "Tab focuses the field");
    rig
}

impl Rig {
    fn focus_in_the_field(&self) -> bool {
        self.fixture.focused_stop().map(|(id, _)| id) == self.field.get()
    }

    /// Whether the toolbar shows the button of `command` disabled.
    fn disabled(&self, command: EditCommands) -> bool {
        let label = label_of(command);
        let button = self.fixture.by_label(&label).unwrap_or_else(|| panic!("the toolbar shows {label}"));
        self.fixture.tree().is_disabled_key(button.node)
    }

    /// Presses the toolbar's button of `command` and returns the ids of the app's Undo and Redo that the press fired.
    fn press(&mut self, command: EditCommands) -> Vec<String> {
        frame::take_output(self.fixture.ctx());
        let label = label_of(command);
        let button = self.fixture.by_label(&label).unwrap_or_else(|| panic!("the toolbar shows {label}"));
        self.fixture.click(&button);
        self.fixture.settle();
        let history = [EditCommands::Undo, EditCommands::Redo].map(|command| CommandRef::from(command).id);
        frame::take_output(self.fixture.ctx()).fired.into_iter().map(|fired| fired.command.id).filter(|id| history.contains(id)).collect()
    }
}

/// With typing in the focused field, the toolbar's Undo takes it back in the field and Redo puts it back, even while
/// the app has nothing to undo: the field's step enables the buttons, neither reaches the app, and the field keeps the
/// focus.
#[trial]
fn the_arrows_undo_and_redo_typing_in_the_pane() {
    let mut rig = rig(false);
    assert!(rig.disabled(EditCommands::Undo), "with no typing and nothing for the app to undo, Undo is disabled");
    rig.fixture.type_text("abc");
    // The typing run ends, so one undo takes back all of it.
    rig.fixture.advance(2.0);
    rig.fixture.settle();
    assert_eq!(*rig.text.borrow(), "helloabc", "the typing reaches the field");
    assert!(!rig.disabled(EditCommands::Undo), "the field's step enables the toolbar's Undo");

    assert_eq!(rig.press(EditCommands::Undo), Vec::<String>::new(), "Undo with a step in the field fires nothing of the app's");
    assert_eq!(*rig.text.borrow(), "hello", "the toolbar's Undo takes back the typing in the field");
    assert!(rig.focus_in_the_field(), "the press keeps the focus in the field");
    assert!(!rig.disabled(EditCommands::Redo), "the undone typing enables Redo");

    assert_eq!(rig.press(EditCommands::Redo), Vec::<String>::new(), "Redo with a step in the field fires nothing of the app's");
    assert_eq!(*rig.text.borrow(), "helloabc", "the toolbar's Redo puts the typing back");
    assert!(rig.focus_in_the_field(), "the press keeps the focus in the field");
}

/// With no step in the focused field, Undo is the app's: the app's state enables the toolbar's Undo, a press fires the
/// app's command, and the field's text and focus stay.
#[trial]
fn undo_with_no_step_in_the_field_runs_the_app_command() {
    let mut rig = rig(true);
    assert!(!rig.disabled(EditCommands::Undo), "the app's Undo enables the toolbar's though the field has nothing to undo");
    let undo = CommandRef::from(EditCommands::Undo).id;
    assert_eq!(rig.press(EditCommands::Undo), vec![undo], "the press runs the app's Undo");
    assert_eq!(*rig.text.borrow(), "hello", "the field's text stays");
    assert!(rig.focus_in_the_field(), "the press keeps the focus in the field");

    // With the app's Undo disabled too, nothing answers for it.
    rig.fixture.command_state(EditCommands::Undo, CommandState::new().enabled(false));
    rig.fixture.settle();
    assert!(rig.disabled(EditCommands::Undo), "with neither the field nor the app able to undo, the toolbar's Undo is disabled");
}

/// A system menu's Select All, then its Copy, each picked before the frame's UI, select the field's text and copy it
/// in their own frames; a menu pick in the UI, though before the field, copies in the next frame. Guards against a
/// system menu edit that waited a pass, against a field whose recorded edits kept Copy disabled after a Select All of
/// the same pass (which dropped the Copy), and against an in-window menu's edit taken a frame earlier than before or
/// lost.
#[trial]
fn a_menus_select_all_then_copy_copy_the_fields_text() {
    let mut rig = rig(false);
    rig.fixture.key(Key::End, Modifiers::NONE).settle();
    rig.menu.set(Some((EditCommands::SelectAll, false)));
    rig.fixture.frame(Vec::new());
    rig.menu.set(Some((EditCommands::Copy, false)));
    rig.fixture.frame(Vec::new());
    assert_eq!(rig.fixture.copied(), ["hello"], "the system menu's Copy copies the selected text in its own frame");
    assert!(rig.focus_in_the_field(), "the field keeps the focus");

    rig.menu.set(Some((EditCommands::Copy, true)));
    rig.fixture.frame(Vec::new());
    assert!(rig.fixture.copied().is_empty(), "the in-window menu's Copy waits for the next frame");
    rig.fixture.frame(Vec::new());
    assert_eq!(rig.fixture.copied(), ["hello"], "the in-window menu's Copy copies in the next frame");
}
