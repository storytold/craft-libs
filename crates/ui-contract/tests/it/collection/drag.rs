//! Drags in a collection: the marquee from empty space, the scroll at an edge while a drag is held, and which button
//! drags rows.

use super::kit::{Count, Dismissed, ROW_H, Row};
use super::table::Table;
use craft_behavior::{ItemSource, Selection, SelectionModel};
use craft_ui::contract::{
    CollectionEvent, CollectionProps, ContainerProps, Props as _, ReorderRule, Scroll, SelectMode, Show as _, ShowCollection as _, ShowContainer as _,
};
use craft_ui::testing::layout::{Block, in_region};
use craft_ui::testing::{Frames, pointer, trial};
use craft_ui_test_kit::body::Slab;
use egui::{Event, Modifiers, Pos2, Rect, Ui, Vec2};
use std::cell::{Cell, RefCell};

const ORIGIN: Pos2 = Pos2::new(10.0, 10.0);
const VIEW: Vec2 = Vec2::new(400.0, 240.0);
const COL: f32 = 100.0;
const PANEL: Vec2 = Vec2::new(300.0, 200.0);
/// What a drag held past an edge scrolls in one frame: four rows a second, the framework's full rate, at the 1/60 s a
/// trial frame takes.
const EDGE_STEP: f32 = 4.0 * ROW_H / 60.0;
/// Frames a drag is held at an edge while its scrolling is counted.
const HELD: usize = 6;

/// What a collection reported and where its rows were.
#[derive(Default)]
struct Scene {
    events: RefCell<Vec<CollectionEvent<usize, Dismissed>>>,
    rows: RefCell<Vec<(usize, Rect)>>,
    /// The first row the collection showed in its last frame.
    visible: Cell<usize>,
}

impl Scene {
    fn row(&self, key: usize) -> Rect {
        self.rows.borrow().iter().find(|(held, _)| *held == key).map(|(_, rect)| *rect).expect("the row was shown")
    }

    /// The selection the reported changes make, applied in order.
    fn selected(&self) -> Vec<usize> {
        let mut selection = Selection::<usize>::default();
        for event in self.events.borrow().iter() {
            if let CollectionEvent::Selection { change, .. } = event {
                selection.apply(change.clone());
            }
        }
        selection.items
    }

    /// How far the rows have scrolled, up to a constant: a row wholly in view against where its index puts it.
    fn scrolled(&self) -> f32 {
        let key = self.visible.get() + 2;
        key as f32 * ROW_H - self.row(key).min.y
    }

    fn reorders(&self) -> usize {
        self.events.borrow().iter().filter(|event| matches!(event, CollectionEvent::Reorder(_))).count()
    }
}

/// A table of `source` where `ui` is, its rows and events noted in `scene`.
fn table_in(ui: &mut Ui, id: &'static str, props: CollectionProps, source: &dyn ItemSource<usize>, scene: &Scene) {
    let out = Table::new(props).id_salt(id).show_items(ui, source, &mut |key, cx| {
        let mut rows = scene.rows.borrow_mut();
        rows.retain(|(held, _)| held != key);
        rows.push((*key, cx.rect));
        Row::new()
    });
    scene.visible.set(out.visible.start);
    scene.events.borrow_mut().extend(out.events);
}

fn show_table(ui: &mut Ui, props: CollectionProps, source: &dyn ItemSource<usize>, scene: &Scene) {
    in_region(ui, Rect::from_min_size(ORIGIN, VIEW), |ui| table_in(ui, "table", props, source, scene));
}

/// A table whose rows reorder by drag.
fn reorderable() -> CollectionProps {
    CollectionProps::new().select(SelectMode::Multi).reorder(ReorderRule::new())
}

/// A drag from empty space under the rows up to the middle of row 2 of five.
fn sweep_up(frames: &mut Frames, scene: &Scene, show: &mut impl FnMut(&mut Ui)) {
    let x = ORIGIN.x + COL / 2.0;
    let from = Pos2::new(x, scene.row(4).max.y + 40.0);
    let to = Pos2::new(x, scene.row(2).center().y);
    pointer::drag(frames, from, to - from, show);
}

