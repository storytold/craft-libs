//! `BodyLayout` under direction: the physical layouts, measured `Row` placement and the `Grid` cursor; a content-sized
//! column's held children, a spread row's room and the end region's line.

use craft_behavior::{ColumnWidth, Proposal, Sense};
use craft_ui::contract::{
    BodyCx, Common, Component, Container, ContainerProps, Cx, EndRegion, MeasureCx, Props as _, Role, Show as _, ShowContainer as _, Sizing,
};
use craft_ui::layout::{BodyLayout, RowAlign};
use craft_ui::runtime::env::{Direction, with_env};
use craft_ui::style::StyleCx;
use craft_ui::testing::layout::{Block, body};
use craft_ui::testing::{Fixture, Frames, is_measuring, trial};
use craft_ui_test_kit::contract::{Panel, PlainStyle, Probe, Seen};
use egui::{Rect, Response, Ui, Vec2};
use std::cell::{Cell, RefCell};
use std::rc::Rc;

/// Half a pixel at the scales the box runs.
const SLACK: f32 = 0.5;

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

/// The length of the text a `Wrap` lays out, its longest word and its line height.
const TEXT: f32 = 240.0;
const WORD: f32 = 60.0;
const LINE: f32 = 20.0;

/// How many ideal measures a `Wrap` answered in measuring runs, without a room and in one.
#[derive(Default)]
struct Measures {
    unbounded: Cell<usize>,
    bounded: Cell<usize>,
}

/// A leaf laid out like a paragraph: in a room as wide as the room up to its text, a line for each width of it; with
/// no room on two lines; at its least as wide as its longest word.
#[derive(Component)]
struct Wrap<'a> {
    #[common]
    common: Common,
    seen: &'a RefCell<Seen>,
    measures: &'a Measures,
}

impl<'a> Wrap<'a> {
    fn new(seen: &'a RefCell<Seen>, measures: &'a Measures) -> Self {
        Wrap { common: Common::new(), seen, measures }
    }
}

