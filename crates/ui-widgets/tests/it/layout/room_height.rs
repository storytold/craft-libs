//! A content-sized host with a room height, inside a scrolling body, offers its children that height as the most they
//! take: a child that fills takes its content height while the content fits, and a scrolling child whose content is
//! taller takes the room, in the measuring run and in the real run. A filling panel whose content outgrows the room
//! takes the room and shares it between its list and the bar under it.

use craft_behavior::{DisplayOrder, ItemSource};
use craft_ui::buttons::Button;
use craft_ui::contract::{Bind, Props as _, Scroll, Show as _, ShowCollection as _, ShowContainer as _};
use craft_ui::layout_components::Stack;
use craft_ui::list::{List, ListRow};
use craft_ui::testing::{Fixture, forget_measurements, last_pass_measuring_runs, measured_contributions, trial};
use craft_ui::value_drag::Slider;
use egui::{Rect, Ui, Vec2};
use std::cell::Cell;
use std::rc::Rc;

const TOLERANCE: f32 = 0.5;

#[trial]
fn a_room_height_is_the_most_a_filling_child_takes_inside_a_scroll() {
    const ROOM: Vec2 = Vec2::new(400.0, 300.0);
    // The rects of the short host, its fill child, the tall host and its scrolling child, from the last real run.
    let seen = Rc::new(Cell::new([Rect::NOTHING; 4]));
    let out = Rc::clone(&seen);
    let mut fixture = Fixture::new(move |ui| {
        let room = ui.available_height() / 2.0;
        Stack::new().fill().scroll(Scroll::vertical()).show(ui, |ui, _| {
            let short = Stack::new()
                .fill_width()
                .room_height(room)
                .show(ui, |ui, _| Stack::new().fill().show(ui, |ui, _| Button::new("Inside").show(ui)).output.rect);
            let tall = Stack::new().fill_width().room_height(room).show(ui, |ui, _| {
                Stack::new()
                    .fill()
                    .scroll(Scroll::vertical())
                    .show(ui, |ui, _| {
                        for row in 0..40 {
                            Button::new(format!("Row {row}")).show(ui);
                        }
                    })
                    .output
                    .rect
            });
            if !ui.is_sizing_pass()
                && let (Some(fill), Some(list)) = (short.inner, tall.inner)
            {
                out.set([short.output.rect, fill, tall.output.rect, list]);
            }
        });
    });
    fixture.room(ROOM).settle();
    let [short, fill, tall, list] = seen.get();
    let dump = format!("short {short:?}, fill {fill:?}, tall {tall:?}, list {list:?}");
    assert!(fill.height() > 0.0, "the fill child shows: {dump}");
    assert!((fill.height() - short.height()).abs() <= TOLERANCE, "the host is as tall as its fill child: {dump}");
    assert!(list.height().is_finite() && list.height() < ROOM.y, "the scrolling child stops at the room, not the scroll's unbounded room: {dump}");
    assert!(fill.height() < list.height(), "a fill child whose content fits takes its content height, less than the room: {dump}");
    assert!((tall.height() - list.height()).abs() <= TOLERANCE, "the tall host is as tall as its scrolling child: {dump}");
}

/// How a filling panel holds the bar under its list, and what the list is.
#[derive(Clone, Copy)]
enum Bar {
    /// The bar is the panel's footer region.
    Footer,
    /// The bar is the list's sibling in the panel's body.
    Sibling,
    /// The bar is the panel's footer region, under a list collection.
    CollectionFooter,
}

/// The rows of a list collection.
struct Rows(usize);

impl DisplayOrder<usize> for Rows {
    fn len(&self) -> usize {
        self.0
    }

    fn key(&self, at: usize) -> usize {
        at
    }
}

impl ItemSource<usize> for Rows {}

