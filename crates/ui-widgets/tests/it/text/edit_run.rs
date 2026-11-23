//! A text field's typing run across focus changes: switching to another app carries the run on; focus that arrives
//! between passes selects a select-on-focus field's text before the next pass's typing; a pointer that brings focus
//! keeps the caret or range it made.

use craft_behavior::{CancelReason, EditEvent};
use craft_ui::contract::{Bind, Show as _};
use craft_ui::runtime::focus::{self, FocusTarget};
use craft_ui::testing::{Fixture, input, pointer, trial};
use craft_ui::text::TextField;
use egui::{Event, Id, Key, Modifiers, Pos2};
use std::cell::{Cell, RefCell};
use std::rc::Rc;

/// How a pass of the field ended a typing run.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Ended {
    Commit,
    Cancel(CancelReason),
}

/// A text field named "City" bound to `text`, alone in a window.
struct Rig {
    fixture: Fixture,
    text: Rc<RefCell<String>>,
    /// The next pass asks for the field's focus after drawing it, as a dialog does.
    focus_after: Rc<Cell<bool>>,
    field: Rc<Cell<Id>>,
    ended: Rc<RefCell<Vec<Ended>>>,
}

fn rig(text: &str, select_on_focus: bool) -> Rig {
    let text = Rc::new(RefCell::new(text.to_owned()));
    let focus_after = Rc::new(Cell::new(false));
    let field = Rc::new(Cell::new(Id::NULL));
    let ended = Rc::new(RefCell::new(Vec::new()));
    let (held, asks, id, log) = (text.clone(), focus_after.clone(), field.clone(), ended.clone());
    let mut fixture = Fixture::new(move |ui| {
        let mut text = held.borrow_mut();
        let mut field = TextField::new("City", Bind::bound(&mut *text));
        if select_on_focus {
            field = field.select_on_focus();
        }
        let out = field.show(ui);
        id.set(out.response.id);
        log.borrow_mut().extend(out.events.iter().filter_map(|event| match event {
            EditEvent::Commit { .. } => Some(Ended::Commit),
            EditEvent::Cancel { reason, .. } => Some(Ended::Cancel(*reason)),
            _ => None,
        }));
        if asks.take() {
            focus::request(ui.ctx(), FocusTarget::Id(out.response.id));
        }
    });
    fixture.settle();
    Rig { fixture, text, focus_after, field, ended }
}

impl Rig {
    fn text(&self) -> String {
        self.text.borrow().clone()
    }

    fn ended(&self) -> Vec<Ended> {
        self.ended.borrow().clone()
    }

    fn focused(&self) -> bool {
        self.fixture.focused_stop().map(|(id, _)| id) == Some(self.field.get())
    }

    fn at(&self) -> Pos2 {
        self.fixture.by_label("City").expect("the field").center()
    }

    /// A click in the middle of the field, well after any other: two clicks never make a double click.
    fn click(&mut self) {
        self.fixture.advance(1.0);
        let at = self.at();
        self.fixture.click_at(at);
        self.fixture.settle();
    }

    fn select_all(&mut self) {
        self.fixture.key(Key::A, Modifiers::COMMAND);
        self.fixture.settle();
    }

    fn type_text(&mut self, text: &str) {
        self.fixture.type_text(text);
        self.fixture.settle();
    }

    fn escape(&mut self) {
        self.fixture.key(Key::Escape, Modifiers::NONE);
        self.fixture.settle();
    }

    /// Clicks into the field and retypes its whole text as `text`.
    fn retype(&mut self, text: &str) {
        self.click();
        self.select_all();
        self.type_text(text);
    }
}

/// Switching to another app and back is no focus move: the field keeps the focus and its run, the selection it had
/// carries on, and nothing commits or cancels; Escape afterwards still gives back the text from before the run.
#[trial]
fn switching_apps_carries_on_with_the_typing_run() {
    let mut rig = rig("Lisbon", false);
    rig.retype("Rome");
    rig.select_all();
    rig.fixture.frame(vec![pointer::window_blurred()]);
    rig.fixture.idle(4);
    assert!(rig.focused(), "the field keeps the focus while the window is in the background");
    assert_eq!(rig.ended(), [], "the window losing focus ends nothing");
    rig.fixture.frame(vec![Event::WindowFocused(true)]);
    rig.fixture.settle();
    assert!(rig.focused(), "the field has the focus when the window is back");
    assert_eq!(rig.ended(), [], "the window coming back ends nothing");
    rig.type_text("Porto");
    assert_eq!(rig.text(), "Porto", "the selection carried on: typing replaced it");
    rig.escape();
    assert_eq!(rig.text(), "Lisbon", "Escape gives back the text from before the run");
    assert_eq!(rig.ended(), [Ended::Cancel(CancelReason::Escape)]);
}

/// A dialog asks for the field's focus after drawing it, so the focus arrives at the end of the pass: the field selects
/// its text before the next pass's input, and text typed in that pass replaces it.
#[trial]
fn text_typed_in_the_pass_focus_arrives_replaces_a_select_on_focus_text() {
    let mut rig = rig("Craft Library", true);
    rig.focus_after.set(true);
    rig.fixture.frame(Vec::new());
    assert!(rig.focused(), "the request moved the focus to the field for the next pass");
    rig.fixture.frame(vec![input::text("Fresh")]);
    rig.fixture.settle();
    assert_eq!(rig.text(), "Fresh", "the first typed text replaced the selected text");
    rig.escape();
    assert_eq!(rig.text(), "Craft Library", "Escape gives back the text from before the run");
    assert_eq!(rig.ended(), [Ended::Cancel(CancelReason::Escape)]);
}

/// A plain click into a select-on-focus field selects its whole text, as a browser's address bar does.
#[trial]
fn a_click_into_a_select_on_focus_field_selects_its_text() {
    let mut rig = rig("Lisbon", true);
    rig.click();
    rig.type_text("Kyoto");
    assert_eq!(rig.text(), "Kyoto");
}

/// A drag into a select-on-focus field selects what the drag covers: the focus the press brought does not replace the
/// pointer's range with the whole text.
#[trial]
fn a_drag_into_a_select_on_focus_field_selects_what_it_covers() {
    let mut rig = rig("Lisbon Portugal Europe Atlantic Ocean", true);
    let field = rig.fixture.by_label("City").expect("the field").rect;
    let (from, to) = (field.center(), egui::pos2(field.right() + 40.0, field.center().y));
    rig.fixture.drag_at(from, to - from);
    rig.fixture.settle();
    assert!(rig.focused(), "the press focused the field");
    rig.type_text("X");
    let text = rig.text();
    assert!(text.starts_with("Lisbon") && text.ends_with('X') && text != "X", "the drag's range was replaced: {text:?}");
}

