//! The columns of a grid keep their least: a column whose content takes the room it is offered (wrapping text, a
//! picture) leaves the others theirs, and a grid whose columns do not fit even at their least lays its cells out one
//! under another. A picture takes the width it is given.

use craft_behavior::{ColumnWidth, Mixed};
use craft_ui::contract::{Bind, Show as _, ShowContainer as _};
use craft_ui::display::Text;
use craft_ui::layout_components::{Grid, Stack};
use craft_ui::testing::{Frames, shrink_least_runs, trial};
use craft_ui::value_drag::PointPicker;
use egui::{Rect, Ui, Vec2};
use std::cell::Cell;

const SNAP: f32 = 1.0;
const LONG: &str = "A sentence that wraps at whatever width its column is offered, however wide that is";
const WORD: &str = "Brightness";
const HALVES: &[ColumnWidth] = &[ColumnWidth::Content, ColumnWidth::Content];
const SIDE_AND_MAIN: &[ColumnWidth] = &[ColumnWidth::Content, ColumnWidth::fill(1.0)];

/// Shows `ui` in a stack `width` wide for a few frames.
fn in_room(width: f32, mut ui_fn: impl FnMut(&mut Ui)) -> Frames {
    let mut frames = Frames::new();
    frames.idle(4, |ui| {
        let rect = Rect::from_min_size(ui.available_rect_before_wrap().min, Vec2::new(width, 600.0));
        Stack::new().fill().show_at(ui, rect, |ui, _| ui_fn(ui));
    });
    frames
}

/// The rects of the two cells of a grid over `columns`: `first` in the first, a single word in the second.
fn cells(columns: &'static [ColumnWidth], width: f32, first: &'static str) -> (Rect, Rect, Frames) {
    let seen = Cell::new((Rect::NOTHING, Rect::NOTHING));
    let frames = in_room(width, |ui| {
        Grid::new(columns).show(ui, |ui, _| {
            let a = Text::new(first).show(ui).rect;
            let b = Text::new(WORD).show(ui).rect;
            seen.set((a, b));
        });
    });
    let (a, b) = seen.get();
    (a, b, frames)
}

/// The width of `WORD` on one line.
fn word_width() -> f32 {
    let seen = Cell::new(0.0);
    let _ = in_room(800.0, |ui| seen.set(Text::new(WORD).show(ui).rect.width()));
    seen.get()
}

#[trial]
fn a_column_that_takes_its_room_leaves_the_next_column_its_least() {
    let word = word_width();
    for columns in [HALVES, SIDE_AND_MAIN] {
        let (wrap, next, frames) = cells(columns, 400.0, LONG);
        assert!(next.width() >= word - SNAP, "the second column keeps its widest word");
        assert!(wrap.right() <= next.left() + SNAP, "the wrapping column ends before the next one starts");
        assert!(next.right() <= 400.0 + SNAP, "both columns fit the room");
        assert!(shrink_least_runs(&frames.ctx).iter().all(|(_, count)| *count == 1), "a short grid asks its columns once a run");
    }
}

#[trial]
fn a_grid_that_fits_asks_no_column_for_its_least() {
    let (_, _, frames) = cells(HALVES, 400.0, "Exposure");
    assert!(shrink_least_runs(&frames.ctx).is_empty(), "a grid whose columns fit measures once");
}

#[trial]
fn a_grid_whose_columns_do_not_fit_at_their_least_stacks_them() {
    let word = word_width();
    let room = word * 1.5;
    let (first, second, _) = cells(HALVES, room, WORD);
    assert!(second.top() >= first.bottom() - SNAP, "the second cell goes under the first");
    assert!(first.right() <= room + SNAP && second.right() <= room + SNAP, "no cell runs past the room");
}

#[trial]
fn a_picture_takes_the_width_it_is_given() {
    let width = |room: f32| {
        let seen = Cell::new(0.0);
        let _ = in_room(room, |ui| {
            let mut point = [Mixed::Same(0.5), Mixed::Same(0.5)];
            seen.set(PointPicker::new(Bind::bound(&mut point), 0.0..=1.0, 0.0..=1.0).show(ui).response.rect.width());
        });
        seen.get()
    };
    let (wide, wider) = (width(500.0), width(700.0));
    assert!((wide - 500.0).abs() <= SNAP && (wider - 700.0).abs() <= SNAP, "the picture follows its room");
}
