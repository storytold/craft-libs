//! Focus scopes: initial focus, restore, trapping, panes, programmatic requests and focus after removal.

use super::{control, focus, focused, id, tap};
use craft_ui::testing::{Frames, trial};
use egui::Ui;
use std::cell::Cell;

/// Spec test row "when the focused component is not shown, focus goes nowhere": no stop inherits it, whether stops
/// are left after the removed one or only before it.
#[trial]
fn focus_goes_nowhere_when_the_focused_stop_is_not_shown() {
    let mut frames = Frames::new();
    let hidden = Cell::new(None);
    let mut show = |ui: &mut Ui| {
        for name in ["one", "two", "three"] {
            if hidden.get() != Some(name) {
                control(ui, name);
            }
        }
    };
    frames.idle(2, &mut show);
    focus(&mut frames, "two", &mut show);
    frames.idle(1, &mut show);
    hidden.set(Some("two"));
    frames.idle(2, &mut show);
    assert_eq!(focused(&frames.ctx), None, "a stop after the removed one does not take the focus");
    focus(&mut frames, "three", &mut show);
    frames.idle(1, &mut show);
    hidden.set(Some("three"));
    frames.idle(2, &mut show);
    assert_eq!(focused(&frames.ctx), None, "no stop after it: the one before does not take the focus either");
}

/// A stop that is shown only while another one holds the focus (a field's completion list) is not shown in the pass a
/// Tab moves the focus onto it: the focus goes on to the next stop that is shown, not nowhere.
#[trial]
fn tab_onto_a_stop_gone_in_the_same_pass_goes_on_to_the_next_stop() {
    let mut frames = Frames::new();
    let mut show = |ui: &mut Ui| {
        control(ui, "field");
        if focused(ui.ctx()) == Some(id("field")) {
            control(ui, "offer");
        }
        control(ui, "next");
    };
    frames.idle(2, &mut show);
    focus(&mut frames, "field", &mut show);
    frames.idle(1, &mut show);
    tap(&mut frames, egui::Key::Tab, egui::Modifiers::NONE, &mut show);
    frames.idle(2, &mut show);
    assert_eq!(focused(&frames.ctx), Some(id("next")), "the Tab goes on past the stop that went away");
}

/// With no stop shown after the one that went away, the Tab gives the focus back to the stop it left.
#[trial]
fn tab_onto_the_last_stop_gone_in_the_same_pass_stays_where_it_was() {
    let mut frames = Frames::new();
    let mut show = |ui: &mut Ui| {
        control(ui, "field");
        if focused(ui.ctx()) == Some(id("field")) {
            control(ui, "offer");
        }
    };
    frames.idle(2, &mut show);
    focus(&mut frames, "field", &mut show);
    frames.idle(1, &mut show);
    tap(&mut frames, egui::Key::Tab, egui::Modifiers::NONE, &mut show);
    frames.idle(2, &mut show);
    assert_eq!(focused(&frames.ctx), Some(id("field")), "the focus stays on the stop the Tab left");
}

/// A Tab onto the last entry that went away wraps to the first entry, as Tab does from the last stop, rather than
/// staying on the stop it left.
#[trial]
fn tab_onto_the_last_stop_gone_in_the_same_pass_wraps_to_the_first() {
    let mut frames = Frames::new();
    let mut show = |ui: &mut Ui| {
        control(ui, "first");
        control(ui, "field");
        if focused(ui.ctx()) == Some(id("field")) {
            control(ui, "offer");
        }
    };
    frames.idle(2, &mut show);
    focus(&mut frames, "field", &mut show);
    frames.idle(1, &mut show);
    tap(&mut frames, egui::Key::Tab, egui::Modifiers::NONE, &mut show);
    frames.idle(2, &mut show);
    assert_eq!(focused(&frames.ctx), Some(id("first")), "the last entry gone, Tab wraps to the first, as it does from any last stop");
}

/// A Tab from no focus at all can land on a stop this pass does not show (one drawn only while it lacks the focus):
/// the direction is the Tab's own, so the focus goes on to the next shown stop rather than nowhere.
#[trial]
fn tab_from_no_focus_onto_a_stop_gone_in_the_same_pass_goes_on() {
    let mut frames = Frames::new();
    let mut show = |ui: &mut Ui| {
        if focused(ui.ctx()) != Some(id("offer")) {
            control(ui, "offer");
        }
        control(ui, "first");
        control(ui, "next");
    };
    frames.idle(2, &mut show);
    assert_eq!(focused(&frames.ctx), None, "nothing has the focus");
    tap(&mut frames, egui::Key::Tab, egui::Modifiers::NONE, &mut show);
    frames.idle(2, &mut show);
    assert_eq!(focused(&frames.ctx), Some(id("first")), "the Tab goes on past the stop it landed on");
}
