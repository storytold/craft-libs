//! Focus scopes: initial focus, restore, trapping, panes, programmatic requests and focus after removal.

use super::{control, focus, focused};
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
