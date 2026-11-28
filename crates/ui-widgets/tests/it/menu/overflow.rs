//! A menu taller than its room: it stays inside the room at the width of its rows, scrolls by the strips above and
//! below its rows and by the wheel, and reopens at its first row; one that fits shows no strips. A tall submenu of a
//! low row opens upward. A context menu that fits on neither side of the pointer, or of the control a key asked it
//! for, but fits the window covers its anchor instead of scrolling; a tooltip at the pointer never covers the pointer.

use craft_host::target::Os;
use craft_ui::buttons::Button;
use craft_ui::contract::tooltip::DELAY;
use craft_ui::contract::{Bind, Entry, Props as _, Role, STRIP_BACK, STRIP_FORWARD, Show as _, ShowContainer as _, TooltipAt, TooltipSpec, Trigger};
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

/// The clock a frame resting on a strip moves on beyond its own: a strip scrolls by the pass's elapsed time, taken up
/// to a tenth of a second, so each frame takes the longest step a strip takes.
const STRIP_STEP: f64 = 0.1;

/// Runs frames until `done` holds, at most `frames` of them; whether it held.
fn until(fixture: &mut Fixture, frames: usize, events: impl Fn() -> Vec<egui::Event>, done: impl Fn(&Fixture) -> bool) -> bool {
    until_stepped(fixture, frames, 0.0, events, done)
}