/// Presses at `from`, moves to `to` over three frames and holds there for two more.
fn pull(frames: &mut Frames, from: Pos2, to: Pos2, show: &mut impl FnMut(&mut Ui)) {
    frames.frame(vec![pointer::moved(from)], &mut *show);
    frames.frame(vec![pointer::press(from)], &mut *show);
    for step in 1..=3 {
        frames.frame(vec![pointer::moved(from + (to - from) * (step as f32 / 3.0))], &mut *show);
    }
    frames.idle(2, &mut *show);
}

/// Releases at `at`, then a frame with no input.
fn let_go(frames: &mut Frames, at: Pos2, show: &mut impl FnMut(&mut Ui)) {
    frames.frame(vec![pointer::release(at)], &mut *show);
    frames.frame(Vec::new(), &mut *show);
}

/// A drag from empty space selects the rows its rectangle touches; with Shift they join the selection the drag found.
#[trial]
fn a_marquee_from_empty_space_selects_the_rows_it_touches() {
    let (scene, source) = (Scene::default(), Count(5));
    let props = || CollectionProps::new().select(SelectMode::Multi).marquee(true);
    let mut frames = Frames::new();
    let mut show = |ui: &mut Ui| show_table(ui, props(), &source, &scene);
    frames.idle(3, &mut show);
    sweep_up(&mut frames, &scene, &mut show);
    assert_eq!(scene.selected(), vec![2, 3, 4]);

    pointer::click(&mut frames, scene.row(0).center(), &mut show);
    assert_eq!(scene.selected(), vec![0]);
    frames.hold(Modifiers::SHIFT);
    sweep_up(&mut frames, &scene, &mut show);
    assert_eq!(scene.selected(), vec![0, 2, 3, 4]);
}

/// A drag held past the end of a scroll body scrolls it by the frame's time, however many passes the frame runs; a
/// table inside the body that scrolls its own rows under its own drag takes the step, so the body does not step too.
#[trial]
fn a_drag_held_at_a_scroll_bodys_edge_scrolls_it_once_per_frame() {
    let (inner, outer) = (Scene::default(), Scene::default());
    let (lines, source) = ([Cell::new(0.0f32), Cell::new(0.0f32)], Count(40));
    let (one, two, side) = (ORIGIN, ORIGIN + Vec2::new(0.0, 220.0), ORIGIN + Vec2::new(320.0, 0.0));
    let blocks = |ui: &mut Ui, n: usize| {
        for _ in 0..n {
            Block::new(Vec2::new(PANEL.x, ROW_H)).show(ui);
        }
    };
    let mut frames = Frames::new();
    let mut show = |ui: &mut Ui| {
        in_region(ui, Rect::from_min_size(one, PANEL), |ui| {
            Slab::new(ContainerProps::new().scroll(Scroll::vertical())).id_salt("one").show(ui, |ui, _| {
                lines[0].set(ui.next_widget_position().y);
                blocks(ui, 40);
            });
        });
        // The table's rows end just inside the panel's view, with more blocks below, so the pointer at the panel's
        // end is past the table's rows and in the band of a panel that can still scroll.
        in_region(ui, Rect::from_min_size(two, PANEL), |ui| {
            Slab::new(ContainerProps::new().scroll(Scroll::vertical())).id_salt("two").show(ui, |ui, _| {
                lines[1].set(ui.next_widget_position().y);
                blocks(ui, 4);
                table_in(ui, "inner", reorderable().max_rows(5), &source, &inner);
                blocks(ui, 30);
            });
        });
        in_region(ui, Rect::from_min_size(side, Vec2::new(150.0, PANEL.y)), |ui| table_in(ui, "outer", reorderable(), &Count(200), &outer));
    };
    frames.idle(3, &mut show);
    let rest = (lines[1].get(), inner.scrolled(), outer.scrolled());
    let held = |moved: f32| moved > EDGE_STEP * HELD as f32 * 0.8 && moved < EDGE_STEP * HELD as f32 * 1.2;

    // A row of the side table dragged to the first panel's end: the panel scrolls, by one step a frame.
    let end_of_one = Pos2::new(one.x + PANEL.x / 2.0, one.y + PANEL.y - 5.0);
    pull(&mut frames, outer.row(2).center(), end_of_one, &mut show);
    let before = lines[0].get();
    frames.idle(HELD, &mut show);
    let moved = before - lines[0].get();
    assert!(held(moved), "a drag held at the panel's end scrolls it {EDGE_STEP} a frame, not {} in {HELD} frames", moved);
    assert!((lines[1].get() - rest.0).abs() < 0.5 && (inner.scrolled() - rest.1).abs() < 0.5, "the other panel and its table stay");
    assert!((outer.scrolled() - rest.2).abs() < 0.5, "the pointer is beside the side table's columns: it stays");
    let_go(&mut frames, end_of_one, &mut show);

    // A row of the table inside the second panel dragged to the panel's end: the table scrolls, the panel does not.
    let end_of_two = Pos2::new(two.x + PANEL.x / 2.0, two.y + PANEL.y - 5.0);
    pull(&mut frames, inner.row(2).center(), end_of_two, &mut show);
    let before = inner.scrolled();
    frames.idle(HELD, &mut show);
    let moved = inner.scrolled() - before;
    assert!(held(moved), "the table scrolls its own rows {EDGE_STEP} a frame, not {moved} in {HELD} frames");
    assert!((lines[1].get() - rest.0).abs() < 0.5, "the panel around the table does not step too: {} -> {}", rest.0, lines[1].get());
}

