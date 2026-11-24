//! The number field's inline look (`NumberLook::Inline`): the number and its unit are one run of the text part, so
//! editing reaches the unit's text as it reaches the number's.

use craft_ui::contract::{Bind, Props as _, Show as _};
use craft_ui::runtime::env::{LengthUnits, UnitKind};
use craft_ui::testing::{Fixture, trial};
use craft_ui::text::{NumberField, NumberLook};
use egui::{Id, Key, Modifiers, Rect};
use std::cell::Cell;
use std::rc::Rc;

/// A length field in points under the inline look, its value, id and rect.
fn inline_field() -> (Fixture, Rc<Cell<Option<f64>>>, Rc<Cell<Id>>, Rc<Cell<Rect>>) {
    let (value, id, rect) = (Rc::new(Cell::new(Some(720.0))), Rc::new(Cell::new(Id::NULL)), Rc::new(Cell::new(Rect::NOTHING)));
    let (v, i, r) = (value.clone(), id.clone(), rect.clone());
    let mut fixture = Fixture::new(move |ui| {
        let mut shown = v.get();
        let lengths = LengthUnits::new(UnitKind::Points, UnitKind::Points, 72.0);
        let out = NumberField::length("Width", Bind::bound(&mut shown), lengths).id_salt("width").show(ui);
        v.set(shown);
        i.set(out.response.id);
        r.set(out.response.rect);
    });
    fixture.root_value(NumberLook::Inline);
    fixture.settle();
    (fixture, value, id, rect)
}

/// The field's text as the tree reads it, and the runs painted inside the field.
fn shown(fixture: &Fixture, id: Id, rect: Rect) -> (String, Vec<String>) {
    let text = fixture.tree().value(id).unwrap_or_default().to_owned();
    let runs = fixture.texts().iter().filter(|run| rect.contains_rect(run.rect)).map(|run| run.text.clone()).collect();
    (text, runs)
}

/// At rest the field paints one run, the number with its unit, from the field's start; the unit is no run of its own.
#[trial]
fn an_inline_field_paints_its_number_and_unit_as_one_run_at_its_start() {
    let (fixture, _, id, rect) = inline_field();
    let (text, runs) = shown(&fixture, id.get(), rect.get());
    assert!(text.starts_with("720") && text.len() > "720".len(), "the field's text holds the number and its unit: {text:?}");
    assert_eq!(runs, [text.clone()], "the field paints its text as one run, no unit apart");
    let run = fixture.texts().iter().find(|run| run.text == text).map(|run| run.rect).unwrap_or(Rect::NOTHING);
    assert!(run.center().x < rect.get().center().x, "the run starts at the field's start: {run:?} in {:?}", rect.get());
}

/// Focus selects the whole run, unit included; typed text replaces it and commits, and the unit comes back with the
/// new number. A caret edit inside the run keeps the unit's text and commits the edited number.
#[trial]
fn an_inline_field_edits_its_whole_run_and_commits_the_number() {
    let (mut fixture, value, id, rect) = inline_field();
    let (before, _) = shown(&fixture, id.get(), rect.get());
    fixture.click_at(rect.get().center());
    fixture.key(Key::A, Modifiers::COMMAND);
    let selection = fixture.tree().text_selection(id.get()).map(|(anchor, focus)| anchor.1.abs_diff(focus.1));
    assert_eq!(selection, Some(before.chars().count()), "select all covers the number and its unit");
    fixture.type_text("36");
    fixture.key(Key::Enter, Modifiers::NONE);
    fixture.settle();
    assert_eq!(value.get(), Some(36.0), "the typed number commits");
    let (after, runs) = shown(&fixture, id.get(), rect.get());
    assert_eq!(after.strip_prefix("36"), before.strip_prefix("720"), "the unit follows the new number: {after:?}");
    assert_eq!(runs, [after.clone()], "the field shows the same one run");

    fixture.key(Key::Home, Modifiers::NONE);
    fixture.key(Key::Delete, Modifiers::NONE);
    fixture.type_text("4");
    assert_eq!(fixture.tree().value(id.get()).map(|text| text.strip_prefix("46") == after.strip_prefix("36")), Some(true));
    fixture.key(Key::Enter, Modifiers::NONE);
    fixture.settle();
    assert_eq!(value.get(), Some(46.0), "the number edited inside the run commits without its unit");
}
