//! `BodyLayout` under direction: the physical layouts, measured `Row` placement and the `Grid` cursor.

use craft_behavior::ColumnWidth;
use craft_ui::layout::BodyLayout;
use craft_ui::runtime::env::{Direction, with_env};
use craft_ui::testing::layout::body;
use craft_ui::testing::{Frames, trial};
use egui::{Rect, Ui, Vec2};
use std::cell::{Cell, RefCell};

const ITEM: Vec2 = Vec2::new(40.0, 10.0);

/// Runs `show` for five frames, in `direction` when given, and returns the last frame's region and rects.
fn lay_out(direction: Option<Direction>, show: impl Fn(&mut Ui) -> Vec<Rect>) -> (Rect, Vec<Rect>) {
    let mut frames = Frames::new();
    let region = Cell::new(Rect::NOTHING);
    let rects = RefCell::new(Vec::new());
    frames.idle(5, |ui| {
        region.set(ui.available_rect_before_wrap());
        *rects.borrow_mut() = match direction {
            Some(direction) => with_env(ui, |o| o.direction = Some(direction), &show),
            None => show(ui),
        };
    });
    (region.get(), rects.into_inner())
}

static COLUMNS: [ColumnWidth; 3] = [ColumnWidth::Fixed(100.0), ColumnWidth::Content, ColumnWidth::fill(1.0)];

/// Skeleton: `frames_state::grid_places_children_row_major_with_shared_column_widths`.
#[trial]
fn grid_places_children_row_major_with_shared_column_widths() {
    let grid = |long: bool| {
        move |ui: &mut Ui| {
            let wide = Vec2::new(if long { 150.0 } else { 40.0 }, 10.0);
            body(ui, "g", &BodyLayout::grid(&COLUMNS), &[ITEM, ITEM, ITEM, ITEM, wide, ITEM])
        }
    };
    let (_, ltr) = lay_out(None, grid(false));
    assert!((ltr[0].min.x - ltr[3].min.x).abs() < 1.0 && (ltr[2].min.x - ltr[5].min.x).abs() < 1.0, "a column keeps its x across rows");
    assert!(ltr[0].min.x < ltr[1].min.x && ltr[1].min.x < ltr[2].min.x, "columns run in reading order");
    assert!((ltr[1].min.x - ltr[0].min.x - 100.0).abs() < 1.0, "the Fixed column is the app's width");
    assert!(ltr[3].min.y >= ltr[0].max.y && (ltr[4].min.y - ltr[3].min.y).abs() < 1.0, "the second row lies under the first");
    let (_, long) = lay_out(None, grid(true));
    assert!(long[2].min.x > ltr[2].min.x + 100.0, "a Content column takes its widest child: {:?} -> {:?}", ltr[2], long[2]);
    let (region, rtl) = lay_out(Some(Direction::Rtl), grid(false));
    assert!(rtl[0].min.x > rtl[1].min.x && rtl[1].min.x > rtl[2].min.x, "under RTL the first column is on the right");
    assert!((rtl[0].max.x - region.max.x).abs() < 1.0);
}
