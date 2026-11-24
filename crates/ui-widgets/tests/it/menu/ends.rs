//! Up and Down at a menu's ends, and Home and End (photocraft#15227, photocraft#15219): a context menu, a button's
//! menu and their submenus stop at their first and last enabled rows, Home and End go to them, and a menu bar's menus
//! still continue at the other end.

use craft_host::target::Os;
use craft_ui::buttons::Button;
use craft_ui::contract::{Props as _, Show as _, ShowContainer as _, Trigger};
use craft_ui::menu::bar::{MenuBar, MenuTitle};
use craft_ui::menu::{Menu, MenuItem};
use craft_ui::testing::{Fixture, pointer, trial};
use egui::{Id, Key, Modifiers};
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

/// The ids of the rows the menus showed, by label.
type Rows = Rc<RefCell<HashMap<&'static str, Id>>>;

/// Shows a row labelled `label` and notes its id.
fn row(ui: &mut egui::Ui, rows: &Rows, label: &'static str, enabled: bool) {
    let shown = MenuItem::new(label).enabled(enabled).show(ui);
    rows.borrow_mut().insert(label, shown.response.id);
}

/// The label of the row that holds the focus.
fn focused(fixture: &Fixture, rows: &Rows) -> Option<&'static str> {
    let (id, _) = fixture.focused_stop()?;
    rows.borrow().iter().find(|(_, row)| **row == id).map(|(label, _)| *label)
}

/// Presses `key` and returns the row that holds the focus after it.
fn press(fixture: &mut Fixture, rows: &Rows, key: Key) -> Option<&'static str> {
    fixture.key(key, Modifiers::NONE);
    fixture.settle();
    focused(fixture, rows)
}

/// A context menu over a button: Alpha, a greyed Bravo, Charlie, the submenu More (One, Two, Three) and Echo. With
/// `by_press` it is the button's menu, opened by a click on it.
fn menu(by_press: bool) -> (Fixture, Rows) {
    let rows: Rows = Rc::default();
    let seen = rows.clone();
    let mut fixture = Fixture::new(move |ui| {
        let target = Button::new("Target").show(ui);
        let menu = if by_press { Menu::new("Target menu").opened_by(&target, Trigger::Press) } else { Menu::context("Target menu", &target) };
        menu.show(ui, |ui, _| {
            row(ui, &seen, "Alpha", true);
            row(ui, &seen, "Bravo", false);
            row(ui, &seen, "Charlie", true);
            let more = Menu::submenu_row(ui, "More", |ui, _| {
                row(ui, &seen, "One", true);
                row(ui, &seen, "Two", true);
                row(ui, &seen, "Three", true);
            });
            seen.borrow_mut().insert("More", more.response.id);
            row(ui, &seen, "Echo", true);
        });
    });
    fixture.settle();
    let target = fixture.by_label("Target").expect("the target");
    if by_press {
        fixture.click(&target);
    } else {
        let at = target.center();
        fixture.frame(vec![pointer::moved(at)]);
        fixture.frame(vec![pointer::secondary(at, true)]);
        fixture.frame(vec![pointer::secondary(at, false)]);
    }
    fixture.settle();
    assert!(fixture.by_label("Alpha").is_some(), "the menu is open");
    (fixture, rows)
}

#[trial]
fn up_and_down_stop_at_the_ends_of_a_context_menu_and_its_submenu() {
    for by_press in [false, true] {
        let (mut fixture, rows) = menu(by_press);
        let mut walk = Vec::new();
        for key in [Key::ArrowDown, Key::ArrowUp, Key::ArrowDown, Key::ArrowDown, Key::ArrowDown, Key::ArrowDown, Key::ArrowDown] {
            walk.push(press(&mut fixture, &rows, key));
        }
        assert_eq!(
            walk,
            [Some("Alpha"), Some("Alpha"), Some("Charlie"), Some("More"), Some("Echo"), Some("Echo"), Some("Echo")],
            "Up on the first row and Down on the last stay there, and the greyed row is skipped (press {by_press})"
        );
        assert_eq!(press(&mut fixture, &rows, Key::ArrowUp), Some("More"));
        assert_eq!(press(&mut fixture, &rows, Key::ArrowRight), Some("One"), "Right enters the submenu (press {by_press})");
        let mut walk = Vec::new();
        for key in [Key::ArrowUp, Key::ArrowDown, Key::ArrowDown, Key::ArrowDown] {
            walk.push(press(&mut fixture, &rows, key));
        }
        assert_eq!(walk, [Some("One"), Some("Two"), Some("Three"), Some("Three")], "the submenu stops at its ends too (press {by_press})");
    }
}

#[trial]
fn home_and_end_go_to_the_first_and_last_rows_of_a_context_menu_and_its_submenu() {
    let (mut fixture, rows) = menu(false);
    assert_eq!(press(&mut fixture, &rows, Key::End), Some("Echo"), "End before any row is focused");
    assert_eq!(press(&mut fixture, &rows, Key::Home), Some("Alpha"));
    assert_eq!(press(&mut fixture, &rows, Key::ArrowDown), Some("Charlie"));
    assert_eq!(press(&mut fixture, &rows, Key::End), Some("Echo"));
    assert_eq!(press(&mut fixture, &rows, Key::ArrowUp), Some("More"));
    assert_eq!(press(&mut fixture, &rows, Key::ArrowRight), Some("One"));
    assert_eq!(press(&mut fixture, &rows, Key::End), Some("Three"), "End in the submenu");
    assert_eq!(press(&mut fixture, &rows, Key::Home), Some("One"), "Home in the submenu");
}

#[trial]
fn a_menu_bars_menus_and_their_submenus_still_wrap() {
    let rows: Rows = Rc::default();
    let seen = rows.clone();
    let mut fixture = Fixture::new(move |ui| {
        let seen = seen.clone();
        let titles = vec![MenuTitle::new("&File", move |ui| {
            row(ui, &seen, "New", true);
            let more = Menu::submenu_row(ui, "Recent", |ui, _| {
                row(ui, &seen, "One", true);
                row(ui, &seen, "Two", true);
            });
            seen.borrow_mut().insert("Recent", more.response.id);
            row(ui, &seen, "Save", true);
        })];
        MenuBar::new("Main menu", titles).show(ui);
    });
    fixture.os(Os::Linux);
    fixture.settle();
    fixture.key(Key::F10, Modifiers::NONE);
    fixture.settle();
    fixture.key(Key::ArrowDown, Modifiers::NONE);
    fixture.settle();
    assert!(fixture.by_label("New").is_some(), "Down opens the focused title's menu");
    let mut at = focused(&fixture, &rows);
    // Down until Save holds the focus, then once more: the bar's menu continues at its first row.
    for _ in 0..4 {
        if at == Some("Save") {
            break;
        }
        at = press(&mut fixture, &rows, Key::ArrowDown);
    }
    assert_eq!(at, Some("Save"));
    assert_eq!(press(&mut fixture, &rows, Key::ArrowDown), Some("New"), "Down on the last row of a bar's menu goes to the first");
    assert_eq!(press(&mut fixture, &rows, Key::ArrowUp), Some("Save"), "Up on the first goes to the last");
    assert_eq!(press(&mut fixture, &rows, Key::ArrowUp), Some("Recent"));
    assert_eq!(press(&mut fixture, &rows, Key::ArrowRight), Some("One"));
    assert_eq!(press(&mut fixture, &rows, Key::ArrowUp), Some("Two"), "its submenu wraps as well");
}
