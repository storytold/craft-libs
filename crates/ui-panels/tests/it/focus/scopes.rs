//! Focus scopes: initial focus, restore, trapping, panes, programmatic requests and focus after removal.

use super::{control, focus, focused, id};
use craft_ui::testing::{Frames, trial};
use egui::Ui;
use std::cell::Cell;

/// Spec test row "when the focused component is not shown, focus moves to the next stop in its scope, else
/// the previous".
#[trial]
fn focus_moves_to_the_next_stop_when_the_focused_one_is_not_shown_else_the_previous() {
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
    assert_eq!(focused(&frames.ctx), Some(id("three")), "the stop after the removed one");
    hidden.set(Some("three"));
    frames.idle(2, &mut show);
    assert_eq!(focused(&frames.ctx), Some(id("one")), "no stop after it: the one before");
}
