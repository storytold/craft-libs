//! A Mixed text field shows no text, whatever its codec: typing starts from empty text and the codec parses what is
//! typed, and empty text leaves the value Mixed without an event.

use craft_behavior::Mixed;
use craft_theme::Rgba;
use craft_ui::contract::{Bind, Show as _};
use craft_ui::testing::{Fixture, trial};
use craft_ui::text::HexField;
use egui::{Key, Modifiers};
use std::cell::{Cell, RefCell};
use std::rc::Rc;

type Rig = (Fixture, Rc<RefCell<Mixed<Rgba>>>, Rc<Cell<usize>>);

fn rig() -> Rig {
    let value = Rc::new(RefCell::new(Mixed::<Rgba>::Mixed));
    let events = Rc::new(Cell::new(0));
    let (shown, seen) = (value.clone(), events.clone());
    let mut fixture = Fixture::new(move |ui| {
        let out = HexField::hex("Colour", Bind::bound_mixed(&mut *shown.borrow_mut())).show(ui);
        if !out.events.is_empty() {
            seen.set(seen.get() + 1);
        }
    });
    fixture.settle();
    fixture.key(Key::Tab, Modifiers::NONE);
    (fixture, value, events)
}

/// Enter on text that was emptied sends no event and the value stays Mixed.
#[trial]
fn enter_on_empty_text_leaves_a_mixed_field_mixed() {
    let (mut fixture, value, events) = rig();
    fixture.type_text("f");
    fixture.key(Key::Backspace, Modifiers::NONE);
    fixture.key(Key::Enter, Modifiers::NONE);
    assert_eq!(*value.borrow(), Mixed::Mixed, "empty text is not a colour");
    assert_eq!(events.get(), 0, "no event for empty text");
}
