//! A scrolling body: the wheel and the thumb move it in the frame that carries them, and its content size and
//! scroll offset follow the content.

use craft_behavior::ColumnWidth;
use craft_ui::buttons::Button;
use craft_ui::contract::{ContainerProps, Props as _, Scroll, Show as _, ShowContainer as _, Sizing};
use craft_ui::layout_components::Grid;
use craft_ui::runtime::focus::{FocusTarget, request};
use craft_ui::testing::layout::{Block, in_region};
use craft_ui::testing::{Frames, pointer, trial};
use craft_ui_test_kit::body::Slab;
use egui::{Id, Pos2, Rect, Ui, Vec2};
use std::cell::{Cell, RefCell};

const LINE: Vec2 = Vec2::new(100.0, 20.0);
/// Positions that should coincide, and a looser bound for positions that follow a drag.
const SAME: f32 = 0.5;
const BIG_STEP: f32 = 200.0;
const HUNDRED: f32 = 100.0;
/// How many view heights a thumb drag of 60 points over thirty 100-point children moves the content, at least.
const JUMP_VIEWS: f32 = 3.0;
/// What the gaps between lines add to a body's height, at most.
const GAPS: f32 = 20.0;
const VIEW: Vec2 = Vec2::new(300.0, 200.0);
const ORIGIN: Pos2 = Pos2::new(10.0, 10.0);

/// What a frame of the body saw.
#[derive(Default)]
struct Seen {
    first_top: Cell<f32>,
    /// The height of the content, gaps included.
    content: Cell<f32>,
    body: Cell<Option<Rect>>,
}

fn wheel_at(frames: &mut Frames, at: Pos2, dy: f32, show: &mut impl FnMut(&mut Ui)) {
    frames.frame(vec![pointer::moved(at)], &mut *show);
    frames.frame(vec![pointer::wheel(dy)], &mut *show);
}

/// Children of a scrolling body that run only in view (`Lazy`): one far below the view does not run and keeps the
/// body's content size; scrolled into view, it runs again. A thumb drag that jumps several views shows children that
/// ran in that frame. Guards the out-of-view rule, its one store, and that the decision sees the painted offset.
#[trial]
fn an_out_of_view_child_does_not_run_until_it_scrolls_in() {
    const CHILDREN: usize = 30;
    let runs: Vec<Cell<usize>> = (0..CHILDREN).map(|_| Cell::new(0)).collect();
    let seen = Seen::default();
    // The rects of the children the last pass ran for real.
    let shown = RefCell::new(Vec::new());
    let mut show = |ui: &mut Ui| {
        shown.borrow_mut().clear();
        in_region(ui, Rect::from_min_size(ORIGIN, VIEW), |ui| {
            Slab::new(ContainerProps::new().scroll(Scroll::vertical())).id_salt("panel").show(ui, |ui, cx| {
                seen.body.set(Some(cx.rect));
                seen.first_top.set(ui.next_widget_position().y);
                for (at, count) in runs.iter().enumerate() {
                    craft_ui::contract::Lazy::new(at).show(ui, |ui| {
                        count.set(count.get() + 1);
                        let rect = Block::new(Vec2::new(LINE.x, HUNDRED)).show(ui).rect;
                        if !ui.is_sizing_pass() {
                            shown.borrow_mut().push(rect);
                        }
                    });
                }
                seen.content.set(ui.min_rect().height());
            });
        });
    };
    let mut frames = Frames::new();
    frames.idle(3, &mut show);
    let full = seen.content.get();
    let last = CHILDREN - 1;
    runs.iter().for_each(|count| count.set(0));
    frames.frame(vec![], &mut show);
    assert!(runs[0].get() > 0, "the child in view runs");
    assert_eq!(runs[last].get(), 0, "the child far below the view does not run");
    assert!((seen.content.get() - full).abs() <= SAME, "the content keeps its size: {} then {full}", seen.content.get());
    let view = seen.body.get().expect("the body ran");
    for _ in 0..CHILDREN {
        wheel_at(&mut frames, view.center(), -BIG_STEP, &mut show);
    }
    runs.iter().for_each(|count| count.set(0));
    frames.frame(vec![], &mut show);
    assert!(runs[last].get() > 0, "scrolled to the end, the last child runs");
    assert_eq!(runs[0].get(), 0, "and the first, far above the view, does not");
    assert!((seen.content.get() - full).abs() <= SAME, "the content keeps its size at the end");

    // Back at the top, a thumb drag of 60 points moves the content by several views in one frame.
    for _ in 0..CHILDREN {
        wheel_at(&mut frames, view.center(), BIG_STEP, &mut show);
    }
    let top0 = seen.first_top.get();
    let thumb = Pos2::new(view.max.x - 2.0, view.min.y + 4.0);
    frames.frame(vec![pointer::moved(thumb)], &mut show);
    frames.frame(vec![pointer::press(thumb)], &mut show);
    frames.frame(vec![pointer::moved(thumb + Vec2::new(0.0, 4.0))], &mut show);
    frames.frame(vec![pointer::moved(thumb + Vec2::new(0.0, 8.0))], &mut show);
    let before = top0 - seen.first_top.get();
    frames.frame(vec![pointer::moved(thumb + Vec2::new(0.0, 60.0))], &mut show);
    let jumped = top0 - seen.first_top.get();
    assert!(jumped - before > JUMP_VIEWS * view.height(), "the drag jumped several views: {before} -> {jumped}");
    let mut spans: Vec<(f32, f32)> = shown.borrow().iter().map(|rect| (rect.min.y, rect.max.y)).collect();
    spans.sort_by(|a, b| a.0.total_cmp(&b.0));
    let mut covered = view.min.y;
    for (top, bottom) in spans {
        if top <= covered + GAPS {
            covered = covered.max(bottom);
        }
    }
    assert!(covered >= view.max.y, "the children in view after the jump ran in that frame: covered to {covered}, view {view:?}");
    frames.frame(vec![pointer::release(thumb + Vec2::new(0.0, 60.0))], &mut show);
}

