//! `Menu::tab_exits`: Tab in an anchored menu leaves it for the next stop after its opener and closes it without
//! running a row; without it, Tab stays in the menu.

use craft_ui::buttons::Button;
use craft_ui::contract::{Role, Show as _, ShowContainer as _, Trigger};
use craft_ui::menu::{Menu, MenuItem};
use craft_ui::runtime::focus::{self, FocusTarget};
use craft_ui::testing::{Fixture, trial};
use egui::{Id, Key, Modifiers};
use std::cell::Cell;
use std::rc::Rc;

/// "Before", a "Preset" button opening a menu of two presets, and "After"; the menu opened from the keyboard.
struct Rig {
    fixture: Fixture,
    ran: Rc<Cell<u32>>,
    before: Rc<Cell<Option<Id>>>,
    opener: Rc<Cell<Option<Id>>>,
    after: Rc<Cell<Option<Id>>>,
}

fn rig(exits: bool) -> Rig {
    rig_with_offer(exits, false)
}

fn rig_with_offer(exits: bool, offer: bool) -> Rig {
    let transient = Cell::new(None);
    let ran = Rc::new(Cell::new(0));
    let before = Rc::new(Cell::new(None));
    let after = Rc::new(Cell::new(None));
    let opener = Rc::new(Cell::new(None));
    let (shown_ran, shown_before, shown_after, shown_opener) = (ran.clone(), before.clone(), after.clone(), opener.clone());
    let mut fixture = Fixture::new(move |ui| {
        shown_before.set(Some(Button::new("Before").show(ui).id));
        let preset = Button::new("Preset").show(ui);
        shown_opener.set(Some(preset.id));
        let menu = Menu::new("Presets").opened_by(&preset, Trigger::Press);
        let menu = if exits { menu.tab_exits() } else { menu };
        menu.show(ui, |ui, _| {
            if MenuItem::new("Linear").show(ui).ran {
                shown_ran.set(shown_ran.get() + 1);
            }
            if MenuItem::new("Strong").show(ui).ran {
                shown_ran.set(shown_ran.get() + 1);
            }
        });
        if offer && !focus::focused_rec(ui.ctx()).is_some_and(|rec| Some(rec.stop.id) == transient.get()) {
            transient.set(Some(Button::new("Offer").show(ui).id));
        }
        shown_after.set(Some(Button::new("After").show(ui).id));
    });
    fixture.settle();
    focus::request(fixture.ctx(), FocusTarget::Id(opener.get().expect("the opener ran")));
    fixture.settle();
    fixture.key(Key::Enter, Modifiers::NONE);
    fixture.settle();
    assert!(fixture.by_label("Linear").is_some(), "Enter on the focused opener opens the menu");
    Rig { fixture, ran, before, opener, after }
}

impl Rig {
    fn menu_open(&self) -> bool {
        !self.fixture.by_role(Role::MENU).is_empty()
    }

    fn tab(&mut self) {
        self.fixture.key(Key::Tab, Modifiers::NONE);
        self.fixture.settle();
    }

    fn focused(&self) -> Option<Id> {
        self.fixture.focused_stop().map(|(id, _)| id)
    }
}

#[trial]
fn tab_stays_in_a_menu_by_default() {
    let mut rig = rig(false);
    rig.tab();
    assert!(rig.menu_open(), "the menu stays open");
    assert_ne!(rig.fixture.focused_stop().map(|(id, _)| id), rig.after.get(), "the focus stays in the menu");
}

#[trial]
fn tab_leaves_a_tab_exits_menu_for_the_stop_after_its_opener() {
    let mut rig = rig(true);
    rig.tab();
    assert!(!rig.menu_open(), "the menu closes");
    assert_eq!(rig.ran.get(), 0, "no row runs");
    assert_eq!(rig.fixture.focused_stop().map(|(id, _)| id), rig.after.get(), "the focus moves to the stop after the opener");
}

#[trial]
fn tab_leaves_the_menu_and_continues_past_a_stop_gone_on_arrival() {
    let mut rig = rig_with_offer(true, true);
    assert!(rig.fixture.by_label("Offer").is_some(), "the transient stop is the next entry after the opener");
    rig.tab();
    assert!(!rig.menu_open(), "the menu closes even though the first Tab destination went away");
    assert_eq!(rig.ran.get(), 0, "no row runs");
    assert_eq!(rig.focused(), rig.after.get(), "focus advances to the next still-shown entry");
}

