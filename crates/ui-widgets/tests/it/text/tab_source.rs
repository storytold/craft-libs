//! The stop a Tab leaves is the one focus is on when the Tab applies: a focus request the same pass moved the focus to
//! the second field means the Tab left the second field, so the first, which lost the focus to the request, does not
//! report a Tab departure.

use craft_ui::contract::{Bind, Show as _};
use craft_ui::runtime::focus::{FocusTarget, request};
use craft_ui::testing::focus::key_release;
use craft_ui::testing::input::key_press;
use craft_ui::testing::{Fixture, trial};
use craft_ui::text::TextField;
use egui::{Id, Key, Modifiers};
use std::cell::{Cell, RefCell};
use std::rc::Rc;

#[trial]
fn a_pending_focus_request_moves_the_stop_a_tab_leaves() {
    let first = Rc::new(RefCell::new("one".to_owned()));
    let second = Rc::new(RefCell::new("two".to_owned()));
    let second_id = Rc::new(Cell::new(Id::NULL));
    let first_left = Rc::new(Cell::new(None::<Option<bool>>));
    let (held_first, held_second, held_id, held_left) = (first.clone(), second.clone(), second_id.clone(), first_left.clone());
    let mut fixture = Fixture::new(move |ui| {
        let one = TextField::new("First", Bind::bound(&mut *held_first.borrow_mut())).show(ui);
        // The first field lost the focus to a request this pass, which is no Tab departure.
        if let Some(Some(left)) = one.left_by_tab.map(Some) {
            held_left.set(Some(Some(left)));
        }
        let two = TextField::new("Second", Bind::bound(&mut *held_second.borrow_mut())).show(ui);
        held_id.set(two.response.id);
    });
    fixture.settle();
    let first_field = fixture.by_label("First").expect("the first field");
    fixture.click(&first_field).settle();
    assert!(fixture.focused_stop().is_some(), "the first field has the focus");
    // One pass: the request for the second field, then the Tab that leaves it; the first field must not claim the Tab.
    request(fixture.ctx(), FocusTarget::Id(second_id.get()));
    fixture.frame(vec![key_press(Key::Tab, Modifiers::NONE)]);
    fixture.frame(vec![key_release(Key::Tab, Modifiers::NONE)]);
    fixture.frame(Vec::new());
    assert_eq!(first_left.get(), None, "the first field lost the focus to the request, not to the Tab");
    assert!(fixture.by_label("Second").is_some(), "both fields are still shown");
}
