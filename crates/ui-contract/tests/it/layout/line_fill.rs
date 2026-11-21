//! Filling the line of a row that takes its height from its content: a levelled unit stands on the row's line and its
//! children take no slots on it, a container that fills its height there runs its body in the line's height from the
//! first frame and measures its content while the row measures, and a centred row in a unit of a row that scrolls
//! centres across its own column, not in the view.

use craft_ui::contract::{ContainerProps, HeaderRegion, Props as _, Scroll, Show as _, ShowContainer as _, Sizing};
use craft_ui::layout::{BodyLayout, RowAlign, levelled_unit};
use craft_ui::testing::{Frames, trial};
use craft_ui_test_kit::body::Slab;
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
        levelled_unit(ui, Id::unique("unit"), 2, |ui, _| {
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

/// A levelled unit of two controls stays one line high: its children sit in the unit's own column, not each on a slot
/// as tall as the row's line.
#[trial]
fn a_levelled_unit_of_two_controls_stays_one_line_high() {
    let second = RefCell::new(Seen::default());
    let seen = line_and_fill(|ui, top| {
        levelled_unit(ui, Id::unique("unit"), 1, |ui, _| {
            Probe::new(top).id_salt("first").show(ui);
            Probe::new(&second).id_salt("second").show(ui);
        });
    });
    let second = second.borrow().rect;
    for ((row, top), when) in seen.into_iter().zip(["in the first frame", "once settled"]) {
        assert!((row.height() - TALL).abs() <= SNAP, "{when}: the row {row:?} stays as tall as its tall member");
        assert!(
            top.top() >= row.top() - SNAP && top.bottom() <= row.bottom() + SNAP,
            "{when}: the unit's first control {top:?} lies in the row {row:?}"
        );
    }
    let row = seen[1].0;
    assert!(second.bottom() <= row.bottom() + SNAP, "the unit's second control {second:?} lies in the row {row:?}");
}

/// A container that fills its height, on a row that shares its line with an inline header, measures its content while
/// the row measures: the shared line, shorter than the content, never caps it, and both its lines show in the row.
#[trial]
fn a_fill_height_container_on_a_shared_line_shows_all_its_content() {
    let (column, first, second) = (RefCell::new(Seen::default()), RefCell::new(Seen::default()), RefCell::new(Seen::default()));
    let props = ContainerProps::new().layout(BodyLayout::row()).header(HeaderRegion::new().inline(true));
    let mut frames = Frames::new();
    let mut show = |ui: &mut egui::Ui| {
        Slab::new(props.clone()).id_salt("bar").show(ui, |ui, _| {
            Panel::new().layout(BodyLayout::stack()).sizing(Sizing::FillHeight).seen(&column).id_salt("column").show(ui, |ui, _| {
                Probe::new(&first).id_salt("first").show(ui);
                Probe::new(&second).id_salt("second").show(ui);
            });
        });
    };
    for (frames_run, when) in [(1, "in the first frame"), (4, "once settled")] {
        frames.idle(frames_run, &mut show);
        let (column, first, second) = (column.borrow().rect, first.borrow().rect, second.borrow().rect);
        assert!(second.top() >= first.bottom() - SNAP, "{when}: the second line {second:?} sits under the first {first:?}");
        assert!(
            first.top() >= column.top() - SNAP && second.bottom() <= column.bottom() + SNAP,
            "{when}: the column {column:?} holds both its lines, {first:?} and {second:?}"
        );
    }
}

#[trial]
fn a_fill_height_container_on_a_content_height_row_runs_its_body_in_the_line() {
    let seen = line_and_fill(|ui, fill| {
        let top = RefCell::new(Seen::default());
        Panel::new().layout(BodyLayout::stack()).sizing(Sizing::FillHeight).id_salt("column").show(ui, |ui, _| {
            Probe::new(&top).id_salt("top").show(ui);
            Probe::new(fill).sizing(Sizing::Fill).id_salt("fill").show(ui);
        });
    });
    for ((row, fill), when) in seen.into_iter().zip(["in the first frame", "once settled"]) {
        assert!((row.height() - TALL).abs() <= SNAP, "{when}: the row {row:?} is as tall as its tall member");
        assert!(
            (fill.bottom() - row.bottom()).abs() <= SNAP && fill.height() > TALL / 2.0,
            "{when}: the column's filling child {fill:?} takes what its column leaves of the row's line {row:?}"
        );
    }
}

#[trial]
fn a_centred_row_in_a_unit_of_a_scrolling_row_centres_across_its_column() {
    let (wide, small) = (RefCell::new(Seen::default()), RefCell::new(Seen::default()));
    let mut frames = Frames::new();
    frames.idle(5, |ui| {
        let row = ContainerProps::new().layout(BodyLayout::row()).scroll(Scroll::horizontal()).sizing(Sizing::FillWidth);
        Slab::new(row).id_salt("row").show(ui, |ui, _| {
            levelled_unit(ui, Id::unique("unit"), 1, |ui, _| {
                Panel::new().layout(BodyLayout::stack()).id_salt("column").show(ui, |ui, _| {
                    Probe::new(&wide).size(Vec2::new(200.0, 20.0)).id_salt("wide").show(ui);
                    let centred = ContainerProps::new().layout(BodyLayout::row().align(RowAlign::Center)).sizing(Sizing::FillWidth);
                    Slab::new(centred).id_salt("centred").show(ui, |ui, _| {
                        Probe::new(&small).id_salt("small").show(ui);
                    });
                });
            });
        });
    });
    let (wide, small) = (wide.borrow().rect, small.borrow().rect);
    assert!(
        (small.center().x - wide.center().x).abs() <= SNAP,
        "the centred probe {small:?} sits in the middle of its column, under the wide probe {wide:?}"
    );
}