#[trial]
fn arrows_still_move_through_a_tab_exits_menu() {
    let mut rig = rig(true);
    rig.fixture.key(Key::ArrowDown, Modifiers::NONE);
    rig.fixture.settle();
    assert!(rig.menu_open(), "an arrow key keeps the menu open");
    rig.fixture.key(Key::Enter, Modifiers::NONE);
    rig.fixture.settle();
    assert_eq!(rig.ran.get(), 1, "Enter runs the row the arrows reached");
}

#[trial]
fn shift_tab_leaves_a_tab_exits_menu_for_the_stop_before_its_opener() {
    let mut rig = rig(true);
    rig.fixture.key(Key::ArrowDown, Modifiers::NONE);
    rig.fixture.key(Key::Tab, Modifiers::SHIFT);
    rig.fixture.settle();
    assert!(!rig.menu_open(), "the menu closes");
    assert_eq!(rig.ran.get(), 0, "no row runs");
    assert_eq!(rig.focused(), rig.before.get(), "the focus moves to the stop before the opener, not back to the opener");
}

/// An Escape closes the menu and gives the focus back to its opener; a Tab pressed while the menu still plays its exit
/// goes on to the stop after the opener, and the closing menu does not take the focus back.
#[trial]
fn tab_right_after_escape_goes_on_from_the_opener_of_a_tab_exits_menu() {
    let mut rig = rig(true);
    rig.fixture.key(Key::ArrowDown, Modifiers::NONE);
    rig.fixture.key(Key::Escape, Modifiers::NONE);
    assert_eq!(rig.focused(), rig.opener.get(), "Escape gives the focus back to the opener");
    rig.fixture.key(Key::Tab, Modifiers::NONE);
    rig.fixture.settle();
    assert!(!rig.menu_open(), "the menu is closed");
    assert_eq!(rig.ran.get(), 0, "no row runs");
    assert_eq!(rig.focused(), rig.after.get(), "the focus goes on to the stop after the opener and stays there");
}

/// Where Tab reaches only text fields (a Mac's default), a `tab_exits` menu's non-text opener is no Tab entry itself: it
/// still holds its place in the order, so Tab goes on to the field after it and Shift+Tab to the one before.
#[trial]
fn tab_that_reaches_only_fields_leaves_a_tab_exits_menu_for_the_field_beside_its_opener() {
    let before = Rc::new(Cell::new(None));
    let after = Rc::new(Cell::new(None));
    let opener = Rc::new(Cell::new(None));
    let (shown_before, shown_after, shown_opener) = (before.clone(), after.clone(), opener.clone());
    let mut text = (String::new(), String::new());
    let mut fixture = Fixture::new(move |ui| {
        let mut first = std::mem::take(&mut text.0);
        shown_before.set(Some(craft_ui::text::TextField::new("Before", craft_ui::contract::Bind::bound(&mut first)).show(ui).response.id));
        text.0 = first;
        let preset = Button::new("Preset").show(ui);
        shown_opener.set(Some(preset.id));
        Menu::new("Presets").opened_by(&preset, Trigger::Press).tab_exits().show(ui, |ui, _| {
            MenuItem::new("Linear").show(ui);
        });
        let mut second = std::mem::take(&mut text.1);
        shown_after.set(Some(craft_ui::text::TextField::new("After", craft_ui::contract::Bind::bound(&mut second)).show(ui).response.id));
        text.1 = second;
    });
    fixture.os(craft_host::target::Os::Mac);
    let mut root = craft_ui::runtime::env::RootEnv::default();
    root.tab_all_controls = false;
    fixture.root(root);
    fixture.settle();
    focus::request(fixture.ctx(), FocusTarget::Id(opener.get().expect("the opener ran")));
    fixture.settle();
    fixture.key(Key::Enter, Modifiers::NONE);
    fixture.settle();
    assert!(fixture.by_label("Linear").is_some(), "Enter on the focused opener opens the menu");
    fixture.key(Key::Tab, Modifiers::NONE);
    fixture.settle();
    assert!(fixture.by_label("Linear").is_none(), "Tab leaves the menu");
    assert_eq!(fixture.focused_stop().map(|(id, _)| id), after.get(), "Tab goes on to the field after the opener");
}
