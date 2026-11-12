//! A leaf's `Sizing::FillWidth`: the leaf takes the room its row leaves after the other members.

use craft_ui::contract::{Props as _, Show as _, ShowContainer as _, Sizing};
use craft_ui::layout::BodyLayout;
use craft_ui::testing::{Frames, trial};
use craft_ui_test_kit::contract::{Panel, Probe, Seen};
use egui::Rect;
use std::cell::{Cell, RefCell};

/// The region, the first probe's rect and the second's, in a full-width row whose first member has `sizing`.
fn row(sizing: Sizing) -> (Rect, Rect, Rect) {
    let (first, second) = (RefCell::new(Seen::default()), RefCell::new(Seen::default()));
    let region = Cell::new(Rect::NOTHING);
    let mut frames = Frames::new();
    frames.idle(5, |ui| {
        region.set(ui.available_rect_before_wrap());
        Panel::new().layout(BodyLayout::row()).sizing(Sizing::FillWidth).id_salt("row").show(ui, |ui, _| {
            Probe::new(&first).sizing(sizing).id_salt("first").show(ui);
            Probe::new(&second).id_salt("second").show(ui);
        });
    });
    (region.get(), first.into_inner().rect, second.into_inner().rect)
}

#[trial]
fn a_fill_width_leaf_takes_the_room_its_row_leaves_and_a_content_leaf_does_not() {
    let (region, content, after_content) = row(Sizing::Content);
    let (_, fill, after_fill) = row(Sizing::FillWidth);
    assert!(after_content.max.x < region.center().x, "a content-sized leaf leaves the row's room empty: {after_content:?} in {region:?}");
    assert!(fill.width() > content.width() * 2.0, "the fill-width leaf takes the room: {fill:?} against {content:?}");
    assert!((after_fill.width() - after_content.width()).abs() < 1.0, "the member after it keeps its size");
    assert!(after_fill.min.x >= fill.max.x && after_fill.min.x > region.center().x, "the next member follows it at the row's end: {after_fill:?}");
}
