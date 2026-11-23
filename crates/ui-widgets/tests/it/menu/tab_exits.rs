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
    after: Rc<Cell<Option<Id>>>,
}

fn rig(exits: bool) -> Rig {
    let ran = Rc::new(Cell::new(0));
    let after = Rc::new(Cell::new(None));
    let opener = Rc::new(Cell::new(None));
    let (shown_ran, shown_after, shown_opener) = (ran.clone(), after.clone(), opener.clone());
    let mut fixture = Fixture::new(move |ui| {
        Button::new("Before").show(ui);
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
        shown_after.set(Some(Button::new("After").show(ui).id));
    });
    fixture.settle();
    focus::request(fixture.ctx(), FocusTarget::Id(opener.get().expect("the opener ran")));
    fixture.settle();
    fixture.key(Key::Enter, Modifiers::NONE);
    fixture.settle();
    assert!(fixture.by_label("Linear").is_some(), "Enter on the focused opener opens the menu");
    Rig { fixture, ran, after }
}

impl Rig {
    fn menu_open(&self) -> bool {
        !self.fixture.by_role(Role::MENU).is_empty()
    }

    fn tab(&mut self) {
        self.fixture.key(Key::Tab, Modifiers::NONE);
        self.fixture.settle();
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
fn arrows_still_move_through_a_tab_exits_menu() {
    let mut rig = rig(true);
    rig.fixture.key(Key::ArrowDown, Modifiers::NONE);
    rig.fixture.settle();
    assert!(rig.menu_open(), "an arrow key keeps the menu open");
    rig.fixture.key(Key::Enter, Modifiers::NONE);
    rig.fixture.settle();
    assert_eq!(rig.ran.get(), 1, "Enter runs the row the arrows reached");
}