/// A host with a room height holding a panel that fills: a scrolling list of `rows` and a bar holding a bound, labelled
/// slider. Returns the rects of the host, the list and the bar from the real run.
fn panel(ui: &mut Ui, room: f32, rows: usize, bar: Bar, value: &mut f64) -> Option<[Rect; 3]> {
    let list = |ui: &mut Ui| {
        if let Bar::CollectionFooter = bar {
            return List::new("Rows").show_items(ui, &Rows(rows), &mut |key, _| ListRow::new(format!("Row {key}"))).response.rect;
        }
        Stack::new()
            .fill()
            .scroll(Scroll::vertical())
            .show(ui, |ui, _| {
                for row in 0..rows {
                    Button::new(format!("Row {row}")).show(ui);
                }
            })
            .output
            .rect
    };
    let host = Stack::new().fill_width().room_height(room).show(ui, |ui, _| {
        let mut bar_rect = Rect::NOTHING;
        let shown = match bar {
            Bar::Footer | Bar::CollectionFooter => Stack::new()
                .fill()
                .footer(|ui, _| bar_rect = Slider::new(Bind::bound(&mut *value), 0.0..=100.0).label("Opacity").show(ui).response.rect)
                .show(ui, |ui, _| list(ui)),
            Bar::Sibling => Stack::new().fill().show(ui, |ui, _| {
                let list = list(ui);
                bar_rect = Slider::new(Bind::bound(&mut *value), 0.0..=100.0).label("Opacity").show(ui).response.rect;
                list
            }),
        };
        shown.inner.map(|list| (list, bar_rect))
    });
    let (list, bar) = host.inner.flatten()?;
    (!ui.is_sizing_pass()).then_some([host.output.rect, list, bar])
}

/// A fixture showing [`panel`] at a room height of a third of the window, and the rects of its last real run.
fn panel_fixture(rows: usize, bar: Bar) -> (Fixture, Rc<Cell<[Rect; 3]>>) {
    let seen = Rc::new(Cell::new([Rect::NOTHING; 3]));
    let out = Rc::clone(&seen);
    let mut value = 40.0_f64;
    let mut fixture = Fixture::new(move |ui| {
        let room = ui.available_height() / 3.0;
        Stack::new().fill().scroll(Scroll::vertical()).show(ui, |ui, _| {
            if let Some(rects) = panel(ui, room, rows, bar, &mut value) {
                out.set(rects);
            }
        });
    });
    fixture.room(Vec2::new(400.0, 600.0)).settle();
    (fixture, seen)
}

/// Regression: a filling panel in a room height whose list outgrew the room took the list's whole content height, so the
/// bar under the list sat over its last rows; and a list collection under a panel sized from its content took the whole
/// room in the real run, over the footer. The list takes what the bar leaves, whether the panel fits its room or takes it.
#[trial]
fn a_filling_panel_keeps_the_bar_under_its_list() {
    for (rows, bar) in [(40, Bar::Footer), (40, Bar::Sibling), (40, Bar::CollectionFooter), (3, Bar::CollectionFooter)] {
        let (_, seen) = panel_fixture(rows, bar);
        let [host, list, bar] = seen.get();
        let dump = format!("host {host:?}, list {list:?}, bar {bar:?}");
        assert!(host.height() <= 200.0 + TOLERANCE, "the host stops at its room height: {dump}");
        assert!(bar.top() >= list.bottom() - TOLERANCE, "the bar lies under the list, not over its rows: {dump}");
        assert!(host.contains_rect(bar.shrink(TOLERANCE)), "the host holds the bar: {dump}");
    }
}

/// A filling panel laid out at its room because its content is taller runs no measuring run in a settled frame: its
/// kept measurement holds the layout at the room.
#[trial]
fn a_settled_filling_panel_taller_than_its_room_runs_no_measuring_run() {
    for bar in [Bar::Footer, Bar::Sibling] {
        let (mut fixture, _) = panel_fixture(40, bar);
        fixture.idle(1);
        let runs = last_pass_measuring_runs(fixture.ctx());
        assert!(runs.is_empty(), "a settled pass measured {} bodies: {runs:?}", runs.len());
    }
}

/// A filling panel taller than its room is measured at its content height and then laid out at the room: what the first
/// run handed forward is taken back, so the pass gets each contribution (the slider's least width for its label, its
/// read of the bound value) once, as from the same panel when it fits.
#[trial]
fn a_filling_panel_taller_than_its_room_hands_each_contribution_forward_once() {
    let made = |rows: usize| {
        let (mut fixture, seen) = panel_fixture(rows, Bar::Footer);
        forget_measurements(fixture.ctx());
        fixture.frame(Vec::new());
        (measured_contributions(fixture.ctx()), seen.get()[0].height())
    };
    let ((fits, short), (over, tall)) = (made(2), made(40));
    assert!(short < tall && tall <= 200.0 + TOLERANCE, "the short panel fits its room and the long one fills it: {short}, {tall}");
    assert!(fits > 0, "the panel's measuring run hands contributions forward");
    assert_eq!(over, fits, "the panel taller than its room hands forward as many contributions as one that fits");
}
