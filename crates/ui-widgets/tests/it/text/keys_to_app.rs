//! `FrameInput::keys_to_app`: a window whose keys the app holds for its own prompt routes none of them to the focused
//! field, and leaves them in egui's input for the app.

use craft_ui::contract::{Bind, Props as _, Show as _};
use craft_ui::testing::{Fixture, trial};
use craft_ui::text::TextField;
use egui::{Event, Key, Modifiers, ViewportId};
use std::cell::{Cell, RefCell};
use std::rc::Rc;

/// An Escape while the app holds the window's keys reaches the app's code and does not cancel the field's typing; once
/// the app gives the keys back, Escape cancels it as before.
#[trial]
fn escape_held_for_the_app_leaves_the_fields_typing_alone() {
    let text = Rc::new(RefCell::new(String::new()));
    let (held, covered, escaped) = (text.clone(), Rc::new(Cell::new(false)), Rc::new(Cell::new(0u32)));
    let (cover, seen) = (covered.clone(), escaped.clone());
    let mut fixture = Fixture::new(move |ui| {
        if cover.get() {
            // The app's prompt over the field reads the keys from egui's input, and the field shows behind it, disabled.
            let escapes = ui
                .ctx()
                .input(|input| input.events.iter().filter(|event| matches!(event, Event::Key { key: Key::Escape, pressed: true, .. })).count());
            seen.set(seen.get() + escapes as u32);
        }
        let mut now = held.borrow_mut();
        TextField::new("Name", Bind::bound(&mut *now)).enabled(!cover.get()).show(ui);
    });
    fixture.settle();
    let field = fixture.by_label("Name").expect("the field");
    fixture.click(&field).settle();
    fixture.type_text("Only a draft").settle();
    assert_eq!(*text.borrow(), "Only a draft");
    covered.set(true);
    fixture.keys_to_app(Some(ViewportId::ROOT));
    fixture.key(Key::Escape, Modifiers::NONE).settle();
    assert_eq!(*text.borrow(), "Only a draft", "the held Escape does not cancel the field's typing");
    assert_eq!(escaped.get(), 1, "the app's code reads the Escape from egui's input");
    covered.set(false);
    fixture.keys_to_app(None);
    fixture.settle();
    fixture.click(&field).settle();
    fixture.type_text("!").settle();
    fixture.key(Key::Escape, Modifiers::NONE).settle();
    assert_ne!(*text.borrow(), "Only a draft!", "with the keys back, Escape cancels the field's typing");
}
