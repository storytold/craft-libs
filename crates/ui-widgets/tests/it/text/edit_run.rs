//! A text field's typing run across focus changes: switching to another app carries the run on.

use craft_behavior::{CancelReason, EditEvent};
use craft_ui::contract::{Bind, Show as _};
use craft_ui::testing::{Fixture, pointer, trial};
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
    field: Rc<Cell<Id>>,
    ended: Rc<RefCell<Vec<Ended>>>,
}

fn rig(text: &str, select_on_focus: bool) -> Rig {
    let text = Rc::new(RefCell::new(text.to_owned()));
    let field = Rc::new(Cell::new(Id::NULL));
    let ended = Rc::new(RefCell::new(Vec::new()));
    let (held, id, log) = (text.clone(), field.clone(), ended.clone());
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
    });
    fixture.settle();
    Rig { fixture, text, field, ended }
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

