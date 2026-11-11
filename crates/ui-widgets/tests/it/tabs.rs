//! The tab strip's keys, and a tab's close button, which the tab declares as a part while it is a member of the
//! strip's run.

use craft_behavior::{SelectionChange, keyed};
use craft_ui::contract::{CollectionEvent, ShowCollection as _};
use craft_ui::tabs::{Tab, TabStrip};
use craft_ui::testing::focus::AccessActionKind;
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

/// A click on a tab's close button closes the tab, and the close button is a node under its tab's node that an
/// AccessKit Click closes.
#[trial]
fn the_close_button_and_its_node_close_a_tab() {
    let closed = Rc::new(RefCell::new(Vec::new()));
    let seen = closed.clone();
    let mut fixture = Fixture::new(move |ui| {
        let keys = [0_usize, 1, 2];
        let source = keyed(&keys, |key: &usize| *key);
        let out = TabStrip::new("Documents").shown(0).show_items(ui, &source, &mut |key, _| Tab::new(TITLES[*key]));
        seen.borrow_mut().extend(out.closed);
    });
    fixture.settle();
    let tab = fixture.by_label("Two").expect("the tab's node");
    let close = fixture.by_label("Close Two").expect("the close button's node");
    assert_eq!(fixture.tree().parent_of_key(close.node), Some(tab.node), "the close button's node is under its tab's");
    fixture.click(&close);
    assert_eq!(closed.borrow().clone(), vec![1], "the close button closes its tab");
    let first = fixture.by_label("Close One").expect("the close button's node");
    fixture.access(AccessActionKind::Click, &first).idle(1);
    assert_eq!(closed.borrow().clone(), vec![1, 0], "an AccessKit Click on the close node closes its tab");
}
