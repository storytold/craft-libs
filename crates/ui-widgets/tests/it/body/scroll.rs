//! A scrolling body: the wheel and the thumb move it in the frame that carries them, and its content size and
//! scroll offset follow the content.

use craft_behavior::{ColumnWidth, PartKey};
use craft_ui::buttons::Button;
use craft_ui::contract::{Bind, ContainerProps, MemoProps, MemoView, Props as _, Scroll, Show as _, ShowContainer as _, Sizing, Trigger};
use craft_ui::layout_components::Grid;
use craft_ui::menu::{Menu, MenuItem};
use craft_ui::runtime::focus::{FocusTarget, request};
use craft_ui::testing::layout::{Block, in_region, measured_block};
use craft_ui::testing::text::widget_id;
use craft_ui::testing::{Fixture, Frames, MemoRun, last_frame_memo_runs, pointer, trial};
use craft_ui::text::TextField;
use craft_ui_test_kit::body::Slab;
use egui::{Id, Pos2, Rect, Ui, Vec2};
use std::cell::{Cell, RefCell};
use std::collections::BTreeMap;
use std::rc::Rc;

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
/// How far a wheel step reaches into the child above the view.
const STEP: f32 = 10.0;
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
                    craft_ui::contract::Lazy::new(at, HUNDRED).show(ui, |ui| {
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
    // Every child runs once first, in a room that shows them all, so each has stops to name.
    let room = Cell::new(Vec2::new(VIEW.x, CHILDREN as f32 * 2.0 * HUNDRED));
    let mut show = |ui: &mut Ui| {
        in_region(ui, Rect::from_min_size(ORIGIN, room.get()), |ui| {
            Slab::new(ContainerProps::new().scroll(Scroll::vertical())).id_salt("panel").show(ui, |ui, cx| {
                view.set(cx.rect);
                for (at, (count, button)) in runs.iter().zip(&buttons).enumerate() {
                    craft_ui::contract::Lazy::new(at, HUNDRED).show(ui, |ui| {
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
    frames.screen(room.get());
    frames.idle(3, &mut show);
    assert!(runs.iter().all(|count| count.get() > 0), "every child ran in the tall room");
    room.set(VIEW);
    frames.idle(1, &mut show);
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
    // Every child runs once first, in a room that shows them all, so each has stops to name.
    let room = Cell::new(Vec2::new(VIEW.x, CHILDREN as f32 * 2.0 * VIEW.y));
    let mut show = |ui: &mut Ui| {
        in_region(ui, Rect::from_min_size(ORIGIN, room.get()), |ui| {
            Slab::new(ContainerProps::new().scroll(Scroll::vertical())).id_salt("panel").show(ui, |ui, cx| {
                view.set(cx.rect);
                for (at, (count, button)) in runs.iter().zip(&buttons).enumerate() {
                    craft_ui::contract::Lazy::new(at, HUNDRED).show(ui, |ui| {
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
    frames.screen(room.get());
    frames.idle(3, &mut show);
    assert!(runs.iter().all(|count| count.get() > 0), "every child ran in the tall room");
    room.set(VIEW);
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

/// A scroll region inside another: focus arriving at a control out of the inner region's view, which lies itself out
/// of the outer one's, scrolls the inner region to the control and the outer one to the part the inner one shows.
#[trial]
fn focus_out_of_view_in_a_nested_scroll_region_scrolls_both_regions_to_it() {
    const BUTTONS: usize = 6;
    let buttons: Vec<Cell<(Id, Rect)>> = (0..BUTTONS).map(|_| Cell::new((Id::NULL, Rect::NOTHING))).collect();
    let (outer, inner) = (Cell::new(Rect::NOTHING), Cell::new(Rect::NOTHING));
    let mut show = |ui: &mut Ui| {
        in_region(ui, Rect::from_min_size(ORIGIN, VIEW), |ui| {
            Slab::new(ContainerProps::new().scroll(Scroll::vertical()).room_height(VIEW.y / 2.0)).id_salt("outer").show(ui, |ui, cx| {
                outer.set(cx.rect);
                Block::new(Vec2::new(LINE.x, 2.0 * VIEW.y)).show(ui);
                Slab::new(ContainerProps::new().scroll(Scroll::vertical())).id_salt("inner").show(ui, |ui, cx| {
                    inner.set(cx.rect);
                    for button in &buttons {
                        let out = Button::new("Child").show(ui);
                        if !ui.is_sizing_pass() {
                            button.set((out.id, out.rect));
                        }
                        Block::new(Vec2::new(LINE.x, HUNDRED)).show(ui);
                    }
                });
            });
        });
    };
    let mut frames = Frames::new();
    frames.screen(VIEW + ORIGIN.to_vec2());
    frames.idle(3, &mut show);
    let (target, rect) = buttons[BUTTONS - 2].get();
    assert!(!outer.get().intersects(rect), "the control starts out of view");
    request(&frames.ctx, FocusTarget::Id(target));
    frames.idle(4, &mut show);
    let (_, rect) = buttons[BUTTONS - 2].get();
    assert!(inner.get().contains_rect(rect), "the inner region shows the control: {rect:?} in {:?}", inner.get());
    assert!(outer.get().contains_rect(rect), "the outer region shows it too: {rect:?} in {:?}", outer.get());
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
                            craft_ui::contract::Lazy::new(index, HUNDRED).show(ui, |ui| {
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
                        craft_ui::contract::Lazy::new(index, LINE.y).show(ui, |ui| {
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

/// A memoized line as wide as its view.
#[derive(MemoProps)]
struct Line {
    at: usize,
}

fn line(props: &mut Line, ui: &mut Ui) -> usize {
    Block::new(Vec2::new(ui.available_width(), LINE.y)).show(ui);
    props.at
}

/// A memoized block whose measured width is `width`: a new width asks the body around it to measure again.
#[derive(MemoProps)]
struct Grown {
    width: f32,
}

fn grown(props: &mut Grown, ui: &mut Ui) -> f32 {
    measured_block(ui, "grown", Vec2::new(props.width, LINE.y));
    Block::new(Vec2::new(props.width, LINE.y)).show(ui);
    props.width
}

/// Count trial: an overflowing scrolling body's measuring run lays its content out once, at the view beside the bar.
/// In a pass that measures the body again, its settled memoized children in view replay in the measuring run as in
/// the real run: none runs a measuring run at another width.
#[trial]
fn an_overflowing_bodys_settled_memo_children_run_no_measuring_run_when_it_measures_again() {
    let width = Cell::new(40.0);
    let mut show = |ui: &mut Ui| {
        in_region(ui, Rect::from_min_size(ORIGIN, VIEW), |ui| {
            Slab::new(ContainerProps::new().scroll(Scroll::vertical())).id_salt("memo_lines").show(ui, |ui, _| {
                MemoView::new("grown", Grown { width: width.get() }, grown).show(ui);
                for at in 0..30 {
                    MemoView::new(("line", at), Line { at }, line).show(ui);
                }
            });
        });
    };
    let mut frames = Frames::new();
    frames.idle(5, &mut show);
    width.set(60.0);
    frames.idle(1, &mut show);
    let runs = last_frame_memo_runs(&frames.ctx);
    let ran = |name: &str| runs.iter().filter(|(of, _, run)| of.contains(name) && *run == MemoRun::Ran).count();
    assert!(ran("::Grown") >= 2, "the changed view ran in the body's measuring run and its real run: {runs:?}");
    assert!(runs.iter().any(|(of, _, run)| of.contains("::Line") && *run == MemoRun::Replayed), "the lines in view took part: {runs:?}");
    assert_eq!(ran("::Line"), 0, "no settled line ran, in a measuring run or the real run: {runs:?}");
}

/// A vertical scroll body of `Lazy` children, child `at` as tall as `height(at)` and estimated at `estimate(at)`
/// before it ran. It counts each child's runs and keeps the rects of the children the last real pass ran.
struct LazyColumn {
    runs: Vec<Cell<usize>>,
    shown: RefCell<Vec<(usize, Rect)>>,
    view: Cell<Rect>,
    /// The child the next frames ask to reveal.
    reveal: Cell<Option<usize>>,
    /// Whether the body shows at all: hidden, as a tab switched away is.
    visible: Cell<bool>,
}

impl LazyColumn {
    fn new(children: usize) -> Self {
        LazyColumn {
            runs: (0..children).map(|_| Cell::new(0)).collect(),
            shown: RefCell::new(Vec::new()),
            view: Cell::new(Rect::NOTHING),
            reveal: Cell::new(None),
            visible: Cell::new(true),
        }
    }

    fn show(&self, ui: &mut Ui, height: impl Fn(usize) -> f32, estimate: impl Fn(usize) -> f32) {
        self.shown.borrow_mut().clear();
        if !self.visible.get() {
            return;
        }
        in_region(ui, Rect::from_min_size(ORIGIN, VIEW), |ui| {
            Slab::new(ContainerProps::new().scroll(Scroll::vertical())).id_salt("column").show(ui, |ui, cx| {
                self.view.set(cx.rect);
                for (at, count) in self.runs.iter().enumerate() {
                    craft_ui::contract::Lazy::new(at, estimate(at)).reveal(self.reveal.get() == Some(at)).show(ui, |ui| {
                        count.set(count.get() + 1);
                        let rect = Block::new(Vec2::new(LINE.x, height(at))).show(ui).rect;
                        if !ui.is_sizing_pass() && !craft_ui::testing::is_measuring(ui.ctx()) {
                            self.shown.borrow_mut().push((at, rect));
                        }
                    });
                }
            });
        });
    }

    /// The children that ran since the counts were last cleared, in order.
    fn ran(&self) -> Vec<usize> {
        (0..self.runs.len()).filter(|&at| self.runs[at].get() > 0).collect()
    }

    fn clear(&self) {
        self.runs.iter().for_each(|count| count.set(0));
    }

    /// The rect each child took in the last real run of the last pass that ran it.
    fn rects(&self) -> BTreeMap<usize, Rect> {
        self.shown.borrow().iter().copied().collect()
    }

    /// The first child the last real pass ran whose rect reaches into the view, with its rect.
    fn top(&self) -> Option<(usize, Rect)> {
        let view = self.view.get();
        self.rects().into_iter().find(|(_, rect)| rect.max.y > view.min.y)
    }
}

/// Real heights that differ from child to child.
fn varied(at: usize) -> f32 {
    HUNDRED + GAPS * (at % 3) as f32
}

/// The first show of a scroll body runs only the children in view: the children past the view take their estimates,
/// whether those are short or long. Guards that a child never shown is placed by its estimate, not run.
#[trial]
fn a_first_show_runs_only_the_lazy_children_in_view() {
    const CHILDREN: usize = 40;
    let column = LazyColumn::new(CHILDREN);
    let mut show = |ui: &mut Ui| column.show(ui, varied, |at| if at % 2 == 0 { HUNDRED / 2.0 } else { 2.0 * HUNDRED });
    let mut frames = Frames::new();
    frames.idle(3, &mut show);
    let ran = column.ran();
    assert!(ran.len() < CHILDREN / 2, "only the children in view ran: {ran:?}");
    assert_eq!(ran, (0..ran.len()).collect::<Vec<_>>(), "the children that ran are the first ones");
    let bottom = column.rects().values().map(|rect| rect.max.y).fold(f32::MIN, f32::max);
    assert!(bottom >= column.view.get().max.y, "the children that ran fill the view: down to {bottom}, view {:?}", column.view.get());
}

/// A scroll body hidden for a while (a tab switched away) and shown again runs only the children in view, and shows
/// the same content where it was: the children keep their extents while the body is hidden, and its scroll position
/// names the same child. Guards the kept entries of a scroll area that does not run.
#[trial]
fn a_scroll_area_shown_again_runs_only_the_children_in_view() {
    const CHILDREN: usize = 40;
    let column = LazyColumn::new(CHILDREN);
    let mut show = |ui: &mut Ui| column.show(ui, varied, |_| HUNDRED / 2.0);
    let mut frames = Frames::new();
    frames.idle(3, &mut show);
    let view = column.view.get();
    for _ in 0..8 {
        wheel_at(&mut frames, view.center(), -BIG_STEP, &mut show);
    }
    frames.idle(2, &mut show);
    let (child, rect) = column.top().expect("a child in view");
    assert!(child > 0, "the wheel moved the content");
    column.visible.set(false);
    frames.idle(4, &mut show);
    // Past the time the framework keeps a hidden component's shown memory.
    frames.advance(2.0);
    frames.idle(4, &mut show);
    column.visible.set(true);
    column.clear();
    frames.frame(vec![], &mut show);
    let shown = column.rects();
    for at in column.ran() {
        let ran = shown.get(&at).unwrap_or_else(|| panic!("child {at} ran in the measuring run only"));
        assert!(ran.intersects(view), "only children in view ran: child {at} at {ran:?}, view {view:?}");
    }
    let (again, at) = column.top().expect("a child in view");
    assert_eq!(again, child, "the same child is at the top of the view");
    assert!((at.min - rect.min).length() <= SAME, "and at the same place: {at:?}, then {rect:?}");
}

/// A child above the view that runs for the first time, as a wheel step up brings it in, is taller than its estimate:
/// the children in view move only by the wheel's step, in the frame it runs, and the frame after runs no
/// extra pass. Guards the scroll anchor: the scroll position follows the first child at or after the view's start.
#[trial]
fn content_in_view_stays_when_an_estimate_above_it_resolves() {
    const CHILDREN: usize = 40;
    let column = LazyColumn::new(CHILDREN);
    let mut show = |ui: &mut Ui| column.show(ui, |_| HUNDRED, |_| HUNDRED / 2.0);
    let mut frames = Frames::new();
    frames.idle(3, &mut show);
    let view = column.view.get();
    // A thumb drag jumps past children that never ran.
    let thumb = Pos2::new(view.max.x - 2.0, view.min.y + 4.0);
    frames.frame(vec![pointer::moved(thumb)], &mut show);
    frames.frame(vec![pointer::press(thumb)], &mut show);
    frames.frame(vec![pointer::moved(thumb + Vec2::new(0.0, 4.0))], &mut show);
    frames.frame(vec![pointer::moved(thumb + Vec2::new(0.0, 60.0))], &mut show);
    frames.frame(vec![pointer::release(thumb + Vec2::new(0.0, 60.0))], &mut show);
    frames.idle(2, &mut show);
    let before = column.rects();
    // Where a child starts: the rects are clipped to the view, and one clipped at its start ends a child's height
    // after it.
    let start = |rect: &Rect| if rect.min.y <= view.min.y + SAME { rect.max.y - HUNDRED } else { rect.min.y };
    let (first, top) = column.top().expect("a child in view");
    let above = first.checked_sub(1).expect("the drag moved past the first children");
    assert_eq!(column.runs[above].get(), 0, "child {above} above the view never ran");
    // Up past the start of the first child in view, into the one above it.
    let wheel = view.min.y - start(&top) + STEP;
    frames.frame(vec![pointer::moved(view.center())], &mut show);
    let idle = frames.frame(vec![], &mut show);
    let resolved = frames.frame(vec![pointer::wheel(wheel)], &mut show);
    assert!(column.runs[above].get() > 0, "the child the wheel brought in ran");
    let after = column.rects();
    let moved: Vec<(usize, f32)> = before.iter().filter_map(|(at, rect)| Some((*at, start(after.get(at)?) - start(rect)))).collect();
    let &(_, step) = moved.first().expect("children in view ran in both frames");
    for (at, by) in &moved {
        assert!((by - step).abs() <= SAME, "child {at} moved with the others: {by}, then {step}");
    }
    // Without the anchor the content would move by the wheel and by what the estimate above it grew, half a child.
    assert!((step - wheel).abs() < HUNDRED / 4.0, "the content moved by the wheel's {wheel}, not by the resolved estimate: {step}");
    let next = frames.frame(vec![], &mut show);
    assert!(resolved > idle, "the frame the estimate resolved in ran the pass again at the anchored offset");
    assert_eq!(next, idle, "an idle frame after it runs no extra pass");
}

/// A reveal of a child far below the view that never ran glides the body to it: the frames between show children
/// between the start and the child at the top, each frame runs only the children in view, and the child ends at the
/// view's start. Guards `Lazy::reveal`: a skipped child placed by its estimate is a target, and the glide follows
/// its place while the estimates before it resolve.
#[trial]
fn a_revealed_child_glides_to_the_view_start() {
    const CHILDREN: usize = 40;
    // Frames the glide may take: two seconds at 60 Hz.
    const GLIDE_FRAMES: usize = 120;
    let column = LazyColumn::new(CHILDREN);
    let mut show = |ui: &mut Ui| column.show(ui, varied, |_| HUNDRED / 2.0);
    let mut frames = Frames::new();
    frames.idle(3, &mut show);
    let target = CHILDREN - 6;
    assert_eq!(column.runs[target].get(), 0, "the child far below the view never ran");
    column.reveal.set(Some(target));
    frames.frame(vec![], &mut show);
    column.reveal.set(None);
    let mut between = 0;
    let mut arrived = false;
    for _ in 0..GLIDE_FRAMES {
        column.clear();
        frames.frame(vec![], &mut show);
        let view = column.view.get();
        let rects = column.rects();
        for at in column.ran() {
            let rect = rects.get(&at).expect("a child that ran has a rect");
            assert!(rect.intersects(view), "frame ran child {at} out of view: {rect:?} in {view:?}");
        }
        let (top, rect) = column.top().expect("a child in view");
        if top == target && (rect.min.y - view.min.y).abs() <= SAME {
            arrived = true;
            break;
        }
        if top > 0 && top < target {
            between += 1;
        }
    }
    assert!(arrived, "the child reached the view's start");
    assert!(between > 1, "the glide showed offsets between the start and the child: {between} frames");
}

/// A press on a field the view shows only in part focuses it where it lies: the body moves no content under the held
/// press. Tab to the same field from one in view scrolls it wholly into view. Guards that a reveal follows a focus
/// request or a key move, never a press (a crop frame overhanging its canvas panned the view under the press).
#[trial]
fn a_pressed_field_in_part_view_stays_put_and_tab_to_it_reveals_it() {
    let fields: [Cell<(Id, Rect)>; 2] = [Cell::new((Id::NULL, Rect::NOTHING)), Cell::new((Id::NULL, Rect::NOTHING))];
    let view = Cell::new(Rect::NOTHING);
    // The block between the fields, sized after the first frames so the second field crosses the view's bottom edge.
    let gap = Cell::new(VIEW.y / 2.0);
    let mut show = |ui: &mut Ui| {
        in_region(ui, Rect::from_min_size(ORIGIN, VIEW), |ui| {
            Slab::new(ContainerProps::new().scroll(Scroll::vertical())).id_salt("panel").show(ui, |ui, cx| {
                view.set(cx.rect);
                for (at, field) in fields.iter().enumerate() {
                    if at == 1 {
                        Block::new(Vec2::new(LINE.x, gap.get())).show(ui);
                    }
                    let out = TextField::new(format!("Field {at}"), Bind::given(String::new())).id_salt(("field", at)).show(ui);
                    if !ui.is_sizing_pass() {
                        // Focus lands on the field's text part.
                        field.set((widget_id(out.response.id, PartKey::new("text")), out.response.rect));
                    }
                }
                Block::new(Vec2::new(LINE.x, VIEW.y)).show(ui);
            });
        });
    };
    let mut frames = Frames::new();
    frames.screen(VIEW + ORIGIN.to_vec2() * 2.0);
    frames.idle(3, &mut show);
    // A field's response rect is clipped to the view, so the fields' full height is the first one's, wholly in view.
    let height = fields[0].get().1.height();
    let (_, second) = fields[1].get();
    gap.set(gap.get() + view.get().bottom() - height / 2.0 - second.top());
    frames.idle(3, &mut show);
    let (second_id, second) = fields[1].get();
    assert!(
        second.top() < view.get().bottom() && second.top() + height > view.get().bottom(),
        "the second field shows in part: {second:?} ({height} tall) in {:?}",
        view.get()
    );
    pointer::click(&mut frames, Pos2::new(second.center().x, (second.top() + view.get().bottom()) / 2.0), &mut show);
    frames.idle(2, &mut show);
    assert_eq!(frames.ctx.memory(|memory| memory.focused()), Some(second_id), "the press focused the field");
    assert!((fields[1].get().1.top() - second.top()).abs() < SAME, "the press moved no content: {:?} then {second:?}", fields[1].get().1);
    request(&frames.ctx, FocusTarget::Id(fields[0].get().0));
    frames.idle(2, &mut show);
    frames.frame(vec![craft_ui::testing::input::key_press(egui::Key::Tab, egui::Modifiers::NONE)], &mut show);
    frames.frame(vec![craft_ui::testing::focus::key_release(egui::Key::Tab, egui::Modifiers::NONE)], &mut show);
    frames.idle(2, &mut show);
    assert_eq!(frames.ctx.memory(|memory| memory.focused()), Some(second_id), "Tab moved focus to the second field");
    let (_, rect) = fields[1].get();
    assert!(
        view.get().contains_rect(rect) && rect.height() > height - SAME,
        "Tab scrolled the whole field into view: {rect:?} ({height} tall) in {:?}",
        view.get()
    );
}

/// A menu opened from a scroll region, below the region's view: the focus entering the menu reveals its row in
/// the menu's own scroll body only, so the region keeps its offset. Guards that a container on its own layer passes no
/// reveal to the scroll region of its opener.
#[trial]
fn a_menu_opened_in_a_scroll_region_leaves_the_region_where_it_is() {
    let top = Rc::new(Cell::new(f32::NAN));
    let seen = Rc::clone(&top);
    let mut fixture = Fixture::new(move |ui| {
        in_region(ui, Rect::from_min_size(ORIGIN, VIEW), |ui| {
            Slab::new(ContainerProps::new().scroll(Scroll::vertical())).id_salt("panel").show(ui, |ui, _| {
                seen.set(ui.next_widget_position().y);
                Block::new(Vec2::new(LINE.x, VIEW.y - 1.5 * LINE.y)).show(ui);
                let opener = Button::new("Show").show(ui);
                Menu::new("Main").opened_by(&opener, Trigger::Click).show(ui, |ui, _| {
                    for name in ["New", "Save", "Print", "Quit"] {
                        MenuItem::new(name).show(ui);
                    }
                });
                Block::new(Vec2::new(LINE.x, 3.0 * VIEW.y)).show(ui);
            });
        });
    });
    fixture.room(Vec2::splat(3.0 * VIEW.y)).settle();
    let start = top.get();
    let opener = fixture.by_label("Show").expect("the opener");
    fixture.click(&opener).settle();
    assert!(fixture.by_label("Quit").is_some(), "the menu is open");
    assert!((top.get() - start).abs() < SAME, "the region keeps its offset: content top {} from {start}", top.get());
}

/// A field that takes the focus, above a scrolling row whose view its bottom edge touches and whose end it reaches past:
/// the row is no scroll owner of the field, so the reveal that follows the focus moves nothing in it. Guards that a
/// reveal goes to the scroll owner around the component, never to a region beside it (a status bar scrolled to its end
/// when the document above it took the focus in its first pass).
#[trial]
fn a_focus_beside_a_scrolling_row_leaves_the_row_where_it_is() {
    const ROW_VIEW: Vec2 = Vec2::new(120.0, 30.0);
    let field = Cell::new((Id::NULL, Rect::NOTHING));
    let first = Cell::new(Rect::NOTHING);
    let mut show = |ui: &mut Ui| {
        // The field starts inside the row's view and runs past its end.
        let at = Rect::from_min_size(ORIGIN + Vec2::new(ROW_VIEW.x / 2.0, 0.0), Vec2::new(VIEW.x, LINE.y));
        let bottom = in_region(ui, at, |ui| {
            let out = TextField::new("Field", Bind::given(String::new())).id_salt("field").show(ui);
            if !ui.is_sizing_pass() {
                field.set((widget_id(out.response.id, PartKey::new("text")), out.response.rect));
            }
            out.response.rect.bottom()
        });
        let row = Rect::from_min_size(Pos2::new(ORIGIN.x, bottom), ROW_VIEW);
        in_region(ui, row, |ui| {
            craft_ui::layout_components::Row::new().scroll(Scroll::horizontal()).fill_width().id_salt("row").show(ui, |ui, _| {
                for at in 0..10 {
                    let rect = Block::new(Vec2::new(LINE.y * 3.0, LINE.y)).id_salt(("block", at)).show(ui).rect;
                    if at == 0 && !ui.is_sizing_pass() {
                        first.set(rect);
                    }
                }
            });
        });
    };
    let mut frames = Frames::new();
    frames.screen(VIEW + ORIGIN.to_vec2() * 4.0);
    frames.idle(3, &mut show);
    let (id, rect) = field.get();
    let start = first.get();
    assert!(rect.max.x > start.min.x + ROW_VIEW.x, "the field runs past the row's view: {rect:?}");
    request(&frames.ctx, FocusTarget::Id(id));
    frames.idle(3, &mut show);
    assert_eq!(frames.ctx.memory(|memory| memory.focused()), Some(id), "the field took the focus");
    assert!((first.get().min.x - start.min.x).abs() < SAME, "the row kept its offset: {:?} then {start:?}", first.get());
}
