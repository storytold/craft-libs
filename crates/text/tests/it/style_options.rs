//! The style options editor: a row left out of the style takes no input, and its check includes it.

use std::cell::RefCell;
use std::rc::Rc;

use craft_behavior::EditEvent;
use craft_text::ui::character::{CharacterValues, StyleAttribute, StyleOptionValues, StyleOptionsChange, TextStyleOptions};
use craft_ui::testing::{Fixture, trial};
use egui::{Key, Modifiers};

/// The changes the edits carry, in order.
fn changes(edits: &RefCell<Vec<EditEvent<StyleOptionsChange>>>) -> Vec<StyleOptionsChange> {
    edits.borrow().iter().filter_map(|edit| edit.value().cloned()).collect()
}

#[trial]
fn a_row_left_out_of_the_style_takes_no_input_and_its_check_includes_it() {
    let character = CharacterValues { size: Some(12.0), ..Default::default() };
    let values = StyleOptionValues::new("Emphasis", character);
    let edits = Rc::new(RefCell::new(Vec::new()));
    let sink = edits.clone();
    let mut fixture = Fixture::new(move |ui| {
        let out = TextStyleOptions::new("Style Options", &values, &[StyleAttribute::Size], &[], &[]).show(ui);
        sink.borrow_mut().extend(out.edits);
    });
    fixture.idle(10);

    let size = fixture.by_label("Font size").expect("the Size field is shown");
    fixture.click(&size).idle(2);
    fixture.type_text("20");
    fixture.key(Key::Enter, Modifiers::NONE);
    fixture.idle(4);
    assert!(changes(&edits).is_empty(), "a row that is not included takes no input: {:?}", changes(&edits));

    let include = fixture.by_label("Include Font size").expect("the include check is shown");
    fixture.click(&include).idle(4);
    assert_eq!(changes(&edits), vec![StyleOptionsChange::include(StyleAttribute::Size, true)], "the check of a row that is not included includes it");
}
