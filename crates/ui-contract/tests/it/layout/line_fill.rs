//! Filling the line of a row that takes its height from its content: a levelled unit stands on the row's line.

use craft_ui::contract::{Props as _, Show as _, ShowContainer as _, Sizing};
use craft_ui::layout::{BodyLayout, levelled_unit};
use craft_ui::testing::{Frames, trial};
use craft_ui_test_kit::contract::{Panel, Probe, Seen};
use egui::{Id, Rect, Vec2};
use std::cell::RefCell;

const SNAP: f32 = 1.0;

/// The height of the tall member that sets each row's line.
const TALL: f32 = 100.0;

/// The rect of the row and of the probe that fills, after the first frame and once settled, for a row of a tall probe
/// beside `filler`, which shows the filling probe into `seen`.
fn line_and_fill(filler: impl Fn(&mut egui::Ui, &RefCell<Seen>)) -> [(Rect, Rect); 2] {
    let (tall, fill, row) = (RefCell::new(Seen::default()), RefCell::new(Seen::default()), RefCell::new(Seen::default()));
    let mut frames = Frames::new();
    let mut show = |ui: &mut egui::Ui| {
        Panel::new().layout(BodyLayout::row()).seen(&row).id_salt("row").show(ui, |ui, _| {
            Probe::new(&tall).size(Vec2::new(40.0, TALL)).id_salt("tall").show(ui);
            filler(ui, &fill);
        });
    };
    frames.frame(Vec::new(), &mut show);
    let first = (row.borrow().rect, fill.borrow().rect);
    frames.idle(4, &mut show);
    let settled = (row.borrow().rect, fill.borrow().rect);
    [first, settled]
}

#[trial]
fn a_levelled_unit_stands_on_its_rows_line() {
    let seen = line_and_fill(|ui, fill| {
        levelled_unit(ui, Id::new("unit"), 2, |ui, _| {
            Probe::new(fill).sizing(Sizing::Fill).id_salt("fill").show(ui);
        });
    });
    for ((row, fill), when) in seen.into_iter().zip(["in the first frame", "once settled"]) {
        assert!((row.height() - TALL).abs() <= SNAP, "{when}: the row {row:?} is as tall as its tall member");
        assert!(
            (fill.top() - row.top()).abs() <= SNAP && (fill.bottom() - row.bottom()).abs() <= SNAP,
            "{when}: the unit's filling child {fill:?} spans the row's line {row:?}"
        );
    }
}
