//! A menu taller than its room: it stays inside the room at the width of its rows, scrolls by the strips above and
//! below its rows and by the wheel, and reopens at its first row; one that fits shows no strips. A tall submenu of a
//! low row opens upward.

use craft_ui::buttons::Button;
use craft_ui::contract::{Role, STRIP_BACK, STRIP_FORWARD, Show as _, ShowContainer as _, Trigger};
use craft_ui::menu::bar::{MenuBar, MenuTitle};
use craft_ui::menu::{Menu, MenuItem};
use craft_ui::testing::{Fixture, pointer, trial};
use egui::{Id, Key, Modifiers, Rect, Vec2};
use std::cell::Cell;
use std::rc::Rc;

const ROWS: usize = 40;

fn row(at: usize) -> String {
    format!("Row {at}")
}

/// A menu of `ROWS` rows opened from a button, in a window `height` tall (at least the fixture's 600), with the
/// menu's id.
fn long_menu(height: f32) -> (Fixture, Rc<Cell<Option<Id>>>) {
    let menu = Rc::new(Cell::new(None));
    let seen = Rc::clone(&menu);
    let mut fixture = Fixture::new(move |ui| {
        let opener = Button::new("Open").show(ui);
        let shown = Menu::new("Long").opened_by(&opener, Trigger::Click).show(ui, |ui, _| {
            for at in 0..ROWS {
                MenuItem::new(row(at)).show(ui);
            }
        });
        seen.set(Some(shown.output.response.id));
    });
    fixture.room(Vec2::new(600.0, height)).settle();
    let opener = fixture.by_label("Open").expect("the opener");
    fixture.click(&opener).settle();
    (fixture, menu)
}

fn strips(fixture: &Fixture, menu: Id) -> Option<(Rect, Rect)> {
    fixture.part_rect(menu, STRIP_BACK).zip(fixture.part_rect(menu, STRIP_FORWARD))
}

/// Whether the row `at` is shown whole between the strips.
fn in_view(fixture: &Fixture, menu: Id, at: usize) -> bool {
    let Some((back, forward)) = strips(fixture, menu) else { return false };
    let view = Rect::from_min_max(egui::pos2(back.min.x, back.max.y), egui::pos2(forward.max.x, forward.min.y)).expand(0.5);
    fixture.by_label(&row(at)).is_some_and(|row| view.contains_rect(row.rect))
}

fn menu_rect(fixture: &Fixture) -> Rect {
    fixture.by_role(Role::MENU).first().expect("the menu is open").rect
}

/// Runs frames until `done` holds, at most `frames` of them; whether it held.
fn until(fixture: &mut Fixture, frames: usize, events: impl Fn() -> Vec<egui::Event>, done: impl Fn(&Fixture) -> bool) -> bool {
    for _ in 0..frames {
        if done(fixture) {
            return true;
        }
        fixture.frame(events());
    }
    done(fixture)
}

#[trial]
fn a_menu_taller_than_its_room_scrolls_by_its_strips_and_the_wheel_at_the_width_of_its_rows() {
    let (tall, _) = long_menu(2000.0);
    let width = menu_rect(&tall).width();
    let (mut fixture, menu) = long_menu(600.0);
    let menu = menu.get().expect("the menu's id");
    let (back, forward) = strips(&fixture, menu).expect("an overflowing menu shows its strips");
    let room = fixture.ctx().content_rect();
    let rect = menu_rect(&fixture);
    assert!(room.contains_rect(rect), "the menu {rect:?} stays inside the window {room:?}");
    assert!(back.max.y <= forward.min.y && rect.contains_rect(back) && rect.contains_rect(forward));
    assert!((rect.width() - width).abs() < 0.5, "the strips do not widen the menu: {} for rows {width} wide", rect.width());
    assert!(in_view(&fixture, menu, 0) && !in_view(&fixture, menu, ROWS - 1));

    let rest_on = |at: Rect| move || vec![pointer::moved(at.center())];
    assert!(until(&mut fixture, 400, rest_on(forward), |f| in_view(f, menu, ROWS - 1)), "resting on the lower strip reaches the last row");
    assert!(!in_view(&fixture, menu, 0));
    assert!(until(&mut fixture, 400, rest_on(back), |f| in_view(f, menu, 0)), "resting on the upper strip reaches the first row");

    let middle = Rect::from_min_max(back.center(), forward.center()).center();
    assert!(
        until(&mut fixture, 200, || vec![pointer::moved(middle), pointer::wheel(-60.0)], |f| in_view(f, menu, ROWS - 1)),
        "the wheel reaches the last row"
    );
    assert!((menu_rect(&fixture).width() - width).abs() < 0.5, "scrolling keeps the width");

    // Closed at its end, the menu opens again at its first row.
    fixture.key(Key::Escape, Modifiers::NONE).settle();
    let opener = fixture.by_label("Open").expect("the opener");
    fixture.click(&opener).settle();
    assert!(in_view(&fixture, menu, 0), "a reopened menu starts at its first row");
}

