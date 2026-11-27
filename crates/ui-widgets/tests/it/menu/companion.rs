//! A menu's companion bar: a layer above the menu in its press family.

use craft_ui::buttons::Button;
use craft_ui::contract::{InBar, Role, Show as _, ShowContainer as _, Trigger};
use craft_ui::display::Divider;
use craft_ui::menu::{Menu, MenuItem};
use craft_ui::runtime::env::Env;
use craft_ui::testing::{Fixture, pointer, trial};
use egui::{Key, Modifiers, Pos2, Ui, Vec2};
use std::cell::Cell;
use std::rc::Rc;

/// A page with a target to right-click, a context menu with a companion bar holding a Size button that opens a list,
/// and a page button far from both. `page` counts the page button's clicks.
fn fixture(page: &Rc<Cell<u32>>) -> Fixture {
    let page = Rc::clone(page);
    let mut fixture = Fixture::new(move |ui| {
        ui.add_space(120.0);
        let target = Button::new("Target").show(ui);
        ui.add_space(300.0);
        if Button::new("Page").show(ui).clicks > 0 {
            page.set(page.get() + 1);
        }
        let mut bar = |ui: &mut Ui| {
            let size = Button::new("Size").show(ui);
            Menu::new("Size list").opened_by(&size, Trigger::Click).show(ui, |ui, _| {
                MenuItem::new("Twelve").show(ui);
            });
        };
        Menu::context("Target menu", &target).companion(&mut bar).show(ui, |ui, _| {
            MenuItem::new("Copy").show(ui);
        });
    });
    fixture.room(Vec2::new(800.0, 640.0));
    fixture
}

/// Right-clicks at `at`: move, secondary press, secondary release.
fn right_click(fixture: &mut Fixture, at: Pos2) {
    fixture.frame(vec![pointer::moved(at)]);
    fixture.frame(vec![pointer::secondary(at, true)]);
    fixture.frame(vec![pointer::secondary(at, false)]);
    fixture.settle();
}

/// Opens the context menu on the target and the Size list from the bar.
fn open_all(fixture: &mut Fixture) {
    fixture.settle();
    let target = fixture.by_label("Target").expect("the target has a node").center();
    right_click(fixture, target);
    assert!(fixture.by_label("Copy").is_some(), "the right-click opens the menu");
    assert!(fixture.by_label("Size").is_some(), "the menu shows its companion bar");
}

#[trial]
fn a_press_on_the_companion_bar_or_its_popup_does_not_close_the_menu_and_the_bar_closes_with_the_menu() {
    let page = Rc::new(Cell::new(0));
    let mut fixture = fixture(&page);
    open_all(&mut fixture);

    let size = fixture.by_label("Size").expect("the bar's button");
    fixture.click(&size).settle();
    assert!(fixture.by_label("Twelve").is_some(), "the bar's button opens its list");
    assert!(fixture.by_label("Copy").is_some(), "a press on the bar leaves the menu open");

    let twelve = fixture.by_label("Twelve").expect("the list's row");
    let list = fixture.by_role(Role::MENU).into_iter().find(|menu| menu.rect.contains(twelve.center())).expect("the list's panel");
    fixture.click_at(list.rect.min + Vec2::new(1.0, 1.0)).settle();
    assert!(fixture.by_label("Copy").is_some(), "a press on the bar's popup leaves the menu open");
    assert_eq!(page.get(), 0, "no press reached the page");

    let at = fixture.by_label("Page").expect("the page button").center();
    fixture.click_at(at).settle();
    assert!(fixture.by_label("Copy").is_none(), "a press outside closes the menu");
    assert!(fixture.by_label("Size").is_none(), "the bar closes with the menu");
    assert_eq!(page.get(), 0, "the press that closed the menu did not reach the page");
}

