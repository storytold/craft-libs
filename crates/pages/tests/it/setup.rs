//! The column bar: a divider drag or an arrow key moves the two columns it divides. The Page Setup dialog's Document
//! grid: the counts follow the kind, and a refused OK shows its reason.

use craft_pages::ui::reading::MAX_COLUMNS;
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
    let (mut fixture, widths) = bar(vec![EQUAL; COLUMNS]);
    let divider = fixture.by_label("Column 2 width").expect("the second divider");
    fixture.drag(&divider, Vec2::new(FAR, 0.0));
    let widths = widths.borrow();
    assert!(close(widths.iter().sum::<f64>(), EQUAL * COLUMNS as f64), "the total stays: {widths:?}");
    assert!(close(widths[0], EQUAL), "the column before the divider's pair is untouched: {widths:?}");
    assert!(widths[1] > EQUAL && widths[2] < EQUAL, "the divided pair shares the change: {widths:?}");
    assert!(close(widths[3], EQUAL), "the column past the pair is untouched: {widths:?}");
}

/// A bar over `widths`, which every pass writes back.
fn bar(widths: Vec<f64>) -> (Fixture, Rc<RefCell<Vec<f64>>>) {
    let widths = Rc::new(RefCell::new(widths));
    let seen = Rc::clone(&widths);
    let mut fixture = Fixture::new(move |ui| {
        let mut current = seen.borrow().clone();
        ColumnBar::new("Columns", Bind::bound(&mut current)).show(ui);
        *seen.borrow_mut() = current;
    });
    fixture.settle();
    (fixture, widths)
}

/// With many narrow columns the first and last dividers still move as far as the clamps allow, the total stays and
/// no column reaches zero.
#[trial]
fn a_divider_between_many_narrow_columns_still_moves_and_keeps_the_total() {
    for count in [30, MAX_COLUMNS] {
        let (mut fixture, widths) = bar(vec![EQUAL; count]);
        let total = EQUAL * count as f64;
        for (label, by) in [("Column 1 width".to_owned(), FAR), (format!("Column {} width", count - 1), -FAR)] {
            let divider = fixture.by_label(&label).unwrap_or_else(|| panic!("{count} columns: the divider {label}"));
            let before = widths.borrow().clone();
            fixture.drag(&divider, Vec2::new(by, 0.0));
            let now = widths.borrow();
            assert_ne!(*now, before, "{count} columns: {label} moved");
            assert!(close(now.iter().sum::<f64>(), total), "{count} columns: the total stays after {label}: {now:?}");
            assert!(now.iter().all(|width| *width > 0.0), "{count} columns: every column keeps a width after {label}: {now:?}");
        }
    }
}

/// An arrow moves the focused divider one step toward its side, Shift ten steps; the total stays.
#[trial]
fn an_arrow_moves_the_focused_divider_one_step_and_shift_ten() {
    let (mut fixture, widths) = bar(vec![EQUAL; COLUMNS]);
    let divider = fixture.by_label("Column 2 width").expect("the second divider");
    fixture.click(&divider).settle();
    let before = widths.borrow()[1];
    fixture.key(Key::ArrowRight, Modifiers::NONE).settle();
    let step = widths.borrow()[1] - before;
    assert!(step > 0.0, "Right moves the divider right and widens the column before it: {:?}", widths.borrow());
    fixture.key(Key::ArrowRight, Modifiers::SHIFT).settle();
    let jump = widths.borrow()[1] - before - step;
    assert!(close(jump, 10.0 * step), "Shift moves ten steps: {jump} after a step of {step}");
    let widths = widths.borrow();
    assert!(close(widths.iter().sum::<f64>(), EQUAL * COLUMNS as f64), "the total stays: {widths:?}");
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
    // The dialog is taller than the default room: there the grid section lies below the window and cannot be pressed.
    // The dialog's body scrolls; a room tall enough for every section keeps the grid rows in view, so clicks reach them.
    fixture.room(Vec2::new(1120.0, 1100.0)).settle();
    let disabled = |fixture: &Fixture, label: &str| {
        let field = fixture.by_label(label).unwrap_or_else(|| panic!("the {label} field"));
        fixture.tree().is_disabled_key(field.node)
    };
    let pick = |fixture: &mut Fixture, label: &str| {
        let radio = fixture.by_label(label).unwrap_or_else(|| panic!("the {label} radio"));
        fixture.click(&radio).settle();
    };
    assert!(disabled(&fixture, "Characters per line") && disabled(&fixture, "Lines per page"), "no grid: both counts disabled");
    pick(&mut fixture, "Lines only");
    assert!(disabled(&fixture, "Characters per line") && !disabled(&fixture, "Lines per page"), "lines only: just the lines count");
    pick(&mut fixture, "Lines and characters");
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

/// Page Setup lays its rows out two to a line, as WordCraft's dialog did: Top beside Bottom, Left beside Right, Width
/// beside Height, Portrait beside Landscape, the grid kinds side by side (Text snaps to characters on a second line, at
/// the dialog's wrap width) and the two grid counts on one line, and the section direction on the counts' section.
/// Mirror margins stands at its section's start, as the Headers and footers check boxes do, not in the field column.
/// It then fits the default room without scrolling, Different First Page included.
#[trial]
fn page_setup_rows_stand_two_to_a_line() {
    let start = PageSetup { grid: Some(DocumentGrid::new(GridKind::None, 40, 30)), ..PageSetup::default() };
    let state = Rc::new(RefCell::new(start));
    let seen = Rc::clone(&state);
    let mut fixture = Fixture::new(move |ui| {
        let mut setup = seen.borrow_mut();
        PageSetupDialog::new(Bind::bound(&mut *setup)).layout().direction().opened(Bind::given(true)).show(ui);
    });
    fixture.room(Vec2::new(1440.0, 900.0)).settle();
    let rect = |label: &str| fixture.by_label(label).unwrap_or_else(|| panic!("{label} is shown")).rect;
    let same_line = |a: &str, b: &str| {
        let (a_rect, b_rect) = (rect(a), rect(b));
        assert!((a_rect.center().y - b_rect.center().y).abs() < 1.0 && a_rect.max.x <= b_rect.min.x, "{a} stands before {b} on its line");
    };
    same_line("Top", "Bottom");
    same_line("Left", "Right");
    same_line("Width", "Height");
    same_line("Portrait", "Landscape");
    same_line("No grid", "Lines only");
    same_line("Characters per line", "Lines per page");
    same_line("Left-to-right", "Right-to-left");
    assert!(rect("Top").max.y < rect("Left").min.y, "Left and Right are on the line under Top and Bottom");
    assert!(rect("No grid").max.y <= rect("Text snaps to characters").min.y, "the last grid kind wraps onto a second line");
    let first = rect("Different First Page");
    assert!((rect("Mirror margins").min.x - first.min.x).abs() < 1.0, "Mirror margins stands at its section's start");
    assert!(first.max.y <= 900.0, "the last row is in view without scrolling: {first:?}");
}
