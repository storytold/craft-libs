//! A menu inside a memoized view: while the view replays, the container does not run, so its opener slot's
//! last-shown frame must be re-issued by the replay. Otherwise the pass the view runs again reads the container as
//! gone and closes a menu it still shows.

use craft_ui::buttons::Button;
use craft_ui::contract::{MemoProps, MemoView, Show as _, ShowContainer as _, Trigger};
use craft_ui::menu::{Menu, MenuItem};
use craft_ui::runtime::focus::{self, FocusTarget};
use craft_ui::testing::{Fixture, trial};
use egui::{Id, Key, Modifiers, Ui};
use std::cell::{Cell, RefCell};
use std::rc::Rc;

thread_local! {
    /// The id of the "Options" opener's response, from its last run.
    static OPENER: RefCell<Option<Id>> = const { RefCell::new(None) };
}

/// A bar whose opener hangs the "Options menu"; `version` is its key, so a changed version runs the view again.
#[derive(MemoProps)]
struct Bar {
    version: u32,
}

fn bar(props: &mut Bar, ui: &mut Ui) {
    let _ = props.version;
    let picker = Button::new("Options").show(ui);
    OPENER.with(|opener| *opener.borrow_mut() = Some(picker.id));
    Menu::new("Options menu").opened_by(&picker, Trigger::Click).show(ui, |ui, _| {
        MenuItem::new("Invert").show(ui);
    });
}

#[trial]
fn a_menu_in_a_memoized_view_stays_open_when_the_view_runs_again_after_replays() {
    let version = Rc::new(Cell::new(0u32));
    let key = Rc::clone(&version);
    let mut fixture = Fixture::new(move |ui| {
        let _ = MemoView::new("bar", Bar { version: key.get() }, bar).show(ui);
    });
    fixture.settle();
    let opener = OPENER.with(|opener| opener.borrow().expect("the opener ran"));
    focus::request(fixture.ctx(), FocusTarget::Id(opener));
    fixture.settle();
    fixture.key(Key::Enter, Modifiers::NONE);
    fixture.settle();
    assert!(fixture.by_label("Invert").is_some(), "Enter on the opener opens the menu");
    for _ in 0..3 {
        fixture.frame(vec![]);
        fixture.settle();
    }
    assert!(fixture.by_label("Invert").is_some(), "the menu stays open while the view replays it");
    version.set(1);
    fixture.frame(vec![]);
    fixture.settle();
    assert!(fixture.by_label("Invert").is_some(), "the menu stays open in the pass the view runs again");
}

#[trial]
fn a_menu_is_closed_when_its_memoized_view_shows_again_after_an_absence() {
    let version = Rc::new(Cell::new(0u32));
    let shown = Rc::new(Cell::new(true));
    let (key, showing) = (Rc::clone(&version), Rc::clone(&shown));
    let mut fixture = Fixture::new(move |ui| {
        if showing.get() {
            let _ = MemoView::new("bar", Bar { version: key.get() }, bar).show(ui);
        }
    });
    fixture.settle();
    let opener = OPENER.with(|opener| opener.borrow().expect("the opener ran"));
    focus::request(fixture.ctx(), FocusTarget::Id(opener));
    fixture.settle();
    fixture.key(Key::Enter, Modifiers::NONE);
    fixture.settle();
    assert!(fixture.by_label("Invert").is_some(), "Enter on the opener opens the menu");
    shown.set(false);
    fixture.frame(vec![]);
    fixture.frame(vec![]);
    shown.set(true);
    fixture.frame(vec![]);
    fixture.settle();
    assert!(fixture.by_label("Invert").is_none(), "the menu does not survive the view's absence");
}
