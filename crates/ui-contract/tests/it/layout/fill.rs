//! A leaf's `Sizing::FillWidth`: the leaf takes the room its row leaves after the other members. In a `Stack` body
//! that scrolls along y the children that fill its height share what the view leaves, at least their content. A child
//! that fills a bounded room never takes less than its least; in an unbounded one it keeps its ideal.

use crate::collection::kit::{Count, List, Row as ListRow};
use craft_behavior::{Proposal, Sense};
use craft_ui::contract::{
    Common, Component, ContainerProps, Cx, MeasureCx, Props as _, Role, Scroll, SelectMode, Show as _, ShowCollection as _, ShowContainer as _,
    Sizing,
};
use craft_ui::layout::BodyLayout;
use craft_ui::style::StyleCx;
use craft_ui::testing::layout::Block;
use craft_ui::testing::{Fixture, Frames, is_measuring, pointer, trial};
use craft_ui_test_kit::body::Slab;
use craft_ui_test_kit::contract::{Panel, PlainStyle, Probe, Seen};
use egui::{Pos2, Rect, Response, Vec2};
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

/// A `Stretch`'s ideal width, least width and height.
const IDEAL: f32 = 300.0;
const LEAST: f32 = 40.0;
const LINE: f32 = 20.0;

/// How many measures of each proposal a `Stretch` answered in measuring runs.
#[derive(Default)]
struct Asked {
    ideal: Cell<usize>,
    min: Cell<usize>,
}

/// A leaf `IDEAL` wide that narrows to `LEAST`, whatever its room; it counts the measures it answers in measuring runs.
#[derive(Component)]
struct Stretch<'a> {
    #[common]
    common: Common,
    seen: &'a RefCell<Seen>,
    asked: &'a Asked,
    track_input: bool,
}

impl<'a> Stretch<'a> {
    fn new(seen: &'a RefCell<Seen>, asked: &'a Asked) -> Self {
        Stretch { common: Common::new(), seen, asked, track_input: false }
    }
}

impl Component for Stretch<'_> {
    const ROLE: Role = Role::GROUP;
    type Style = PlainStyle;
    type Output = Response;

    fn style(_: &StyleCx<'_>, _: StretchStyleKey) -> PlainStyle {
        PlainStyle::default()
    }

    fn measure(&mut self, cx: &MeasureCx<'_, Self>) -> Vec2 {
        if self.track_input {
            let _ = cx.column_room();
        }
        let (count, width) = if cx.proposal == Proposal::Min { (&self.asked.min, LEAST) } else { (&self.asked.ideal, IDEAL) };
        if is_measuring(cx.ui.ctx()) {
            count.set(count.get() + 1);
        }
        Vec2::new(width, LINE)
    }

    fn sense(&self) -> Sense {
        Sense::empty()
    }

    fn update(&mut self, cx: &mut Cx<'_, Self>) {
        self.seen.borrow_mut().rect = cx.rect;
    }

    fn finish(self, cx: Cx<'_, Self>) -> Response {
        cx.response.clone()
    }
}

#[trial]
fn a_nested_body_reuses_a_settled_transcript_and_remeasures_after_its_room_changes() {
    let seen = Rc::new(RefCell::new(Seen::default()));
    let asked = Rc::new(Asked::default());
    let sweeps = Rc::new(Cell::new(0));
    let (shown, counts, swept) = (Rc::clone(&seen), Rc::clone(&asked), Rc::clone(&sweeps));
    let mut fixture = Fixture::new(move |ui| {
        Panel::new().layout(BodyLayout::row()).sizing(Sizing::FillWidth).id_salt("outer").show(ui, |ui, _| {
            Panel::new().layout(BodyLayout::row()).sizing(Sizing::FillWidth).id_salt("inner").show(ui, |ui, _| {
                if is_measuring(ui.ctx()) {
                    swept.set(swept.get() + 1);
                }
                let mut child = Stretch::new(&shown, &counts);
                child.track_input = true;
                child.sizing(Sizing::FillWidth).id_salt("child").show(ui);
            });
        });
    });
    // The measures the child answered in measuring runs.
    let measured = |asked: &Asked| asked.ideal.get() + asked.min.get();
    fixture.room(Vec2::new(240.0, 100.0)).settle();
    assert!(sweeps.get() > 0 && measured(&asked) > 0, "the first layout measures the nested body");
    let before = seen.borrow().rect;
    sweeps.set(0);
    asked.ideal.set(0);
    asked.min.set(0);
    fixture.idle(3);
    assert_eq!(sweeps.get(), 0, "a kept transcript skips nested measuring sweeps");
    assert_eq!(measured(&asked), 0, "recorded inputs need no new measure on idle");
    assert_eq!(seen.borrow().rect, before, "replay keeps the same geometry");
    fixture.room(Vec2::new(140.0, 100.0)).settle();
    assert!(sweeps.get() > 0 && measured(&asked) > 0, "a changed room takes the measuring fallback");
    let after = seen.borrow().rect;
    assert!((before.width() - after.width()).abs() > 0.5, "the changed input updates geometry: {before:?} -> {after:?}");
}