/// A focus request for a control in a child far out of view runs that child, keeps the focus there, and the body
/// scrolls the control into view: the child's stops from its last run name it.
#[trial]
fn a_focus_request_into_an_out_of_view_child_runs_it_and_scrolls_it_in() {
    const CHILDREN: usize = 30;
    let runs: Vec<Cell<usize>> = (0..CHILDREN).map(|_| Cell::new(0)).collect();
    let buttons: Vec<Cell<(Id, Rect)>> = (0..CHILDREN).map(|_| Cell::new((Id::NULL, Rect::NOTHING))).collect();
    let view = Cell::new(Rect::NOTHING);
    let mut show = |ui: &mut Ui| {
        in_region(ui, Rect::from_min_size(ORIGIN, VIEW), |ui| {
            Slab::new(ContainerProps::new().scroll(Scroll::vertical())).id_salt("panel").show(ui, |ui, cx| {
                view.set(cx.rect);
                for (at, (count, button)) in runs.iter().zip(&buttons).enumerate() {
                    craft_ui::contract::Lazy::new(at).show(ui, |ui| {
                        count.set(count.get() + 1);
                        let out = Button::new(format!("Child {at}")).show(ui);
                        if !ui.is_sizing_pass() {
                            button.set((out.id, out.rect));
                        }
                        Block::new(Vec2::new(LINE.x, HUNDRED)).show(ui);
                    });
                }
            });
        });
    };
    let mut frames = Frames::new();
    frames.idle(3, &mut show);
    let last = CHILDREN - 1;
    runs.iter().for_each(|count| count.set(0));
    frames.frame(vec![], &mut show);
    assert_eq!(runs[last].get(), 0, "the child far below the view does not run");
    let target = buttons[last].get().0;
    request(&frames.ctx, FocusTarget::Id(target));
    frames.idle(3, &mut show);
    assert!(runs[last].get() > 0, "the requested child runs");
    assert_eq!(frames.ctx.memory(|memory| memory.focused()), Some(target), "the focus stays on the requested control");
    let (_, rect) = buttons[last].get();
    assert!(view.get().contains_rect(rect), "the body scrolled the control into view: {rect:?} in {:?}", view.get());
}

