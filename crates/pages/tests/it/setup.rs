//! The column bar: a divider drag and the columns beyond it.

use craft_pages::ui::ColumnBar;
use craft_ui::contract::{Bind, Show as _};
use craft_ui::testing::{Fixture, trial};
use egui::Vec2;
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
fn a_divider_pushes_the_columns_beyond_it() {
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
    assert!(widths[1] > EQUAL && widths[2] < EQUAL && widths[3] < EQUAL, "the columns beyond give way: {widths:?}");
}
