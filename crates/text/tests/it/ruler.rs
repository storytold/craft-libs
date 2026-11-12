//! The text ruler's keys: the ruler is one Tab stop, Page Down moves between its markers and stops, and the arrows
//! move the focused one.

use craft_text::ui::ruler::{RulerValues, TabAlign, TabLeader, TabStop, TextRuler};
use craft_ui::buttons::Button;
use craft_ui::contract::{Bind, Show as _, ShowContainer as _};
use craft_ui::layout_components::Stack;
use craft_ui::testing::{Fixture, trial};
use egui::{Key, Modifiers};
use std::cell::RefCell;
use std::rc::Rc;

/// Which of `labels` holds the focus.
fn focused<'a>(fixture: &Fixture, labels: &[&'a str]) -> Option<&'a str> {
    let focused = craft_ui::testing::access::focused_key(fixture.tree());
    labels.iter().copied().find(|label| fixture.by_label(label).is_some_and(|target| target.node == focused))
}

#[trial]
fn the_ruler_is_one_tab_stop_whose_page_keys_pick_and_arrows_move() {
    let mut start = RulerValues::default();
    start.stops = vec![TabStop::new(72.0, TabAlign::Left, TabLeader::None)];
    let values = Rc::new(RefCell::new(start));
    let shown = values.clone();
    let mut fixture = Fixture::new(move |ui| {
        Stack::new().show(ui, |ui, _| {
            Button::new("Before").show(ui);
            TextRuler::new("Ruler", Bind::bound(&mut *shown.borrow_mut()), 300.0).show(ui);
            Button::new("After").show(ui);
        });
    });
    fixture.idle(4);
    let labels = ["Before", "First Line Indent", "Hanging Indent", "After"];
    fixture.key(Key::Tab, Modifiers::NONE).idle(2);
    assert_eq!(focused(&fixture, &labels), Some("Before"), "Tab reaches the first button");
    fixture.key(Key::Tab, Modifiers::NONE).idle(2);
    assert_eq!(focused(&fixture, &labels), Some("First Line Indent"), "Tab enters the ruler at its first marker");
    fixture.key(Key::PageDown, Modifiers::NONE).idle(2);
    assert_eq!(focused(&fixture, &labels), Some("Hanging Indent"), "Page Down moves to the next marker");
    let before_move = values.borrow().clone();
    fixture.key(Key::ArrowRight, Modifiers::NONE).idle(2);
    assert_ne!(*values.borrow(), before_move, "Right moves the focused marker");
    assert_eq!(focused(&fixture, &labels), Some("Hanging Indent"), "Right keeps the focus on the marker");
    fixture.key(Key::Tab, Modifiers::NONE).idle(2);
    assert_eq!(focused(&fixture, &labels), Some("After"), "Tab leaves the ruler: its markers and stops are one stop");
}