/// A child out of view runs nowhere, yet its stops keep their place in Tab order: a settled frame runs only the
/// child in view, and Tab from its control moves focus to the control of the next child, which then runs and scrolls
/// into view. Guards that a skipped `Lazy` registers the focus records of its last run in its place.
#[trial]
fn tab_reaches_the_control_of_a_child_out_of_view() {
    const CHILDREN: usize = 6;
    let runs: Vec<Cell<usize>> = (0..CHILDREN).map(|_| Cell::new(0)).collect();
    let buttons: Vec<Cell<(Id, Rect)>> = (0..CHILDREN).map(|_| Cell::new((Id::NULL, Rect::NOTHING))).collect();
    let view = Cell::new(Rect::NOTHING);
    let mut show = |ui: &mut Ui| {
        in_region(ui, Rect::from_min_size(ORIGIN, VIEW), |ui| {
            Slab::new(ContainerProps::new().scroll(Scroll::vertical())).id_salt("panel").show(ui, |ui, cx| {
                view.set(cx.rect);
                for (at, (count, button)) in runs.iter().zip(&buttons).enumerate() {
                    craft_ui::contract::Lazy::new(at).show(ui, |ui| {
                        count.set(count.get() + 1);
                        let out = Button::new(format!("Child {at}")).show(ui);
                        if !ui.is_sizing_pass() {
                            button.set((out.id, out.rect));
                        }
                        // Each child is taller than the view, so the next one starts out of view.
                        Block::new(Vec2::new(LINE.x, VIEW.y)).show(ui);
                    });
                }
            });
        });
    };
    let mut frames = Frames::new();
    frames.idle(3, &mut show);
    request(&frames.ctx, FocusTarget::Id(buttons[0].get().0));
    frames.idle(3, &mut show);
    runs.iter().for_each(|count| count.set(0));
    frames.frame(vec![], &mut show);
    assert!(runs[0].get() > 0, "the child in view runs");
    assert!(runs[1..].iter().all(|count| count.get() == 0), "no child out of view runs: {:?}", runs.iter().map(Cell::get).collect::<Vec<_>>());
    let next = buttons[1].get().0;
    frames.frame(vec![craft_ui::testing::input::key_press(egui::Key::Tab, egui::Modifiers::NONE)], &mut show);
    frames.frame(vec![craft_ui::testing::focus::key_release(egui::Key::Tab, egui::Modifiers::NONE)], &mut show);
    frames.idle(2, &mut show);
    assert_eq!(frames.ctx.memory(|memory| memory.focused()), Some(next), "Tab moved focus to the next child's control");
    let (_, rect) = buttons[1].get();
    assert!(view.get().contains_rect(rect), "the body scrolled the control into view: {rect:?} in {:?}", view.get());
}

