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

/// A mixed number field that asks for it reads "Mixed" where it otherwise shows the dash.
#[trial]
fn a_mixed_number_field_can_read_mixed() {
    use craft_ui::text::NumberField;
    let mut fixture = Fixture::new(|ui| {
        NumberField::new("Size", Bind::given(Mixed::<Option<f64>>::Mixed)).mixed_word(true).show(ui);
    });
    fixture.settle();
    assert!(
        fixture.texts().iter().any(|run| run.text == "Mixed"),
        "reads Mixed: {:?}",
        fixture.texts().iter().map(|run| run.text.clone()).collect::<Vec<_>>()
    );
}

/// The word takes the room of the unit: a mixed field that reads "Mixed" shows no unit after it.
#[trial]
fn a_mixed_number_field_that_reads_mixed_shows_no_unit() {
    use craft_ui::text::NumberField;
    let mut fixture = Fixture::new(|ui| {
        NumberField::new("Size", Bind::given(Mixed::<Option<f64>>::Mixed)).unit("pt").mixed_word(true).show(ui);
    });
    fixture.settle();
    assert!(!fixture.texts().iter().any(|run| run.text == "pt"), "no unit beside Mixed");
}
