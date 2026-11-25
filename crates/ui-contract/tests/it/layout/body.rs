//! `BodyLayout` under direction: the physical layouts, measured `Row` placement and the `Grid` cursor; a content-sized
//! column's held children, a spread row's room and the end region's line.

use craft_behavior::{ColumnWidth, Insets, Metrics, Proposal, Sense};
use craft_ui::contract::{
    BodyCx, Common, Component, Container, ContainerProps, Cx, EndRegion, MeasureCx, MemoProps, MemoView, Props as _, Role, Show as _,
    ShowContainer as _, Sizing,
};
use craft_ui::layout::{BodyLayout, RowAlign};
use craft_ui::runtime::env::{Direction, with_env};
use craft_ui::style::{ComponentStyle, StyleCx};
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

#[trial]
fn a_fill_before_a_wrapping_sibling_keeps_its_geometry_after_replay_and_resize() {
    let rects = Rc::new(Cell::new([Rect::NOTHING; 2]));
    let shown = Rc::clone(&rects);
    let mut fixture = Fixture::new(move |ui| {
        let (fill, wrap) = (RefCell::new(Seen::default()), RefCell::new(Seen::default()));
        let measures = Measures::default();
        Panel::new().layout(BodyLayout::row()).sizing(Sizing::FillWidth).id_salt("row").show(ui, |ui, _| {
            Probe::new(&fill).size(Vec2::new(20.0, LINE)).sizing(Sizing::FillWidth).id_salt("fill").show(ui);
            Wrap::new(&wrap, &measures).id_salt("wrap").show(ui);
        });
        shown.set([fill.borrow().rect, wrap.borrow().rect]);
    });
    for width in [400.0, 240.0, 400.0] {
        fixture.room(Vec2::new(width, 400.0)).settle();
        let [fill, wrap] = rects.get();
        assert!(fill.width() > 0.0 && wrap.width() >= WORD - SLACK, "fill and wrap are visible: {fill:?}, {wrap:?}");
        assert!(fill.max.x <= wrap.min.x + SLACK, "wrap follows fill: {fill:?}, {wrap:?}");
        fixture.idle(3);
        let replayed = rects.get();
        assert_eq!([fill, wrap], replayed, "replay preserves the measured geometry");
    }
}

#[derive(Component)]
struct ConditionalDemand {
    #[common]
    common: Common,
}

impl Component for ConditionalDemand {
    const ROLE: Role = Role::GROUP;
    type Style = PlainStyle;
    type Output = Response;

    fn style(_: &StyleCx<'_>, _: ConditionalDemandStyleKey) -> PlainStyle {
        PlainStyle::default()
    }

    fn measure(&mut self, cx: &MeasureCx<'_, Self>) -> Vec2 {
        Vec2::new(if cx.proposal == Proposal::Min { 10.0 } else { 100.0 }, LINE)
    }

    fn sense(&self) -> Sense {
        Sense::empty()
    }

    fn finish(self, cx: Cx<'_, Self>) -> Response {
        cx.response.clone()
    }
}

#[trial]
fn a_nested_body_can_have_different_children_in_its_least_probe() {
    let saw = Rc::new(RefCell::new(Vec::new()));
    let found = Rc::clone(&saw);
    let mut fixture = Fixture::new(move |ui| {
        Panel::new().layout(BodyLayout::row()).sizing(Sizing::FillWidth).id_salt("outer").show(ui, |ui, _| {
            Panel::new().layout(BodyLayout::row()).id_salt("nested").show(ui, |ui, _| {
                ConditionalDemand { common: Common::new() }.id_salt("a").show(ui);
                let width = ui.available_rect_before_wrap().width();
                let unbounded = !width.is_finite();
                if is_measuring(ui.ctx()) {
                    found.borrow_mut().push(width);
                }
                if unbounded {
                    Block::new(Vec2::new(200.0, LINE)).show(ui);
                }
            });
            Block::new(Vec2::new(180.0, LINE)).show(ui);
        });
    });
    fixture.room(Vec2::new(200.0, 300.0)).settle();
    assert!(!craft_ui::testing::shrink_least_runs(fixture.ctx()).is_empty(), "the outer row ran its least probe");
    let saw = saw.borrow();
    assert!(saw.iter().any(|width| width.is_finite()) && saw.iter().any(|width| !width.is_finite()), "observed {saw:?}");
}

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

