//! The column bar: a divider drag or an arrow key moves the two columns it divides.

use craft_pages::ui::ColumnBar;
use craft_pages::ui::reading::MAX_COLUMNS;
use craft_ui::contract::{Bind, Show as _};
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
