//! A leaf's `Sizing::FillWidth`: the leaf takes the room its row leaves after the other members. In a `Stack` body
//! that scrolls along y the children that fill its height share what the view leaves, at least their content.

use craft_ui::contract::{ContainerProps, Props as _, Scroll, Show as _, ShowContainer as _, Sizing};
use craft_ui::layout::BodyLayout;
use craft_ui::testing::layout::Block;
use craft_ui::testing::{Fixture, Frames, is_measuring, pointer, trial};
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

/// What the container that fills holds in the `scrolled` rig: a probe at its least height, or a block this tall.
#[derive(Clone, Copy)]
pub(super) enum Held {
    Probe,
    Block(f32),
}

/// A `Stack` body that fills a 200 by 200 room, scrolling along y when `scrolls`, holding a block `fixed` tall, a leaf
/// that fills and a container that fills, holding `held`. Each frame sets `seen` to the view, the block, the leaf, the
/// container (each as shown, clipped to the view) and what it holds; `runs` counts the measuring runs of the
/// container's body.
pub(super) fn scrolled_fixture(fixed: f32, held: Held, scrolls: bool, seen: Rc<Cell<[Rect; 5]>>, runs: Rc<Cell<usize>>) -> Fixture {
    let mut fixture = Fixture::new(move |ui| {
        let (leaf, inner) = (RefCell::new(Seen::default()), RefCell::new(Seen::default()));
        let props = ContainerProps::new().sizing(Sizing::Fill);
        let props = if scrolls { props.scroll(Scroll::vertical()) } else { props };
        let body = Slab::new(props).id_salt("view").show(ui, |ui, _| {
            let block = Block::new(Vec2::new(40.0, fixed)).show(ui).rect;
            Probe::new(&leaf).sizing(Sizing::Fill).id_salt("leaf").show(ui);
            let held = Panel::new().sizing(Sizing::Fill).id_salt("held").show(ui, |ui, _| {
                if is_measuring(ui.ctx()) {
                    runs.set(runs.get() + 1);
                }
                match held {
                    Held::Probe => {
                        Probe::new(&inner).id_salt("inner").show(ui);
                        inner.borrow().rect
                    }
                    // The rect the body laid the block out in, not clipped to the view.
                    Held::Block(height) => {
                        Block::new(Vec2::new(40.0, height)).show(ui);
                        ui.min_rect()
                    }
                }
            });
            (block, held.output.rect, held.inner)
        });
        if let Some((block, held, content)) = body.inner {
            seen.set([body.output.rect, block, leaf.borrow().rect, held, content.unwrap_or(Rect::NOTHING)]);
        }
    });
    fixture.room(Vec2::splat(200.0)).settle();
    fixture
}

/// The view, the fixed block, the leaf, the container and what it holds, in the scrolling `scrolled_fixture` scrolled
/// to its end.
fn scrolled(fixed: f32, held: Held) -> [Rect; 5] {
    let seen = Rc::new(Cell::new([Rect::NOTHING; 5]));
    let mut fixture = scrolled_fixture(fixed, held, true, Rc::clone(&seen), Rc::new(Cell::new(0)));
    fixture.frame(vec![pointer::moved(Pos2::new(100.0, 100.0))]);
    fixture.frame(vec![pointer::wheel(-10_000.0)]).settle();
    seen.get()
}

#[trial]
fn a_scrolled_stack_gives_its_fill_children_what_the_view_leaves_and_scrolls_past_their_least() {
    let [view, block, leaf, held, _] = scrolled(40.0, Held::Probe);
    let dump = format!("view {view:?}, block {block:?}, leaf {leaf:?}, held {held:?}");
    assert!(leaf.height() > 40.0 && (leaf.height() - held.height()).abs() < 1.0, "shorter content: the fill children share the rest: {dump}");
    assert!((held.bottom() - view.bottom()).abs() < 1.0, "shorter content: it reaches the view's end: {dump}");
    let [view, _, leaf, held, _] = scrolled(400.0, Held::Probe);
    let dump = format!("view {view:?}, leaf {leaf:?}, held {held:?}");
    assert!((leaf.height() - 20.0).abs() < 1.0 && (held.height() - 20.0).abs() < 1.0, "longer content: the fill children at their least: {dump}");
    assert!((held.bottom() - view.bottom()).abs() < 1.0, "longer content: the view scrolls to the end of the last one: {dump}");
    // A container whose own content is taller than the view lays it out whole, and the view scrolls to its end.
    const TALL: f32 = 300.0;
    let [view, _, _, held, content] = scrolled(40.0, Held::Block(TALL));
    let dump = format!("view {view:?}, held {held:?}, content {content:?}");
    assert!((content.height() - TALL).abs() < 0.5, "a taller container keeps its content height: {dump}");
    assert!((content.bottom() - view.bottom()).abs() < 1.0, "nothing in it is cut: the view scrolls to the content's end: {dump}");
    assert!((held.bottom() - view.bottom()).abs() < 1.0, "the container reaches the view's end: {dump}");
}
