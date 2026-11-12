//! A menu taller than its room: it stays inside the room at the width of its rows, scrolls by the strips above and
//! below its rows and by the wheel, and reopens at its first row; one that fits shows no strips. A tall submenu of a
//! low row opens upward.

use craft_ui::buttons::Button;
use craft_ui::contract::{Bind, Entry, Role, STRIP_BACK, STRIP_FORWARD, Show as _, ShowContainer as _, Trigger};
use craft_ui::menu::bar::{MenuBar, MenuTitle};
use craft_ui::menu::{Menu, MenuItem};
use craft_ui::pickers::Select;
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

#[trial]
fn no_menu_or_submenu_of_a_menu_bar_covers_the_bar() {
    let mut fixture = Fixture::new(|ui| {
        let titles = vec![MenuTitle::new("&File", |ui| {
            for at in 0..8 {
                MenuItem::new(row(at)).show(ui);
            }
            Menu::submenu_row(ui, "More", |ui, _| {
                for at in 0..ROWS {
                    MenuItem::new(format!("Sub {at}")).show(ui);
                }
            });
        })];
        MenuBar::new("Main menu", titles).show(ui);
    });
    fixture.settle();
    let file = fixture.by_label("File").expect("the File title");
    fixture.click(&file).settle();
    let more = fixture.by_label("More").expect("the submenu row").center();
    fixture.frame(vec![pointer::moved(more)]).settle();
    assert!(fixture.by_label("Sub 0").is_some(), "the submenu is open");
    let room = fixture.ctx().content_rect();
    let menus = fixture.by_role(Role::MENU);
    assert!(menus.len() >= 2, "the menu and its submenu: {menus:?}");
    for menu in menus {
        assert!(menu.rect.top() >= file.rect.bottom() - 0.5, "{:?} covers the bar ending at {}", menu.rect, file.rect.bottom());
        assert!(room.contains_rect(menu.rect), "{:?} stays inside the window {room:?}", menu.rect);
    }
}

/// A bar's File menu of `ROWS` rows with a submenu row among the last ones, opened and wheeled to its end.
fn scrolled_bar_menu() -> (Fixture, Rect) {
    let mut fixture = Fixture::new(|ui| {
        let titles = vec![MenuTitle::new("&File", |ui| {
            for at in 0..ROWS {
                MenuItem::new(row(at)).show(ui);
                if at == ROWS - 3 {
                    Menu::submenu_row(ui, "Recent", |ui, _| {
                        MenuItem::new("Report.txt").show(ui);
                    });
                }
            }
        })];
        MenuBar::new("Main menu", titles).show(ui);
    });
    fixture.settle();
    let file = fixture.by_label("File").expect("the File title").rect;
    fixture.click_at(file.center()).settle();
    let menu = menu_rect(&fixture);
    let at_end = |f: &Fixture| f.by_label(&row(ROWS - 1)).is_some_and(|last| menu.contains_rect(last.rect));
    assert!(until(&mut fixture, 200, || vec![pointer::moved(menu.center()), pointer::wheel(-60.0)], at_end), "the wheel reaches the end");
    (fixture, file)
}

#[trial]
fn a_menu_bar_menu_closed_at_its_end_reopens_at_its_first_row_with_one_level_open() {
    let (mut fixture, file) = scrolled_bar_menu();
    fixture.key(Key::Escape, Modifiers::NONE).settle();
    assert!(fixture.by_role(Role::MENU).is_empty(), "Escape closed the menu");
    fixture.click_at(file.center());
    for frame in 0..6 {
        fixture.frame(vec![]);
        let menus = fixture.by_role(Role::MENU);
        assert_eq!(menus.len(), 1, "frame {frame}: the menu is open and no submenu opened under the title");
        let first = fixture.by_label(&row(0)).expect("the first row");
        assert!(menus[0].rect.contains_rect(first.rect), "frame {frame}: the menu starts at its first row");
    }
}

#[trial]
fn a_dropdown_in_a_menu_bar_menu_keeps_its_cap() {
    let mut fixture = Fixture::new(|ui| {
        let titles = vec![MenuTitle::new("&View", |ui| {
            let entries: Vec<Entry<u8>> = (0..60u8).map(|at| Entry::new(at, format!("Zoom {at}"))).collect();
            let mut value = 0u8;
            Select::new("Zoom", Bind::bound(&mut value), &entries).show(ui);
        })];
        MenuBar::new("Main menu", titles).show(ui);
    });
    fixture.settle();
    let view = fixture.by_label("View").expect("the View title");
    fixture.click(&view).settle();
    let field = fixture.by_role(Role::COMBO_BOX).first().copied().expect("the select");
    fixture.click(&field).settle();
    let list = fixture.by_role(Role::LIST_BOX).first().expect("the list is open").rect;
    let window = fixture.ctx().content_rect();
    assert!(list.height() <= window.height() * 0.5, "the list {list:?} keeps the dropdown's share of the window {window:?}");
}