impl Component for Wrap<'_> {
    const ROLE: Role = Role::GROUP;
    type Style = PlainStyle;
    type Output = Response;

    fn style(_: &StyleCx<'_>, _: WrapStyleKey) -> PlainStyle {
        PlainStyle::default()
    }

    fn measure(&mut self, cx: &MeasureCx<'_, Self>) -> Vec2 {
        if cx.proposal == Proposal::Min {
            return Vec2::new(WORD, LINE);
        }
        if is_measuring(cx.ui.ctx()) {
            let count = if cx.room.is_some() { &self.measures.bounded } else { &self.measures.unbounded };
            count.set(count.get() + 1);
        }
        match cx.room {
            Some(room) => {
                let width = room.clamp(WORD, TEXT);
                Vec2::new(width, LINE * (TEXT / width).ceil())
            }
            None => Vec2::new(TEXT / 2.0, 2.0 * LINE),
        }
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

/// A content-sized column holding a fill-width `Wrap` over a probe `sibling` wide: the column's rect, the wrap's and
/// the probe's, and the wrap's ideal measures in measuring runs without a room and in one.
fn held_in_column(sibling: f32) -> ([Rect; 3], (usize, usize)) {
    let (wrap, probe) = (Rc::new(RefCell::new(Seen::default())), Rc::new(RefCell::new(Seen::default())));
    let (measures, column) = (Rc::new(Measures::default()), Rc::new(Cell::new(Rect::NOTHING)));
    let (shown_wrap, shown_probe) = (Rc::clone(&wrap), Rc::clone(&probe));
    let (counted, shown_column) = (Rc::clone(&measures), Rc::clone(&column));
    let mut fixture = Fixture::new(move |ui| {
        let shown = Panel::new().id_salt("column").show(ui, |ui, _| {
            Wrap::new(&shown_wrap, &counted).sizing(Sizing::FillWidth).id_salt("wrap").show(ui);
            Probe::new(&shown_probe).size(Vec2::new(sibling, LINE)).id_salt("sibling").show(ui);
        });
        shown_column.set(shown.output.rect);
    });
    fixture.room(Vec2::splat(400.0)).settle();
    let rects = [column.get(), wrap.borrow().rect, probe.borrow().rect];
    (rects, (measures.unbounded.get(), measures.bounded.get()))
}

#[trial]
fn a_held_fill_width_child_measures_again_only_when_a_sibling_widens_the_column() {
    let ([column, wrap, sibling], (unbounded, bounded)) = held_in_column(320.0);
    let dump = format!("column {column:?}, wrap {wrap:?}, sibling {sibling:?}");
    assert!((wrap.width() - sibling.width()).abs() < SLACK, "the held child shows across the column: {dump}");
    assert!((column.bottom() - sibling.bottom()).abs() < SLACK, "a blank band under the held child: {dump}");
    let again = "the wider sibling: each measuring run measures the held child once more, at the column";
    assert!(bounded >= 1 && bounded == unbounded, "{again}: {unbounded} and {bounded}");
    let ([column, narrow, sibling], (unbounded, bounded)) = held_in_column(80.0);
    let dump = format!("column {column:?}, wrap {narrow:?}, sibling {sibling:?}");
    assert!(narrow.height() > wrap.height() + SLACK, "the held child's height follows the column's width: {dump} against {wrap:?}");
    assert!((column.bottom() - sibling.bottom()).abs() < SLACK, "a blank band under the held child: {dump}");
    assert!(unbounded >= 1 && bounded == 0, "the narrower sibling: the held child measured again: {unbounded} and {bounded}");
}

#[trial]
fn a_space_between_child_leaves_room_for_the_siblings_after_it() {
    let (first, last) = (Rc::new(RefCell::new(Seen::default())), Rc::new(RefCell::new(Seen::default())));
    let rects = Rc::new(Cell::new([Rect::NOTHING; 2]));
    let (shown_first, shown_last, shown) = (Rc::clone(&first), Rc::clone(&last), Rc::clone(&rects));
    let mut fixture = Fixture::new(move |ui| {
        let measures = Measures::default();
        let row = BodyLayout::row().align(RowAlign::SpaceBetween);
        let spread = Panel::new().layout(row).sizing(Sizing::FillWidth).id_salt("spread").show(ui, |ui, _| {
            Wrap::new(&shown_first, &measures).id_salt("wrap").show(ui);
            // A wrapping row of three blocks: its least is one block's.
            let wrapped = Panel::new().layout(BodyLayout::row().wrap(true)).id_salt("wrapped").show(ui, |ui, _| {
                for _ in 0..3 {
                    Block::new(Vec2::new(50.0, LINE)).show(ui);
                }
            });
            Probe::new(&shown_last).id_salt("last").show(ui);
            wrapped.output.rect
        });
        shown.set([spread.output.rect, spread.inner.unwrap_or(Rect::NOTHING)]);
    });
    fixture.room(Vec2::new(200.0, 400.0)).settle();
    let ([row, wrapped], first, last) = (rects.get(), first.borrow().rect, last.borrow().rect);
    let dump = format!("row {row:?}, first {first:?}, wrapped {wrapped:?}, last {last:?}");
    assert!(last.max.x <= row.max.x + SLACK, "the last child ends past the row: {dump}");
    assert!(first.max.x <= wrapped.min.x + SLACK && wrapped.max.x <= last.min.x + SLACK, "the children overlap: {dump}");
    assert!(first.width() >= WORD - SLACK, "the wrapping row asked for its one-line width as its least: {dump}");
}

/// A container whose body is the caller's closure and whose end region holds a probe that records into `end`.
#[derive(Component)]
#[component(container)]
struct Ended<'a> {
    #[common]
    common: Common,
    #[container]
    props: ContainerProps,
    end: &'a RefCell<Seen>,
}

impl Component for Ended<'_> {
    const ROLE: Role = Role::GROUP;
    type Style = PlainStyle;
    type Output = Response;

    fn style(_: &StyleCx<'_>, _: EndedStyleKey) -> PlainStyle {
        PlainStyle::default()
    }

    fn sense(&self) -> Sense {
        Sense::empty()
    }

    fn finish(self, cx: Cx<'_, Self>) -> Response {
        cx.response.clone()
    }
}

impl Container for Ended<'_> {
    fn end(&mut self, ui: &mut Ui, _: &mut BodyCx<'_>) {
        Probe::new(self.end).id_salt("end").show(ui);
    }
}

#[trial]
fn the_end_region_shares_the_members_line() {
    let (member, end) = (Rc::new(RefCell::new(Seen::default())), Rc::new(RefCell::new(Seen::default())));
    let (shown_member, shown_end) = (Rc::clone(&member), Rc::clone(&end));
    let mut fixture = Fixture::new(move |ui| {
        let (first, second) = (RefCell::new(Seen::default()), RefCell::new(Seen::default()));
        let props = ContainerProps::new().layout(BodyLayout::row()).end(EndRegion::new());
        Ended { common: Common::new(), props, end: &shown_end }.id_salt("ended").show(ui, |ui, _| {
            Probe::new(&shown_member).id_salt("member").show(ui);
            // A member of two lines: the row's line is its first line, above the middle of the row.
            Panel::new().id_salt("lines").show(ui, |ui, _| {
                Probe::new(&first).id_salt("first").show(ui);
                Probe::new(&second).id_salt("second").show(ui);
            });
        });
    });
    fixture.room(Vec2::splat(400.0)).settle();
    let (member, end) = (member.borrow().rect, end.borrow().rect);
    assert!((end.center().y - member.center().y).abs() < SLACK, "the end region's probe {end:?} is off the members' line {member:?}");
}
