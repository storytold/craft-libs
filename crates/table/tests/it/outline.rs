//! The outline bands.

use std::cell::RefCell;
use std::collections::BTreeMap;
use std::rc::Rc;

use craft_table::ui::outline::{OutlineAxis, OutlineBand, OutlineGroup, bands};
use craft_table::ui::{GridLayout, LineIndex, SheetGeometry, SheetMetrics};
use craft_ui::contract::Show as _;
use craft_ui::testing::{Fixture, trial};
use egui::Rect;

const LANE: f32 = 16.0;
const SLACK: f32 = 4.0;
const ROW_HEIGHT: f32 = 20.0;

/// A sheet filling `room`, with the bands of outlines `levels` deep.
fn geometry(room: Rect, levels: (u8, u8), frozen: (u32, u32), hidden: &BTreeMap<u32, f32>) -> SheetGeometry {
    let rows = LineIndex::new(hidden, ROW_HEIGHT, 1000);
    let columns = LineIndex::new(&BTreeMap::new(), 80.0, 100);
    let layout = GridLayout::new(room, rows, columns).frozen(frozen.0, frozen.1).bands(bands(levels, true, 1.0, LANE, SLACK));
    SheetGeometry::new(layout, SheetMetrics::new(ROW_HEIGHT, 8.0, 6.0))
}

/// A group whose first line is hidden and whose other lines are frozen keeps its bracket: a vertical line in its
/// lane, apart from the band's edge.
#[trial]
fn a_group_hidden_at_its_start_in_a_frozen_pane_keeps_its_bracket() {
    let axis = OutlineAxis::new(1, vec![OutlineGroup::new(1, 2, 1)]);
    let edge = Rc::new(RefCell::new(0.0_f32));
    let band_edge = edge.clone();
    let mut fixture = Fixture::new(move |ui| {
        let geometry = geometry(ui.available_rect_before_wrap(), (1, 0), (3, 0), &BTreeMap::from([(1, 0.0)]));
        *band_edge.borrow_mut() = geometry.cells().left() - geometry.header_size().x;
        let none = OutlineAxis::default();
        OutlineBand::new("Outline", &axis, &none, &geometry).show_at(ui, OutlineBand::area(&geometry));
    });
    fixture.settle();
    let edge = *edge.borrow();
    // A bracket is a vertical stroke (as wide as its own stroke) inside the band, left of the band's edge line.
    let brackets = fixture
        .shape_bounds()
        .iter()
        .zip(fixture.shape_overshoot())
        .filter(|(bounds, overshoot)| bounds.width() <= 2.0 * **overshoot && bounds.height() > 0.0 && bounds.right() < edge)
        .count();
    assert_eq!(brackets, 1, "one bracket in the lane");
}