/// A picture from `least` up to `MOST_PICTURE` wide: square (`picture_size`), or a band of its own height
/// (`picture_width`), with `PICTURE_PAD` of padding at each side.
#[derive(Component)]
struct Picture<'a> {
    #[common]
    common: Common,
    least: f32,
    square: bool,
    seen: &'a RefCell<Seen>,
    measures: &'a Measures,
}

/// The widest a `Picture` takes.
const MOST_PICTURE: f32 = 1000.0;

/// A `Picture`'s padding at each side.
const PICTURE_PAD: f32 = 20.0;

/// A style padded `PICTURE_PAD` at each side.
#[derive(Clone, Debug, Default, PartialEq, ComponentStyle)]
struct PaddedStyle {
    #[style(metrics)]
    metrics: Metrics,
}

impl Component for Picture<'_> {
    const ROLE: Role = Role::GROUP;
    type Style = PaddedStyle;
    type Output = Response;

    fn style(_: &StyleCx<'_>, _: PictureStyleKey) -> PaddedStyle {
        PaddedStyle { metrics: Metrics::new().padding(Insets::new(PICTURE_PAD, PICTURE_PAD, 0.0, 0.0)) }
    }

    fn measure(&mut self, cx: &MeasureCx<'_, Self>) -> Vec2 {
        if cx.proposal == Proposal::Ideal && is_measuring(cx.ui.ctx()) {
            let count = if cx.room.is_some() { &self.measures.bounded } else { &self.measures.unbounded };
            count.set(count.get() + 1);
        }
        match self.square {
            true => cx.picture_size(self.least, MOST_PICTURE, |width| width),
            false => Vec2::new(cx.picture_width(self.least, MOST_PICTURE), LINE),
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

/// The picture a column trial shows: its least width and whether it is square.
#[derive(Clone, Copy)]
struct Shown {
    least: f32,
    square: bool,
}

impl Shown {
    const fn square(least: f32) -> Self {
        Shown { least, square: true }
    }
}

/// What a content-sized column shows with a picture (none when `None`), a `Wrap` and a probe under them `sibling`
/// wide: the column's rect, the picture's and the probe's, the picture's ideal measures in measuring runs without a
/// room and in one, and the wrap's.
struct PictureColumn {
    rects: [Rect; 3],
    picture: (usize, usize),
    wrap: (usize, usize),
}

fn picture_in_column(picture: Option<Shown>, sibling: f32) -> PictureColumn {
    let (seen, under, wrap) =
        (Rc::new(RefCell::new(Seen::default())), Rc::new(RefCell::new(Seen::default())), Rc::new(RefCell::new(Seen::default())));
    let (pictures, wraps, column) = (Rc::new(Measures::default()), Rc::new(Measures::default()), Rc::new(Cell::new(Rect::NOTHING)));
    let (shown_picture, shown_under, shown_wrap) = (Rc::clone(&seen), Rc::clone(&under), Rc::clone(&wrap));
    let (counted_pictures, counted_wraps, shown_column) = (Rc::clone(&pictures), Rc::clone(&wraps), Rc::clone(&column));
    let mut fixture = Fixture::new(move |ui| {
        let shown = Panel::new().id_salt("column").show(ui, |ui, _| {
            if let Some(Shown { least, square }) = picture {
                let picture = Picture { common: Common::new(), least, square, seen: &shown_picture, measures: &counted_pictures };
                picture.id_salt("picture").show(ui);
            }
            Wrap::new(&shown_wrap, &counted_wraps).id_salt("wrap").show(ui);
            Probe::new(&shown_under).size(Vec2::new(sibling, LINE)).id_salt("under").show(ui);
        });
        shown_column.set(shown.output.rect);
    });
    fixture.room(Vec2::splat(800.0)).settle();
    let rects = [column.get(), seen.borrow().rect, under.borrow().rect];
    PictureColumn { rects, picture: (pictures.unbounded.get(), pictures.bounded.get()), wrap: (wraps.unbounded.get(), wraps.bounded.get()) }
}

// A square picture in a content-sized column takes no width of its own there; the column settles on its siblings'
// width and the picture shows across it, as tall as wide, with what follows it laid out below the shown square.
#[trial]
fn a_square_picture_in_a_content_column_pushes_what_follows_below_its_shown_size() {
    let PictureColumn { rects: [column, picture, under], .. } = picture_in_column(Some(Shown::square(60.0)), 300.0);
    let dump = format!("column {column:?}, picture {picture:?}, under {under:?}");
    assert!(picture.width() > 60.0 + 2.0 * PICTURE_PAD + SLACK, "the picture shows wider than its least, across the column: {dump}");
    assert!((picture.width() - 2.0 * PICTURE_PAD - picture.height()).abs() < SLACK, "a square picture's content is as tall as it shows wide: {dump}");
    assert!(under.top() >= picture.bottom() - SLACK, "what follows lies below the shown picture: {dump}");
    assert!(column.bottom() >= under.bottom() - SLACK, "the column holds everything it shows: {dump}");
}

// Count trial: a column whose square picture took its least sweeps again once at its settled width; a column without
// a picture, whose square picture is its widest child (padding included), or whose picture is a band of its own
// height, sweeps once.
#[trial]
fn a_content_column_sweeps_again_only_for_a_square_picture_narrower_than_it() {
    let none = picture_in_column(None, 300.0);
    let narrow = picture_in_column(Some(Shown::square(60.0)), 300.0);
    assert!(none.wrap.0 >= 1 && none.wrap.1 == 0, "no picture: the column sweeps its children: {:?}", none.wrap);
    let (unbounded, bounded) = narrow.picture;
    assert!(
        bounded >= 1 && bounded == unbounded,
        "a narrower square picture measures once more per measuring run, at the column: {unbounded} and {bounded}"
    );
    let once = [
        ("a square picture wider than the column", Shown::square(400.0)),
        ("a square picture as wide as the column with its padding", Shown::square(300.0 - 2.0 * PICTURE_PAD)),
        ("a band picture", Shown { least: 60.0, square: false }),
    ];
    for (what, shown) in once {
        let column = picture_in_column(Some(shown), 300.0);
        assert_eq!(column.wrap, none.wrap, "{what} adds no sweep");
        assert!(column.picture.0 >= 1 && column.picture.1 == 0, "{what} keeps its first measure: {:?}", column.picture);
    }
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

/// The children of a wrapping `Row` stand on the line they wrap onto: one-line children sit at the middle of their
/// line, a child of more than one line at its top, and each line is as tall as its tallest child, also in the frames
/// that reuse the row's measurement. The row is sized from its content, so its real run is narrower than its
/// measuring run's room.
#[trial]
fn a_wrapping_row_sets_its_children_on_the_middle_of_their_line() {
    const SIZES: [Vec2; 5] = [ITEM, Vec2::new(40.0, 30.0), ITEM, Vec2::new(40.0, 40.0), ITEM];
    let seen: Rc<[RefCell<Seen>; 7]> = Rc::new(Default::default());
    let shown = Rc::clone(&seen);
    let mut fixture = Fixture::new(move |ui| {
        Panel::new().layout(BodyLayout::row().wrap(true)).id_salt("wrapping").show(ui, |ui, _| {
            for (i, size) in SIZES.iter().take(3).enumerate() {
                Probe::new(&shown[i]).size(*size).id_salt(i).show(ui);
            }
            // A column of two lines: it starts the second line.
            Panel::new().layout(BodyLayout::Stack).id_salt("column").show(ui, |ui, _| {
                Probe::new(&shown[5]).size(ITEM).id_salt("upper").show(ui);
                Probe::new(&shown[6]).size(ITEM).id_salt("lower").show(ui);
            });
            for (i, size) in SIZES.iter().enumerate().skip(3) {
                Probe::new(&shown[i]).size(*size).id_salt(i).show(ui);
            }
        });
    });
    fixture.room(Vec2::new(150.0, 400.0)).settle();
    let rects = || seen.each_ref().map(|seen| seen.borrow().rect);
    let settled = rects();
    let [first, tall, third, taller, last, upper, lower] = settled;
    let dump = format!("{settled:?}");
    assert!(
        (first.center().y - tall.center().y).abs() < SLACK && (third.center().y - tall.center().y).abs() < SLACK,
        "the first line's one-line children share its middle: {dump}"
    );
    assert!(upper.min.x < tall.min.x && upper.top() >= tall.bottom() - SLACK, "the column wraps onto the second line: {dump}");
    assert!((upper.top() - taller.top()).abs() < SLACK && lower.bottom() < taller.bottom(), "the column sits at the top of its line: {dump}");
    assert!((last.center().y - taller.center().y).abs() < SLACK, "the second line's one-line children share its middle: {dump}");
    assert!(upper.top() - tall.bottom() < ITEM.y, "the first line is as tall as its tallest child: {dump}");
    fixture.idle(3);
    assert_eq!(rects(), settled, "the frames that reuse the measurement set the children on the same lines");
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

/// A memoized view of one short block.
#[derive(MemoProps)]
struct Short {}

fn short(_: &mut Short, ui: &mut Ui) -> Rect {
    Block::new(Vec2::new(40.0, 10.0)).show(ui).rect
}

/// A memoized view in a row sits on the row's line, as the one-line child beside it does, also while it replays. Its
/// content starts a layout room of its own, which left it at the row's top (a menu bar beside an app icon,
/// storytold/photocraft#16113).
#[trial]
fn a_memoized_view_sits_on_the_rows_line() {
    let seen = Rc::new(Cell::new((Rect::NOTHING, Rect::NOTHING)));
    let shown = Rc::clone(&seen);
    let mut fixture = Fixture::new(move |ui| {
        Panel::new().layout(BodyLayout::row()).id_salt("row").show(ui, |ui, _| {
            let tall = Block::new(Vec2::new(20.0, 30.0)).show(ui).rect;
            let view = MemoView::new("short", Short {}, short).show(ui);
            if !is_measuring(ui.ctx()) {
                shown.set((tall, view.unwrap_or(Rect::NOTHING)));
            }
        });
    });
    fixture.room(Vec2::splat(400.0)).settle();
    for frame in 0..3 {
        let (tall, view) = seen.get();
        assert!(view.height() > 0.0, "frame {frame}: the view showed: {view:?}");
        assert!((view.center().y - tall.center().y).abs() < SLACK, "frame {frame}: the view {view:?} is off the line of {tall:?}");
        fixture.settle();
    }
}

/// A Row in right-to-left holding a vertically scrolling container (content-sized, then filling its height) beside a
/// probe: the container, the probe inside it and the probe beside it are finite, and the container lies in the row.
#[trial]
fn a_scrolling_container_in_a_right_to_left_row_measures_and_shows_finite() {
    use craft_ui::contract::Scroll;
    use craft_ui_test_kit::body::Slab;
    for (sizing, wide) in [(Sizing::Content, 40.0), (Sizing::FillHeight, 40.0), (Sizing::Content, 2000.0), (Sizing::FillHeight, 2000.0)] {
        let (inside, beside) = (RefCell::new(Seen::default()), RefCell::new(Seen::default()));
        let (region, rects) = lay_out(Some(Direction::Rtl), |ui| {
            let row = Panel::new().layout(BodyLayout::row()).sizing(Sizing::FillWidth).id_salt("row").show(ui, |ui, _| {
                let props = ContainerProps::new().scroll(Scroll::vertical()).sizing(sizing);
                let panel = Slab::new(props).id_salt("settings").show(ui, |ui, _| {
                    Probe::new(&inside).size(Vec2::new(120.0, 400.0)).id_salt("inside").show(ui);
                });
                Probe::new(&beside).size(Vec2::new(wide, 10.0)).sizing(Sizing::FillWidth).id_salt("beside").show(ui);
                panel.output.rect
            });
            vec![row.output.rect, row.inner.unwrap_or(Rect::NOTHING)]
        });
        let (inside, beside) = (inside.into_inner().rect, beside.into_inner().rect);
        let finite = |rect: Rect| rect.min.is_finite() && rect.max.is_finite();
        let (row, panel) = (rects[0], rects[1]);
        let dump = format!("{sizing:?}: region {region:?}, row {row:?}, panel {panel:?}, inside {inside:?}, beside {beside:?}");
        assert!(finite(row) && finite(panel) && finite(inside) && finite(beside), "every rect is finite: {dump}");
        assert!(panel.width() > 100.0 && panel.max.x <= region.max.x + SLACK, "the container shows in the row: {dump}");
        assert!(panel.min.x > beside.max.x - SLACK, "under RTL the container is right of the probe after it: {dump}");
    }
}