#[trial]
fn a_companion_bars_combo_box_is_compact() {
    let mut fixture = Fixture::new(|ui| {
        let target = Button::new("Target").show(ui);
        let options = [craft_ui::contract::Entry::new("8".to_owned(), "8"), craft_ui::contract::Entry::new("72".to_owned(), "72")];
        let mut size = "11".to_owned();
        let mut bar = |ui: &mut Ui| {
            let _ = craft_ui::pickers::ComboBox::new("Font Size", craft_ui::contract::Bind::bound(&mut size), &options).show(ui);
        };
        Menu::context("Target menu", &target).companion(&mut bar).show(ui, |ui, _| {
            MenuItem::new("Copy").show(ui);
        });
    });
    fixture.room(Vec2::new(800.0, 640.0));
    fixture.settle();
    let target = fixture.by_label("Target").expect("the target has a node").center();
    right_click(&mut fixture, target);
    let combo = fixture.by_label("Font Size").expect("the combo box in companion bar");
    assert!(combo.rect.width() < 100.0, "combo box in companion bar is compact: {}", combo.rect.width());
}

#[trial]
fn escape_closes_the_topmost_layer_first() {
    let page = Rc::new(Cell::new(0));
    let mut fixture = fixture(&page);
    open_all(&mut fixture);
    let size = fixture.by_label("Size").expect("the bar's button");
    fixture.click(&size).settle();
    assert!(fixture.by_label("Twelve").is_some(), "the list is open");

    fixture.key(Key::Escape, Modifiers::NONE).settle();
    assert!(fixture.by_label("Twelve").is_none(), "Escape closes the bar's list first");
    assert!(fixture.by_label("Copy").is_some() && fixture.by_label("Size").is_some(), "the menu and its bar stay after the first Escape");

    fixture.key(Key::Escape, Modifiers::NONE).settle();
    assert!(fixture.by_label("Copy").is_none(), "the next Escape closes the menu");
    assert!(fixture.by_label("Size").is_none(), "the bar goes with the menu");
}

#[trial]
fn companion_layer_has_bar_padding_inbar_env_and_start_alignment() {
    let in_bar_seen = Rc::new(Cell::new(false));
    let in_bar_clone = Rc::clone(&in_bar_seen);

    let mut fixture = Fixture::new(move |ui| {
        ui.add_space(120.0);
        let target = Button::new("Target").show(ui);
        let in_bar = Rc::clone(&in_bar_clone);
        let mut bar = move |ui: &mut Ui| {
            in_bar.set(Env::of(ui).get::<InBar>().0);
            Button::new("Bold").show(ui);
            Divider::vertical().show(ui);
            Button::new("Italic").show(ui);
            Button::new("Underline").show(ui);
        };
        Menu::context("Target menu", &target).companion(&mut bar).show(ui, |ui, _| {
            MenuItem::new("Copy").show(ui);
        });
    });
    fixture.room(Vec2::new(800.0, 640.0));
    fixture.settle();

    let target = fixture.by_label("Target").expect("the target has a node").center();
    right_click(&mut fixture, target);

    assert!(fixture.by_label("Copy").is_some(), "the right-click opens the menu");
    assert!(fixture.by_label("Bold").is_some(), "the companion bar shows its buttons");

    // 1. InBar environment propagation
    assert!(in_bar_seen.get(), "InBar(true) is set in the companion bar content env");

    // 2. Bar padding (Pad::Control) around the companion layer's controls
    let companion_bar = fixture.by_label("Menu bar").expect("companion bar group container");
    let bold = fixture.by_label("Bold").expect("first button in companion bar");
    let underline = fixture.by_label("Underline").expect("last button in companion bar");

    assert!(companion_bar.rect.left() < bold.rect.left(), "companion bar has left padding");
    assert!(companion_bar.rect.top() < bold.rect.top(), "companion bar has top padding");
    assert!(companion_bar.rect.right() > underline.rect.right(), "companion bar has right padding");
    assert!(companion_bar.rect.bottom() > underline.rect.bottom(), "companion bar has bottom padding");

    // 3. Start alignment (Align::Start) relative to parent Menu panel
    let menu = fixture
        .by_role(Role::MENU)
        .into_iter()
        .find(|m| fixture.by_label("Copy").is_some_and(|c| m.rect.contains(c.center())))
        .expect("parent context menu panel");

    assert!(
        (companion_bar.rect.left() - menu.rect.left()).abs() < 1.0,
        "companion bar aligns with menu's start edge: bar left={}, menu left={}",
        companion_bar.rect.left(),
        menu.rect.left()
    );
    assert!(
        companion_bar.rect.bottom() <= menu.rect.top() + 1.0,
        "companion bar is anchored above the menu: bar bottom={}, menu top={}",
        companion_bar.rect.bottom(),
        menu.rect.top()
    );
}