/// [`until`] with the clock moved on by `step` before each frame, beyond the frame's own.
fn until_stepped(fixture: &mut Fixture, frames: usize, step: f64, events: impl Fn() -> Vec<egui::Event>, done: impl Fn(&Fixture) -> bool) -> bool {
    for frame in 0..frames {
        if done(fixture) {
            eprintln!("DIAG overflow until: held after {frame} frames");
            return true;
        }
        fixture.advance(step).frame(events());
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
    assert!(
        until_stepped(&mut fixture, 400, STRIP_STEP, rest_on(forward), |f| in_view(f, menu, ROWS - 1)),
        "resting on the lower strip reaches the last row"
    );
    assert!(!in_view(&fixture, menu, 0));
    assert!(until_stepped(&mut fixture, 400, STRIP_STEP, rest_on(back), |f| in_view(f, menu, 0)), "resting on the upper strip reaches the first row");

    let middle = Rect::from_min_max(back.center(), forward.center()).center();
    assert!(
        until(&mut fixture, 200, || vec![pointer::moved(middle), pointer::wheel(-240.0)], |f| in_view(f, menu, ROWS - 1)),
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

/// A context menu of `rows` rows on a target `top` down the fixture's 800 x 600 window, right-clicked at the target's
/// centre: the fixture, the menu's id and the pointer.
fn context_menu(top: f32, rows: usize) -> (Fixture, Id, egui::Pos2) {
    let menu = Rc::new(Cell::new(None));
    let seen = Rc::clone(&menu);
    let mut fixture = Fixture::new(move |ui| {
        ui.add_space(top);
        let target = Button::new("Target").show(ui);
        let shown = Menu::context("Target menu", &target).show(ui, |ui, _| {
            for at in 0..rows {
                MenuItem::new(row(at)).show(ui);
            }
        });
        seen.set(Some(shown.output.response.id));
    });
    fixture.settle();
    let at = fixture.by_label("Target").expect("the target").center();
    fixture.frame(vec![pointer::moved(at)]);
    fixture.frame(vec![pointer::secondary(at, true)]);
    fixture.frame(vec![pointer::secondary(at, false)]);
    fixture.settle();
    (fixture, menu.get().expect("the menu's id"), at)
}

#[trial]
fn a_context_menu_too_tall_for_either_side_of_the_pointer_covers_it_and_shows_every_row_without_strips() {
    let rows = 18;
    let (fixture, menu, at) = context_menu(290.0, rows);
    let room = fixture.ctx().content_rect();
    let rect = menu_rect(&fixture);
    assert!(strips(&fixture, menu).is_none(), "the menu {rect:?} fits the window {room:?}: no strips");
    let (first, last) = (fixture.by_label(&row(0)).expect("the first row").rect, fixture.by_label(&row(rows - 1)).expect("the last row").rect);
    let tall = last.bottom() - first.top();
    assert!(tall > at.y - room.top() && tall > room.bottom() - at.y, "the rows ({tall}) fit on neither side of the pointer at {at:?}");
    assert!(room.contains_rect(rect), "the menu {rect:?} stays inside the window {room:?}");
    assert!(rect.contains_rect(first) && rect.contains_rect(last), "every row shows: {first:?} to {last:?} in {rect:?}");
}

/// The same menu asked for with Shift+F10 on the focused target: it hangs from the target's rect, which it covers.
#[trial]
fn a_context_menu_a_key_asked_for_covers_its_target_when_it_fits_neither_side() {
    let rows = 18;
    let (mut fixture, menu, _) = context_menu(290.0, rows);
    fixture.key(Key::Escape, Modifiers::NONE).settle();
    assert!(fixture.by_role(Role::MENU).is_empty(), "Escape closes the menu the pointer opened");
    let target = fixture.by_label("Target").expect("the target").rect;
    fixture.key(Key::Tab, Modifiers::NONE);
    fixture.key(Key::F10, Modifiers::SHIFT).settle();
    let room = fixture.ctx().content_rect();
    let rect = menu_rect(&fixture);
    assert!(strips(&fixture, menu).is_none(), "the menu {rect:?} fits the window {room:?}: no strips");
    let (first, last) = (fixture.by_label(&row(0)).expect("the first row").rect, fixture.by_label(&row(rows - 1)).expect("the last row").rect);
    assert!(rect.contains_rect(first) && rect.contains_rect(last), "every row shows: {first:?} to {last:?} in {rect:?}");
    assert!(rect.intersects(target), "the menu {rect:?} covers its target {target:?}");
}

/// A tooltip at the pointer too tall for either side of it stays off the pointer: it opens on its preferred side,
/// below, where the room is larger, and does not slide up over the pointer as a menu does.
#[trial]
fn a_tooltip_at_the_pointer_too_tall_for_either_side_does_not_cover_the_pointer() {
    let text = (0..40).map(|line| format!("Line {line}")).collect::<Vec<_>>().join("\n");
    let mut fixture = Fixture::new(move |ui| {
        ui.add_space(200.0);
        Button::new("Target").tooltip(TooltipSpec::new(text.clone()).at(TooltipAt::Pointer)).show(ui);
    });
    fixture.settle();
    let at = fixture.by_label("Target").expect("the target").center();
    fixture.frame(vec![pointer::moved(at)]);
    fixture.advance(DELAY);
    fixture.settle();
    let tips = fixture.by_role(Role::TOOLTIP);
    let tip = tips.first().expect("the tooltip shows").rect;
    let room = fixture.ctx().content_rect();
    let lines = fixture.texts().iter().filter(|run| run.text.starts_with("Line ")).fold(Rect::NOTHING, |all, run| all.union(run.rect));
    assert!(lines.height() > at.y - room.top() && lines.height() > room.bottom() - at.y, "the text ({lines:?}) fits on neither side of {at:?}");
    assert!(tip.top() >= at.y, "the tooltip {tip:?} opens below the pointer at {at:?} and does not cover it");
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
    // The in-window menu bar and its Alt, F10 and letter keys are PC behaviour: a Mac app's menu bar is native.
    fixture.os(Os::Linux);
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
    fixture.os(Os::Linux);
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
    fixture.os(Os::Linux);
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
    fixture.os(Os::Linux);
    fixture.settle();
    let view = fixture.by_label("View").expect("the View title");
    fixture.click(&view).settle();
    let field = fixture.by_role(Role::COMBO_BOX).first().copied().expect("the select");
    fixture.click(&field).settle();
    let list = fixture.by_role(Role::LIST_BOX).first().expect("the list is open").rect;
    let window = fixture.ctx().content_rect();
    assert!(list.height() <= window.height() * 0.5, "the list {list:?} keeps the dropdown's share of the window {window:?}");
}

#[trial]
fn a_long_menu_and_its_submenu_keep_their_rects_from_frame_to_frame() {
    let mut fixture = low_submenu(18, 8);
    let rects = |fixture: &Fixture| fixture.by_role(Role::MENU).iter().map(|menu| menu.rect).collect::<Vec<_>>();
    let settled = rects(&fixture);
    assert!(settled.len() >= 2, "the menu and its submenu are open: {settled:?}");
    for frame in 0..3 {
        fixture.frame(vec![]);
        assert_eq!(rects(&fixture), settled, "frame {frame}: the menu and its submenu keep their rects");
    }
}