/// The rect of a row that fills a room `width` wide, scrolling along x when `scrolls`, and of the `Stretch` that fills
/// its width alone in it.
fn stretched(width: f32, scrolls: bool) -> (Rect, Rect) {
    let (seen, row) = (Rc::new(RefCell::new(Seen::default())), Rc::new(Cell::new(Rect::NOTHING)));
    let (shown, shown_row) = (Rc::clone(&seen), Rc::clone(&row));
    let mut fixture = Fixture::new(move |ui| {
        let asked = Asked::default();
        let props = ContainerProps::new().layout(BodyLayout::row()).sizing(Sizing::FillWidth);
        let props = if scrolls { props.scroll(Scroll::horizontal()) } else { props };
        let body = Slab::new(props).id_salt("row").show(ui, |ui, _| {
            Stretch::new(&shown, &asked).sizing(Sizing::FillWidth).id_salt("stretch").show(ui);
        });
        shown_row.set(body.output.rect);
    });
    fixture.room(Vec2::new(width, 100.0)).settle();
    let leaf = seen.borrow().rect;
    (row.get(), leaf)
}

/// In a content-sized row beside a probe three lines high: the probe's rect, that of a container that fills and that
/// of a `Stretch` that fills.
fn in_line() -> [Rect; 3] {
    let (tall, leaf) = (Rc::new(RefCell::new(Seen::default())), Rc::new(RefCell::new(Seen::default())));
    let held = Rc::new(Cell::new(Rect::NOTHING));
    let (shown_tall, shown_leaf, shown_held) = (Rc::clone(&tall), Rc::clone(&leaf), Rc::clone(&held));
    let mut fixture = Fixture::new(move |ui| {
        let asked = Asked::default();
        Panel::new().layout(BodyLayout::row()).id_salt("line").show(ui, |ui, _| {
            Probe::new(&shown_tall).size(Vec2::new(40.0, 3.0 * LINE)).id_salt("tall").show(ui);
            let container = Panel::new().sizing(Sizing::Fill).id_salt("held").show(ui, |ui, _| {
                Block::new(Vec2::new(40.0, LINE)).show(ui);
            });
            shown_held.set(container.output.rect);
            Stretch::new(&shown_leaf, &asked).sizing(Sizing::Fill).id_salt("leaf").show(ui);
        });
    });
    fixture.room(Vec2::splat(800.0)).settle();
    [tall.borrow().rect, held.get(), leaf.borrow().rect]
}

#[trial]
fn a_fill_child_fills_a_bounded_room_down_to_its_least_and_keeps_its_ideal_in_an_unbounded_one() {
    let (row, leaf) = stretched(200.0, false);
    assert!((leaf.width() - row.width()).abs() < 0.5 && leaf.width() < IDEAL, "below its ideal it fills the row: {leaf:?} in {row:?}");
    let (row, leaf) = stretched(30.0, false);
    assert!((leaf.width() - LEAST).abs() < 0.5, "below its least it keeps its least: {leaf:?} in {row:?}");
    let (row, leaf) = stretched(200.0, true);
    assert!((leaf.width() - IDEAL).abs() < 0.5, "in a row that scrolls it keeps its ideal: {leaf:?} in {row:?}");
    let [tall, held, leaf] = in_line();
    let dump = format!("tall {tall:?}, container {held:?}, leaf {leaf:?}");
    assert!((leaf.height() - tall.height()).abs() < 0.5, "the leaf that fills spans the row's line: {dump}");
    assert!((held.height() - tall.height()).abs() < 0.5, "the container that fills spans the row's line as the leaf does: {dump}");
}

/// The leaves of the `measured` rig.
const LEAVES: [&str; 2] = ["first", "second"];

#[trial]
fn fill_children_measure_once_per_sweep() {
    for (layout, sizing, name) in [(BodyLayout::stack(), Sizing::Fill, "stack"), (BodyLayout::row(), Sizing::FillWidth, "row")] {
        let leaves = Rc::new([Asked::default(), Asked::default()]);
        let sweeps = Rc::new(Cell::new(0_usize));
        let (shown, swept) = (Rc::clone(&leaves), Rc::clone(&sweeps));
        let mut fixture = Fixture::new(move |ui| {
            let sink = RefCell::new(Seen::default());
            Panel::new().layout(layout.clone()).sizing(Sizing::Fill).id_salt("body").show(ui, |ui, _| {
                // The body's content runs once per sweep of its measuring runs.
                if is_measuring(ui.ctx()) {
                    swept.set(swept.get() + 1);
                }
                for (asked, salt) in shown.iter().zip(LEAVES) {
                    Stretch::new(&sink, asked).sizing(sizing).id_salt(salt).show(ui);
                }
                Panel::new().sizing(Sizing::Fill).id_salt("held").show(ui, |ui, _| {
                    Block::new(Vec2::new(40.0, LINE)).show(ui);
                });
            });
        });
        fixture.room(Vec2::splat(400.0)).settle();
        let swept = sweeps.get();
        for asked in leaves.iter() {
            let (ideal, min) = (asked.ideal.get(), asked.min.get());
            assert!(ideal == swept, "{name}: {ideal} ideal measures of a fill leaf over {swept} sweeps");
            assert!(min >= 1 && 2 * min <= swept, "{name}: a fill leaf's least asked {min} times over {swept} sweeps, not once per first sweep");
            asked.ideal.set(0);
            asked.min.set(0);
        }
        sweeps.set(0);
        fixture.idle(3);
        for asked in leaves.iter() {
            let (ideal, min) = (asked.ideal.get(), asked.min.get());
            assert!(ideal == 0 && min == 0, "{name}: unchanged inputs measured a fill leaf again: {ideal} ideal, {min} least");
        }
        assert_eq!(sweeps.get(), 0, "{name}: unchanged inputs ran the body's measuring run");
    }
}