#[trial]
fn a_menu_that_fits_shows_no_strips_and_hugs_its_rows() {
    let (fixture, menu) = long_menu(2000.0);
    let menu = menu.get().expect("the menu's id");
    assert!(strips(&fixture, menu).is_none());
    let rect = menu_rect(&fixture);
    let (first, last) = (fixture.by_label(&row(0)).expect("the first row").rect, fixture.by_label(&row(ROWS - 1)).expect("the last row").rect);
    assert!(((first.top() - rect.top()) - (rect.bottom() - last.bottom())).abs() < 0.5, "as tall as its rows: {rect:?}, rows {first:?} to {last:?}");
}

/// A menu of `above` rows over a submenu row whose submenu has `rows` rows, in the fixture's 800 x 600 window.
fn low_submenu(above: usize, rows: usize) -> Fixture {
    let mut fixture = Fixture::new(move |ui| {
        let opener = Button::new("Open").show(ui);
        Menu::new("Main").opened_by(&opener, Trigger::Click).show(ui, |ui, _| {
            for at in 0..above {
                MenuItem::new(row(at)).show(ui);
            }
            Menu::submenu_row(ui, "More", |ui, _| {
                for at in 0..rows {
                    MenuItem::new(format!("Sub {at}")).show(ui);
                }
            });
        });
    });
    fixture.settle();
    let opener = fixture.by_label("Open").expect("the opener");
    fixture.click(&opener).settle();
    let more = fixture.by_label("More").expect("the submenu row").center();
    fixture.frame(vec![pointer::moved(more)]).settle();
    fixture
}

#[trial]
fn a_tall_submenu_of_a_low_row_opens_upward() {
    let fixture = low_submenu(18, 8);
    let more = fixture.by_label("More").expect("the submenu row").rect;
    let (first, last) = (fixture.by_label("Sub 0").expect("the submenu is open").rect, fixture.by_label("Sub 7").expect("its last row").rect);
    let room = fixture.ctx().content_rect();
    assert!(more.top() + (last.bottom() - first.top()) > room.bottom(), "the submenu does not fit below its row");
    assert!(first.top() < more.top(), "the submenu opens upward: {first:?} over {more:?}");
    assert!((last.bottom() - more.bottom()).abs() < 0.5, "its last row is level with the submenu row: {last:?} and {more:?}");
}

#[trial]
fn a_menu_bar_menu_taller_than_the_window_stays_inside_and_the_wheel_reaches_its_last_row() {
    let mut fixture = Fixture::new(|ui| {
        let titles = vec![MenuTitle::new("&File", |ui| {
            for at in 0..ROWS {
                MenuItem::new(row(at)).show(ui);
            }
        })];
        MenuBar::new("Main menu", titles).show(ui);
    });
    fixture.settle();
    let file = fixture.by_label("File").expect("the File title");
    fixture.click(&file).settle();
    let room = fixture.ctx().content_rect();
    let rect = menu_rect(&fixture);
    assert!(room.contains_rect(rect) && rect.top() >= file.rect.bottom(), "the menu {rect:?} hangs inside the window {room:?}");
    let last = |f: &Fixture| f.by_label(&row(ROWS - 1)).is_some_and(|last| rect.contains_rect(last.rect));
    assert!(!last(&fixture));
    assert!(until(&mut fixture, 200, || vec![pointer::moved(rect.center()), pointer::wheel(-60.0)], last), "the wheel reaches the last row");
}
