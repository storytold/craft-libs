//! Tests for flat and spatial navigation.

use super::*;
use crate::intent::{MoveMods, NavMove};
use emath::Rect;

struct Rows {
    depth: Vec<usize>,
    open: Vec<bool>,
    children: Vec<bool>,
    disabled: Vec<usize>,
}

impl Rows {
    fn flat(n: usize) -> Rows {
        Rows { depth: vec![0; n], open: vec![false; n], children: vec![false; n], disabled: vec![] }
    }
}

impl NavOutline for Rows {
    fn len(&self) -> usize {
        self.depth.len()
    }
    fn depth(&self, at: usize) -> usize {
        self.depth[at]
    }
    fn has_children(&self, at: usize) -> bool {
        self.children[at]
    }
    fn open(&self, at: usize) -> bool {
        self.open[at]
    }
    fn is_section(&self, _: usize) -> bool {
        false
    }
    fn enabled(&self, at: usize) -> bool {
        !self.disabled.contains(&at)
    }
}

fn go(shape: NavShape, rows: &Rows, state: &mut NavState, mv: NavMove) -> NavStep {
    step(shape, rows, state, mv, MoveMods::PLAIN)
}

#[test]
fn flat_moves_skip_disabled_rows_and_stop_or_wrap_at_the_ends() {
    let mut rows = Rows::flat(5);
    rows.disabled = vec![2];
    let mut state = NavState::at(1);
    assert_eq!(go(NavShape::Flat, &rows, &mut state, NavMove::Down), NavStep::Moved(3));
    assert_eq!(go(NavShape::Flat, &rows, &mut state, NavMove::End), NavStep::Moved(4));
    assert_eq!(go(NavShape::Flat, &rows, &mut state, NavMove::Forward), NavStep::Stay);
    state.wrap = true;
    assert_eq!(go(NavShape::Flat, &rows, &mut state, NavMove::Forward), NavStep::Moved(0));
    assert_eq!(go(NavShape::Flat, &rows, &mut state, NavMove::PageDown), NavStep::Moved(4));
    let mut none = NavState::new();
    assert_eq!(go(NavShape::Flat, &rows, &mut none, NavMove::Up), NavStep::Moved(4), "the first backward move lands on the last row");
}

/// Rows of 3, 1 and 2 positions; a sheet of 3 rows by 4 columns; four items at free positions.
struct Shapes;

impl NavOutline for Shapes {
    fn len(&self) -> usize {
        6
    }
    fn depth(&self, _: usize) -> usize {
        0
    }
    fn has_children(&self, _: usize) -> bool {
        false
    }
    fn open(&self, _: usize) -> bool {
        false
    }
    fn is_section(&self, _: usize) -> bool {
        false
    }
    fn enabled(&self, _: usize) -> bool {
        true
    }
    fn row_len(&self, row: usize) -> Option<usize> {
        [3, 1, 2].get(row).copied()
    }
    fn columns(&self) -> usize {
        4
    }
    fn rect(&self, at: usize) -> Option<Rect> {
        let spots = [(0.0, 0.0), (100.0, 10.0), (10.0, 100.0), (110.0, 110.0), (200.0, 0.0), (0.0, 200.0)];
        spots.get(at).map(|&(x, y)| Rect::from_min_size(emath::Pos2::new(x, y), emath::Vec2::splat(20.0)))
    }
}

#[test]
fn spatial_moves_go_to_the_nearest_item_in_that_direction() {
    let mut state = NavState::at(0);
    let go = |state: &mut NavState, mv| step(NavShape::Spatial, &Shapes, state, mv, MoveMods::PLAIN);
    assert_eq!(go(&mut state, NavMove::Forward), NavStep::Moved(1), "the near item at the right, not the far one in line");
    assert_eq!(go(&mut state, NavMove::Down), NavStep::Moved(3));
    assert_eq!(go(&mut state, NavMove::Back), NavStep::Moved(2));
    assert_eq!(go(&mut state, NavMove::Up), NavStep::Moved(0));
    assert_eq!(go(&mut state, NavMove::Up), NavStep::Stay);
}

#[test]
fn snap_to_step_keeps_stops_and_range_ends() {
    assert_eq!(snap_to_step(9.5, 4.0, 0.0, 0.0..=10.0), 10.0, "the end of the range is a stop");
    assert_eq!(snap_to_step(9.0, 4.0, 0.0, 0.0..=10.0), 10.0, "a tie goes to the larger stop");
    assert_eq!(snap_to_step(-2.5, 1.0, 0.0, f64::NEG_INFINITY..=f64::INFINITY), -2.0, "ties go up for negatives too");
    assert_eq!(snap_to_step(0.3, 0.1, 0.0, 0.0..=1.0), 0.3, "a value on a stop comes back unchanged");
    assert_eq!(snap_to_step(4.9, 2.0, 0.0, 1.0..=10.0), 4.0, "the grid starts at the origin, not the range");
    assert_eq!(snap_to_step(12.0, 0.0, 0.0, 0.0..=10.0), 10.0, "a step of 0 only clamps");
}
