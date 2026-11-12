//! The stroke editor's dash fields: typing in the blank after the pattern adds an entry, clearing a later field ends
//! the pattern there, and the first field never clears.

use craft_draw::ui::stroke::{StrokeEditor, StrokeSections, StrokeStyle};
use craft_ui::contract::{Bind, Show as _};
use craft_ui::testing::{Fixture, trial};
use egui::{Key, Modifiers};
use std::cell::RefCell;
use std::rc::Rc;

/// The panel editor over a style with `dashes`, and the style it writes through to.
fn editor(dashes: Vec<f32>) -> (Fixture, Rc<RefCell<StrokeStyle>>) {
    let style = Rc::new(RefCell::new(StrokeStyle { dashes, ..StrokeStyle::default() }));
    let shown = Rc::clone(&style);
    let mut fixture = Fixture::new(move |ui| {
        let mut style = shown.borrow_mut();
        let _ = StrokeEditor::new("Stroke", Bind::bound(&mut *style)).sections(StrokeSections::PANEL).show(ui);
    });
    fixture.settle();
    (fixture, style)
}

/// Replaces the text of the field named `label` with `text` (empty clears it) and commits it with Enter.
fn type_into(fixture: &mut Fixture, label: &str, text: &str) {
    let field = fixture.by_label(label).unwrap_or_else(|| panic!("the editor shows {label}"));
    fixture.click(&field);
    fixture.key(Key::A, Modifiers::COMMAND);
    if text.is_empty() {
        fixture.key(Key::Backspace, Modifiers::NONE);
    } else {
        fixture.type_text(text);
    }
    fixture.key(Key::Enter, Modifiers::NONE);
    fixture.idle(3);
}

#[trial]
fn dash_fields_add_after_the_pattern_and_a_cleared_field_ends_it() {
    let (mut fixture, style) = editor(vec![4.0, 2.0]);
    type_into(&mut fixture, "Dash 2", "3");
    assert_eq!(style.borrow().dashes, [4.0, 2.0, 3.0], "the blank after the pattern adds an entry");
    type_into(&mut fixture, "Gap 1", "");
    assert_eq!(style.borrow().dashes, [4.0], "clearing the second field ends the pattern there");
    type_into(&mut fixture, "Dash 1", "");
    assert_eq!(style.borrow().dashes, [4.0], "the first field never clears");
}