/// Lazy cards in the rows of a grid, each keyed by itself: a change of the column count regroups the rows and runs
/// only the cards in view, the cards out of view keeping their extents; the rows in view are as tall as their
/// tallest card. Guards that a `Lazy` in a `Grid` holds its cell, run or skipped.
#[trial]
fn a_column_change_of_lazy_grid_cells_runs_only_the_cells_in_view() {
    const CHILDREN: usize = 30;
    const ONE: &[ColumnWidth] = &[ColumnWidth::fill(1.0)];
    const TWO: &[ColumnWidth] = &[ColumnWidth::fill(1.0); 2];
    let runs: Vec<Cell<usize>> = (0..CHILDREN).map(|_| Cell::new(0)).collect();
    let columns = Cell::new(2);
    // The rects of the cards the last pass ran for real.
    let shown = RefCell::new(Vec::new());
    let mut show = |ui: &mut Ui| {
        shown.borrow_mut().clear();
        in_region(ui, Rect::from_min_size(ORIGIN, VIEW), |ui| {
            Slab::new(ContainerProps::new().scroll(Scroll::vertical())).id_salt("panel").show(ui, |ui, _| {
                let count = columns.get();
                for (row, chunk) in runs.chunks(count).enumerate() {
                    Grid::new(if count == 1 { ONE } else { TWO }).id_salt(("row", row)).show(ui, |ui, _| {
                        for (at, runs) in chunk.iter().enumerate() {
                            let index = row * count + at;
                            craft_ui::contract::Lazy::new(index).show(ui, |ui| {
                                runs.set(runs.get() + 1);
                                // Cards of one row differ in height.
                                let rect = Block::new(Vec2::new(LINE.x, HUNDRED + GAPS * (index % 3) as f32)).show(ui).rect;
                                if !ui.is_sizing_pass() {
                                    shown.borrow_mut().push(rect);
                                }
                            });
                        }
                    });
                }
            });
        });
    };
    let mut frames = Frames::new();
    frames.idle(3, &mut show);
    for count in [1, 2] {
        columns.set(count);
        runs.iter().for_each(|runs| runs.set(0));
        frames.frame(vec![], &mut show);
        let ran = runs.iter().filter(|runs| runs.get() > 0).count();
        assert!(runs[0].get() > 0, "at {count} columns the card in view runs");
        assert_eq!(runs[CHILDREN - 1].get(), 0, "at {count} columns the card far below the view does not run");
        assert!(ran < CHILDREN / 2, "at {count} columns only the cards in view run: {ran} of {CHILDREN}");
        let shown = shown.borrow();
        for (i, a) in shown.iter().enumerate() {
            for b in &shown[i + 1..] {
                assert!(!a.shrink(SAME).intersects(b.shrink(SAME)), "at {count} columns the cards that ran overlap: {a:?} and {b:?}");
            }
        }
    }
}

/// Regression: a Lazy card that fills its grid cell took the cell the row's kept measure gave it, so content that grew
/// inside it stayed clipped at the old row height. A change inside a Lazy reaches the bodies around it: the row grows
/// to the card's new content in the frame of the change.
#[trial]
fn content_that_grows_inside_a_lazy_cell_that_fills_it_grows_its_row() {
    const TWO: &[ColumnWidth] = &[ColumnWidth::fill(1.0); 2];
    let tall = Cell::new(LINE.y);
    let card = Cell::new(Rect::NOTHING);
    let content = Cell::new(Rect::NOTHING);
    let mut show = |ui: &mut Ui| {
        in_region(ui, Rect::from_min_size(ORIGIN, VIEW), |ui| {
            Slab::new(ContainerProps::new().scroll(Scroll::vertical())).id_salt("panel").show(ui, |ui, _| {
                Grid::new(TWO).show(ui, |ui, _| {
                    for index in 0..2 {
                        craft_ui::contract::Lazy::new(index).show(ui, |ui| {
                            let filled = Slab::new(ContainerProps::new().sizing(Sizing::Fill).scroll(Scroll::vertical()))
                                .show(ui, |ui, _| Block::new(Vec2::new(LINE.x, if index == 0 { tall.get() } else { LINE.y })).show(ui).rect);
                            if let (0, false, Some(rect)) = (index, ui.is_sizing_pass(), filled.inner) {
                                card.set(filled.output.rect);
                                content.set(rect);
                            }
                        });
                    }
                });
            });
        });
    };
    let mut frames = Frames::new();
    frames.idle(3, &mut show);
    assert!(card.get().contains_rect(content.get()), "the card holds its content: {:?} in {:?}", content.get(), card.get());
    tall.set(HUNDRED);
    frames.frame(vec![], &mut show);
    assert!(content.get().height() > LINE.y + SAME, "the content grew: {:?}", content.get());
    assert!(card.get().expand(SAME).contains_rect(content.get()), "the card grew to its content: {:?} in {:?}", content.get(), card.get());
}