/// A press-drag with the secondary or the middle button on a row moves nothing; the primary one reorders.
#[trial]
fn only_the_primary_button_drags_rows() {
    let (scene, source) = (Scene::default(), Count(5));
    let mut frames = Frames::new();
    let mut show = |ui: &mut Ui| show_table(ui, reorderable(), &source, &scene);
    frames.idle(3, &mut show);
    let from = scene.row(1).center();
    let to = Pos2::new(from.x, scene.row(3).max.y - 2.0);
    for (name, button) in [("secondary", pointer::secondary as fn(Pos2, bool) -> Event), ("middle", pointer::middle)] {
        frames.frame(vec![pointer::moved(from)], &mut show);
        frames.frame(vec![button(from, true)], &mut show);
        for step in 1..=3 {
            frames.frame(vec![pointer::moved(from + (to - from) * (step as f32 / 3.0))], &mut show);
        }
        frames.frame(vec![button(to, false)], &mut show);
        frames.idle(2, &mut show);
        assert_eq!(scene.reorders(), 0, "a {name} drag on a row reorders nothing");
    }
    pointer::drag(&mut frames, from, to - from, &mut show);
    assert_eq!(scene.reorders(), 1, "the primary drag moves the row");
}

/// How far a table of many rows scrolls while a row dragged from it is held `x` across, below its last visible row.
fn held_below_at(x: f32) -> f32 {
    let (scene, source) = (Scene::default(), Count(200));
    let mut frames = Frames::new();
    let mut show = |ui: &mut Ui| show_table(ui, reorderable(), &source, &scene);
    frames.idle(3, &mut show);
    let rest = scene.scrolled();
    pull(&mut frames, scene.row(2).center(), Pos2::new(x, ORIGIN.y + VIEW.y + 10.0), &mut show);
    frames.idle(HELD, &mut show);
    scene.scrolled() - rest
}

/// A drag held past the rows' end scrolls them while the pointer is over their columns and not while it is beside them.
#[trial]
fn a_row_held_beside_the_list_scrolls_nothing() {
    let over = held_below_at(ORIGIN.x + COL / 2.0);
    assert!(over > EDGE_STEP * HELD as f32 * 0.8, "held over the columns the rows scroll: {over}");
    let beside = held_below_at(ORIGIN.x + VIEW.x + 30.0);
    assert!(beside.abs() < 0.5, "held beside the columns the rows stay: {beside}");
}
