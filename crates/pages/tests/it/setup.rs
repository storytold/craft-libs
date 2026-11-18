//! The column bar: a divider drag moves the two columns it divides. The Page Setup dialog's Document grid: the counts
//! follow the kind, and a refused OK shows its reason.

use craft_pages::ui::{ColumnBar, PageSetupDialog};
use craft_pages::{DocumentGrid, GridKind, MAX_GRID_COUNT, PageSetup};
use craft_ui::contract::{Bind, Show as _};
use craft_ui::dialog::HeldAnswer;
use craft_ui::runtime::env::Label;
use craft_ui::testing::{Fixture, trial};
use egui::{Key, Modifiers, Vec2};
use std::cell::RefCell;
use std::rc::Rc;

/// A column's width at the start of the column tests, and the number of columns.
const EQUAL: f64 = 1.0;
const COLUMNS: usize = 4;

/// A drag far past the end of the bar, points.
const FAR: f32 = 10_000.0;

fn close(a: f64, b: f64) -> bool {
    (a - b).abs() < 1e-6
}

#[trial]
fn a_divider_moves_only_the_two_columns_it_divides() {
    let widths = Rc::new(RefCell::new(vec![EQUAL; COLUMNS]));
    let seen = Rc::clone(&widths);
    let mut fixture = Fixture::new(move |ui| {
        let mut current = seen.borrow().clone();
        ColumnBar::new("Columns", Bind::bound(&mut current)).show(ui);
        *seen.borrow_mut() = current;
    });
    fixture.settle();
    let divider = fixture.by_label("Column 2 width").expect("the second divider");
    fixture.drag(&divider, Vec2::new(FAR, 0.0));
    let widths = widths.borrow();
    assert!(close(widths.iter().sum::<f64>(), EQUAL * COLUMNS as f64), "the total stays: {widths:?}");
    assert!(close(widths[0], EQUAL), "the column before the divider's pair is untouched: {widths:?}");
    assert!(widths[1] > EQUAL && widths[2] < EQUAL, "the divided pair shares the change: {widths:?}");
    assert!(close(widths[3], EQUAL), "the column past the pair is untouched: {widths:?}");
}

#[trial]
fn the_grid_counts_follow_the_grid_kind_and_a_refused_ok_shows_its_reason() {
    let start = PageSetup { grid: Some(DocumentGrid::new(GridKind::None, 40, 30)), ..PageSetup::default() };
    let state = Rc::new(RefCell::new((start, HeldAnswer::<PageSetup>::new())));
    let seen = Rc::clone(&state);
    let mut fixture = Fixture::new(move |ui| {
        let mut state = seen.borrow_mut();
        let (setup, held) = &mut *state;
        PageSetupDialog::new(Bind::bound(setup)).opened(Bind::given(true)).held(held).show(ui);
    });
    fixture.settle();
    let disabled = |fixture: &Fixture, label: &str| {
        let field = fixture.by_label(label).unwrap_or_else(|| panic!("the {label} field"));
        fixture.tree().is_disabled_key(field.node)
    };
    let pick = |fixture: &mut Fixture, label: &str| {
        let radio = fixture.by_label(label).unwrap_or_else(|| panic!("the {label} radio"));
        fixture.click(&radio).settle();
    };
    assert!(disabled(&fixture, "Characters per line") && disabled(&fixture, "Lines per page"), "no grid: both counts disabled");
    pick(&mut fixture, "Line grid only");
    assert!(disabled(&fixture, "Characters per line") && !disabled(&fixture, "Lines per page"), "lines only: just the lines count");
    pick(&mut fixture, "Line and character grid");
    assert!(!disabled(&fixture, "Characters per line") && !disabled(&fixture, "Lines per page"), "lines and characters: both counts");

    let field = fixture.by_label("Characters per line").expect("the characters field");
    fixture.click(&field).settle();
    fixture.key(Key::A, Modifiers::COMMAND).type_text("5000");
    fixture.key(Key::Tab, Modifiers::NONE).settle();
    let ok = fixture.by_label("OK").expect("the OK button");
    fixture.click(&ok).settle();
    let filed = state.borrow_mut().1.take().expect("OK files the setup in the held answer");
    assert_eq!(filed.grid.map(|grid| grid.chars_per_line), Some(MAX_GRID_COUNT), "a count past the range is held to it");

    let reason = "This document cannot take a grid.";
    assert!(state.borrow_mut().1.answer(Err(Label::from(reason.to_owned()))), "the host's refusal reaches the filed answer");
    fixture.settle();
    assert!(fixture.by_label(reason).is_some(), "the refusal shows in the dialog");
    pick(&mut fixture, "No grid");
    assert!(fixture.by_label(reason).is_none(), "changing a value clears the refusal");
}
