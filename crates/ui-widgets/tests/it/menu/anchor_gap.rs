//! A popup does not outlive its anchor's absence: a menu that was open when its opener stopped showing (the row it
//! acts on switched away, the panel it sits in stopped drawing) is closed when the opener shows again, with no input.
//! A fresh press opens it as before.

use craft_ui::buttons::Button;
use craft_ui::contract::{Props as _, Role, Show as _, ShowContainer as _, Trigger};
use craft_ui::layout_components::Stack;
use craft_ui::menu::{Menu, MenuItem};
use craft_ui::testing::{Fixture, pointer, trial};
use egui::Ui;
use std::cell::Cell;
use std::rc::Rc;

#[derive(Clone, Copy, Debug)]
enum Opener {
    Click,
    Context,
}

#[derive(Clone, Copy, Debug)]
enum Gone {
    /// The opener alone is not shown (the row it acts on switched away).
    Opener,
    /// The panel the opener sits in stops drawing.
    Panel,
}

fn opener_and_menu(ui: &mut Ui, opener: Opener) {
    let button = Button::new("Options").show(ui);
    let menu = match opener {
        Opener::Click => Menu::new("Options menu").opened_by(&button, Trigger::Click),
        Opener::Context => Menu::context("Options menu", &button),
    };
    menu.show(ui, |ui, _| {
        MenuItem::new("Invert").show(ui);
        MenuItem::new("Duplicate").show(ui);
    });
}

fn rig(opener: Opener, gone: Gone) -> (Fixture, Rc<Cell<bool>>) {
    let shown = Rc::new(Cell::new(true));
    let showing = shown.clone();
    let fixture = Fixture::new(move |ui| match gone {
        Gone::Opener => {
            if showing.get() {
                opener_and_menu(ui, opener);
            }
        }
        Gone::Panel => {
            if showing.get() {
                Stack::new().id_salt("panel").show(ui, |ui, _| opener_and_menu(ui, opener));
            }
        }
    });
    (fixture, shown)
}

fn open(fixture: &mut Fixture, opener: Opener) {
    let at = fixture.by_label("Options").expect("the opener shows").center();
    match opener {
        Opener::Click => {
            fixture.click_at(at);
        }
        Opener::Context => {
            fixture.frame(vec![pointer::moved(at)]);
            fixture.frame(vec![pointer::secondary(at, true)]);
            fixture.frame(vec![pointer::secondary(at, false)]);
        }
    }
    fixture.settle();
}

fn menu_open(fixture: &Fixture) -> bool {
    !fixture.by_role(Role::MENU).is_empty()
}

#[trial]
fn a_menu_open_when_its_opener_stopped_showing_is_closed_when_the_opener_returns() {
    for opener in [Opener::Click, Opener::Context] {
        for gone in [Gone::Opener, Gone::Panel] {
            for frames in [1, 3] {
                let case = format!("{opener:?} menu, {gone:?} gone for {frames} frame(s)");
                let (mut fixture, shown) = rig(opener, gone);
                fixture.settle();
                open(&mut fixture, opener);
                assert!(menu_open(&fixture), "{case}: the menu opens");
                shown.set(false);
                for _ in 0..frames {
                    fixture.frame(vec![]);
                }
                assert!(!menu_open(&fixture), "{case}: no menu while its opener is gone");
                shown.set(true);
                fixture.frame(vec![]);
                fixture.settle();
                assert!(!menu_open(&fixture), "{case}: the menu stays closed when its opener shows again, with no input");
                open(&mut fixture, opener);
                assert!(menu_open(&fixture), "{case}: a fresh press opens it again");
            }
        }
    }
}

/// An opener clicked on the pass its container returns to after an absence toggles from the state the gap left: the menu
/// opens, and the pass measures what it shows (a peek and the real run read one open state).
#[trial]
fn an_opener_clicked_on_the_pass_its_container_returns_opens_the_menu() {
    let (mut fixture, shown) = rig(Opener::Click, Gone::Opener);
    fixture.settle();
    open(&mut fixture, Opener::Click);
    assert!(menu_open(&fixture), "the menu opens");
    let at = fixture.by_label("Options").expect("the opener shows").center();
    shown.set(false);
    fixture.frame(vec![]);
    shown.set(true);
    fixture.click_at(at);
    fixture.settle();
    assert!(menu_open(&fixture), "the click opens the menu from the closed state the gap left");
}
