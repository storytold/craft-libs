//! The stroke editor's dash fields: typing in the blank after the pattern adds an entry, clearing a later field ends
//! the pattern there, and the first field never clears. Its Caps row on a solid closed path: disabled by default,
//! enabled with `caps_on_closed`.

use craft_draw::ui::stroke::{LineCap, StrokeEditor, StrokeSections, StrokeStyle};
use craft_ui::contract::Bind;
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

/// The panel editor over a solid closed path, with `caps_on_closed` as given, and the style it writes through to.
fn closed_editor(caps_on_closed: bool) -> (Fixture, Rc<RefCell<StrokeStyle>>) {
    let style = Rc::new(RefCell::new(StrokeStyle::default()));
    let shown = Rc::clone(&style);
    let mut fixture = Fixture::new(move |ui| {
        let mut style = shown.borrow_mut();
        let _ = StrokeEditor::new("Stroke", Bind::bound(&mut *style))
            .sections(StrokeSections::PANEL)
            .closed(true)
            .caps_on_closed(caps_on_closed)
            .show(ui);
    });
    fixture.settle();
    (fixture, style)
}

#[trial]
fn caps_on_closed_keeps_the_cap_buttons_enabled_on_a_solid_closed_path() {
    let (mut fixture, style) = closed_editor(false);
    let square = fixture.by_label("Square").expect("the editor shows Square");
    assert!(fixture.tree().is_disabled_key(square.node), "by default Caps is disabled for a solid closed path");
    fixture.click(&square);
    fixture.idle(3);
    assert_ne!(style.borrow().cap, LineCap::Square, "a disabled cap button does not change the cap");

    let (mut fixture, style) = closed_editor(true);
    let square = fixture.by_label("Square").expect("the editor shows Square");
    assert!(!fixture.tree().is_disabled_key(square.node), "with caps_on_closed Caps is enabled");
    fixture.click(&square);
    fixture.idle(3);
    assert_eq!(style.borrow().cap, LineCap::Square, "the cap button sets the cap");
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

#[trial]
fn round_cap_and_round_join_names_lead_to_their_own_actions() {
    use craft_draw::ui::stroke::{LineCap, LineJoin};
    use craft_ui::testing::{access::focused_key, focus::AccessActionKind};

    let (mut fixture, style) = editor(vec![4.0, 2.0]);
    let cap = fixture.by_label("Round Cap").expect("the cap has its own accessible name");
    let join = fixture.by_label("Round Join").expect("the join has its own accessible name");
    assert_ne!(cap.node, join.node, "the names find independent choices");
    fixture.click(&cap).settle();
    assert_eq!(style.borrow().cap, LineCap::Round);
    assert_eq!(style.borrow().join, LineJoin::Miter);
    fixture.access(AccessActionKind::Focus, &join).settle();
    assert_eq!(focused_key(fixture.tree()), join.node);
    fixture.key(Key::Enter, Modifiers::NONE).settle();
    assert_eq!(style.borrow().join, LineJoin::Round);
    assert_eq!(style.borrow().cap, LineCap::Round);
}
