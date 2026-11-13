//! A leaf's `Sizing::FillWidth`: the leaf takes the room its row leaves after the other members. In a `Stack` body
//! that scrolls along y the children that fill its height share what the view leaves, at least their content.

use craft_ui::contract::{ContainerProps, Props as _, Scroll, Show as _, ShowContainer as _, Sizing};
use craft_ui::layout::BodyLayout;
use craft_ui::testing::layout::Block;
use craft_ui::testing::{Fixture, Frames, pointer, trial};
use craft_ui_test_kit::body::Slab;
use craft_ui_test_kit::contract::{Panel, Probe, Seen};
use egui::{Pos2, Rect, Vec2};
use std::cell::{Cell, RefCell};
use std::rc::Rc;

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

/// The view, the fixed block, the leaf that fills and the container that fills (holding a probe at its least height),
/// in a scrolling `Stack` body that fills a 200 by 200 room and starts with a block `fixed` tall, scrolled to its end.
fn scrolled(fixed: f32) -> [Rect; 4] {
    let seen = Rc::new(Cell::new([Rect::NOTHING; 4]));
    let out = Rc::clone(&seen);
    let mut fixture = Fixture::new(move |ui| {
        let (leaf, inner) = (RefCell::new(Seen::default()), RefCell::new(Seen::default()));
        let body = Slab::new(ContainerProps::new().sizing(Sizing::Fill).scroll(Scroll::vertical())).id_salt("view").show(ui, |ui, _| {
            let block = Block::new(Vec2::new(40.0, fixed)).show(ui).rect;
            Probe::new(&leaf).sizing(Sizing::Fill).id_salt("leaf").show(ui);
            let held = Panel::new().sizing(Sizing::Fill).id_salt("held").show(ui, |ui, _| {
                Probe::new(&inner).id_salt("inner").show(ui);
            });
            (block, held.output.rect)
        });
        if let Some((block, held)) = body.inner {
            out.set([body.output.rect, block, leaf.borrow().rect, held]);
        }
    });
    fixture.room(Vec2::splat(200.0)).settle();
    fixture.frame(vec![pointer::moved(Pos2::new(100.0, 100.0))]);
    fixture.frame(vec![pointer::wheel(-10_000.0)]).settle();
    seen.get()
}

#[trial]
fn a_scrolled_stack_gives_its_fill_children_what_the_view_leaves_and_scrolls_past_their_least() {
    let [view, block, leaf, held] = scrolled(40.0);
    let dump = format!("view {view:?}, block {block:?}, leaf {leaf:?}, held {held:?}");
    assert!(leaf.height() > 40.0 && (leaf.height() - held.height()).abs() < 1.0, "shorter content: the fill children share the rest: {dump}");
    assert!((held.bottom() - view.bottom()).abs() < 1.0, "shorter content: it reaches the view's end: {dump}");
    let [view, _, leaf, held] = scrolled(400.0);
    let dump = format!("view {view:?}, leaf {leaf:?}, held {held:?}");
    assert!((leaf.height() - 20.0).abs() < 1.0 && (held.height() - 20.0).abs() < 1.0, "longer content: the fill children at their least: {dump}");
    assert!((held.bottom() - view.bottom()).abs() < 1.0, "longer content: the view scrolls to the end of the last one: {dump}");
}