/// What fills the start of the `content_row` rig.
#[derive(Clone, Copy, Debug)]
enum First {
    /// A leaf that fills the width.
    Leaf,
    /// A fill-width container whose body holds that leaf.
    Held,
    /// A list that fills.
    List,
}

/// The row, what fills its start and the member after it, in a content-sized `Row` inside a content-sized `Stack`.
fn content_row(first: First) -> [Rect; 3] {
    let seen = Rc::new(Cell::new([Rect::NOTHING; 3]));
    let out = Rc::clone(&seen);
    let mut fixture = Fixture::new(move |ui| {
        let (sink, after) = (RefCell::new(Seen::default()), RefCell::new(Seen::default()));
        let asked = Asked::default();
        let rects = Panel::new().id_salt("column").show(ui, |ui, _| {
            let row = Panel::new().layout(BodyLayout::row()).id_salt("row").show(ui, |ui, _| {
                let leaf = |ui: &mut egui::Ui| Stretch::new(&sink, &asked).sizing(Sizing::FillWidth).id_salt("fill").show(ui).rect;
                let filled = match first {
                    First::Leaf => leaf(ui),
                    First::Held => Panel::new().sizing(Sizing::FillWidth).id_salt("held").show(ui, |ui, _| leaf(ui)).inner.unwrap_or(Rect::NOTHING),
                    First::List => List::new(SelectMode::None).id_salt("list").show_items(ui, &Count(8), &mut |_, _| ListRow::new()).response.rect,
                };
                let next = Probe::new(&after).size(Vec2::new(100.0, LINE)).id_salt("after").show(ui).rect;
                [filled, next]
            });
            let [filled, next] = row.inner.unwrap_or([Rect::NOTHING; 2]);
            [row.output.rect, filled, next]
        });
        out.set(rects.inner.unwrap_or([Rect::NOTHING; 3]));
    });
    fixture.room(Vec2::new(900.0, 400.0)).settle();
    seen.get()
}

// A row sized from its content gives a child that fills its width the width it measured, not the row's room: the
// member after it follows it, and both stay inside the row.
#[trial]
fn a_fill_child_of_a_content_sized_row_takes_its_measured_width_and_the_next_member_follows_it() {
    for first in [First::Leaf, First::Held, First::List] {
        let [row, fill, after] = content_row(first);
        assert!(fill.width() > 0.0, "{first:?}: the fill child shows: {fill:?}");
        assert!(fill.max.x <= after.min.x + 0.5, "{first:?}: the member after the fill child follows it: {fill:?}, {after:?}");
        assert!(row.contains_rect(fill) && row.contains_rect(after), "{first:?}: both stay inside the row {row:?}: {fill:?}, {after:?}");
    }
    let [_, leaf, _] = content_row(First::Held);
    assert!(leaf.width() >= IDEAL - 0.5, "the held leaf shows at least its ideal: {leaf:?}");
}

/// A reveal of a rect a scroll body's view already shows moves nothing, so it asks for no second pass: a part that
/// asks every frame must not make every frame run twice (egui's "request_discard has been called N frames in a row").
#[trial]
fn a_reveal_of_a_rect_in_a_scroll_bodys_view_runs_no_second_pass() {
    let owner = egui::Id::unique("revealed body");
    let mut frames = Frames::new();
    let mut show = |ui: &mut egui::Ui| {
        let props = ContainerProps::new().sizing(Sizing::Fill).scroll(Scroll::vertical());
        Slab::new(props).id(owner).show(ui, |ui, _| {
            Block::new(Vec2::new(40.0, 600.0)).show(ui);
        });
    };
    frames.idle(4, &mut show);
    let idle = frames.frame(vec![], &mut show);
    for _ in 0..3 {
        craft_ui::testing::contract::queue_reveal(&frames.ctx, owner, Rect::from_min_size(Pos2::new(0.0, 0.0), Vec2::new(20.0, 20.0)));
        assert_eq!(frames.frame(vec![], &mut show), idle, "a reveal of a rect in view runs no extra pass");
    }
}
