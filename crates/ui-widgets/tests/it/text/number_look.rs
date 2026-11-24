//! The number field's looks an app sets at its root: the inline look (`NumberLook::Inline`), where the number and its
//! unit are one run of the text part, so editing reaches the unit's text as it reaches the number's; and the measured
//! precision (`NumberPrecision::Measured`).

use craft_ui::contract::{Bind, Props as _, Show as _};
use craft_ui::runtime::env::{LengthUnits, UnitKind};
use craft_ui::testing::{Fixture, trial};
use craft_ui::text::{NumberField, NumberLook, NumberPrecision};
use egui::{Id, Key, Modifiers, Rect};
use std::cell::Cell;
use std::rc::Rc;

/// A length field in points under the inline look: the fixture, the bound value, and the field's id and rect.
struct Inline {
    fixture: Fixture,
    value: Rc<Cell<Option<f64>>>,
    field: Rc<Cell<(Id, Rect)>>,
}

impl Inline {
    fn new() -> Inline {
        let (value, field) = (Rc::new(Cell::new(Some(720.0))), Rc::new(Cell::new((Id::NULL, Rect::NOTHING))));
        let (v, f) = (value.clone(), field.clone());
        let mut fixture = Fixture::new(move |ui| {
            let mut shown = v.get();
            let lengths = LengthUnits::new(UnitKind::Points, UnitKind::Points, 72.0);
            let out = NumberField::length("Width", Bind::bound(&mut shown), lengths).id_salt("width").show(ui);
            v.set(shown);
            f.set((out.response.id, out.response.rect));
        });
        fixture.root_value(NumberLook::Inline);
        fixture.settle();
        Inline { fixture, value, field }
    }

    /// The field's text as the tree reads it, and the runs painted inside the field.
    fn shown(&self) -> (String, Vec<String>) {
        let (id, rect) = self.field.get();
        let text = self.fixture.tree().value(id).unwrap_or_default().to_owned();
        let runs = self.fixture.texts().iter().filter(|run| rect.contains_rect(run.rect)).map(|run| run.text.clone()).collect();
        (text, runs)
    }
}

/// At rest the field paints one run, the number with its unit, from the field's start; the unit is no run of its own.
#[trial]
fn an_inline_field_paints_its_number_and_unit_as_one_run_at_its_start() {
    let inline = Inline::new();
    let (text, runs) = inline.shown();
    let (_, rect) = inline.field.get();
    assert!(text.starts_with("720") && text.len() > "720".len(), "the field's text holds the number and its unit: {text:?}");
    assert_eq!(runs, std::slice::from_ref(&text), "the field paints its text as one run, no unit apart");
    let run = inline.fixture.texts().iter().find(|run| run.text == text).map(|run| run.rect).unwrap_or(Rect::NOTHING);
    assert!(run.center().x < rect.center().x, "the run starts at the field's start: {run:?} in {rect:?}");
}

/// Focus selects the whole run, unit included; typed text replaces it and commits, and the unit comes back with the
/// new number. A caret edit inside the run keeps the unit's text and commits the edited number.
#[trial]
fn an_inline_field_edits_its_whole_run_and_commits_the_number() {
    let mut inline = Inline::new();
    let (before, _) = inline.shown();
    let (id, rect) = inline.field.get();
    let fixture = &mut inline.fixture;
    fixture.click_at(rect.center());
    fixture.key(Key::A, Modifiers::COMMAND);
    let selection = fixture.tree().text_selection(id).map(|(anchor, focus)| anchor.1.abs_diff(focus.1));
    assert_eq!(selection, Some(before.chars().count()), "select all covers the number and its unit");
    fixture.type_text("36");
    fixture.key(Key::Enter, Modifiers::NONE);
    fixture.settle();
    assert_eq!(inline.value.get(), Some(36.0), "the typed number commits");
    let (after, runs) = inline.shown();
    assert_eq!(after.strip_prefix("36"), before.strip_prefix("720"), "the unit follows the new number: {after:?}");
    assert_eq!(runs, std::slice::from_ref(&after), "the field shows the same one run");

    let fixture = &mut inline.fixture;
    fixture.key(Key::Home, Modifiers::NONE);
    fixture.key(Key::Delete, Modifiers::NONE);
    fixture.type_text("4");
    assert_eq!(fixture.tree().value(id).map(|text| text.strip_prefix("46") == after.strip_prefix("36")), Some(true));
    fixture.key(Key::Enter, Modifiers::NONE);
    fixture.settle();
    assert_eq!(inline.value.get(), Some(46.0), "the number edited inside the run commits without its unit");
}

/// At the measured precision a percentage reads whole where the standard precision writes one decimal: the same
/// field shows fewer digits.
#[trial]
fn a_percent_at_the_measured_precision_reads_whole() {
    let number = |precision: Option<NumberPrecision>| {
        let mut fixture = Fixture::new(|ui| {
            NumberField::new("Opacity", Bind::bound(&mut Some(60.0f64))).unit_kind(UnitKind::Percent).range(0.0..=100.0).show(ui);
        });
        if let Some(precision) = precision {
            fixture.root_value(precision);
        }
        fixture.settle();
        fixture.texts().iter().map(|run| run.text.clone()).find(|text| text.starts_with("60")).unwrap_or_default()
    };
    let (standard, measured) = (number(None), number(Some(NumberPrecision::Measured)));
    assert!(standard.starts_with("60") && measured.starts_with("60"), "both show the value: {standard:?} {measured:?}");
    assert!(measured.len() < standard.len() && !measured.contains(['.', ',']), "the measured percent is whole: {measured:?}, standard {standard:?}");
}

/// At the measured precision a length shows more of its decimals than the standard precision, and none past the
/// value's own ("720", not "720.000").
#[trial]
fn a_length_at_the_measured_precision_shows_finer_decimals() {
    let number = |value: f64, precision: Option<NumberPrecision>| {
        let mut fixture = Fixture::new(move |ui| {
            let lengths = LengthUnits::new(UnitKind::Points, UnitKind::Points, 72.0);
            NumberField::length("X", Bind::bound(&mut Some(value)), lengths).show(ui);
        });
        if let Some(precision) = precision {
            fixture.root_value(precision);
        }
        fixture.settle();
        let tree = fixture.tree();
        tree.keys()
            .into_iter()
            .find_map(|key| tree.value_of(key).filter(|text| text.starts_with(&value.trunc().to_string())).map(str::to_owned))
            .unwrap_or_default()
    };
    let (standard, measured) = (number(357.1138, None), number(357.1138, Some(NumberPrecision::Measured)));
    let digits = |text: &str| text.chars().filter(char::is_ascii_digit).count();
    assert!(digits(&measured) > digits(&standard), "the measured length shows more decimals: {measured:?}, standard {standard:?}");
    let whole = number(720.0, Some(NumberPrecision::Measured));
    assert!(!whole.is_empty() && whole == number(720.0, None), "a whole length shows no trailing zeros: {whole:?}");
}
