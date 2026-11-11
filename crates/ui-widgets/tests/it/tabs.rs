//! The tab strip's keys.

use craft_behavior::{SelectionChange, keyed};
use craft_ui::contract::{CollectionEvent, ShowCollection as _};
use craft_ui::tabs::{Tab, TabStrip};
use craft_ui::testing::{Fixture, trial};
use egui::{Key, Modifiers};
use std::cell::{Cell, RefCell};
use std::rc::Rc;

const TITLES: [&str; 3] = ["One", "Two", "Three"];

/// An arrow moves the focus to the next tab without showing it; Enter shows it.
#[trial]
fn an_arrow_moves_the_focus_and_enter_shows_the_tab() {
    let shown = Rc::new(Cell::new(0_usize));
    let picked = Rc::new(RefCell::new(Vec::new()));
    let (now, seen) = (shown.clone(), picked.clone());
    let mut fixture = Fixture::new(move |ui| {
        let keys = [0_usize, 1, 2];
        let source = keyed(&keys, |key: &usize| *key);
        let out = TabStrip::new("Documents").shown(now.get()).show_items(ui, &source, &mut |key, _| Tab::new(TITLES[*key]));
        for event in &out.response.events {
            if let CollectionEvent::Selection(SelectionChange::Set(keys)) = event
                && let Some(key) = keys.last()
            {
                seen.borrow_mut().push(*key);
                now.set(*key);
            }
        }
    });
    fixture.settle();
    fixture.key(Key::Tab, Modifiers::NONE);
    fixture.idle(2);
    fixture.key(Key::ArrowRight, Modifiers::NONE);
    fixture.idle(2);
    assert_eq!((shown.get(), picked.borrow().clone()), (0, Vec::new()), "the arrow shows no other tab");
    fixture.key(Key::Enter, Modifiers::NONE);
    fixture.idle(2);
    assert_eq!(shown.get(), 1, "Enter shows the focused tab");
}
