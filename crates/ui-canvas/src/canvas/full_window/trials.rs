//! Trials of the full-window dialog shell's keys.

use super::FullWindowDialog;
use crate::choice::Checkbox;
use crate::contract::{Bind, Props as _, Show as _, ShowContainer as _};
use crate::runtime::focus::Initial;
use crate::sections::{DialogRole, FooterButton};
use crate::testing::input::key_press;
use crate::testing::{Frames, trial};
use egui::{Key, Modifiers, Ui};
use std::cell::Cell;

/// A full-window dialog has no overlay layer; its footer letters follow its own focus scope.
#[trial]
fn a_footer_letter_activates_its_button_in_a_full_window_dialog() {
    let picked = Cell::new(None);
    let mut frames = Frames::new();
    let mut ui = |ui: &mut Ui| {
        let (mut open, mut snap) = (true, false);
        let shown = FullWindowDialog::new("Preferences")
            .opened(Bind::bound(&mut open))
            .initial_focus(Initial::First)
            .buttons([FooterButton::new("Reset", DialogRole::Destructive).letter('r'), FooterButton::new("Close", DialogRole::Cancel)])
            .show(ui, |ui, _| {
                Checkbox::new(Bind::bound(&mut snap)).label("Snap").show(ui);
            });
        if shown.output.picked.is_some() {
            picked.set(shown.output.picked);
        }
    };
    frames.idle(30, &mut ui);
    frames.frame(vec![key_press(Key::R, Modifiers::NONE)], &mut ui);
    frames.idle(2, &mut ui);
    assert_eq!(picked.get(), Some(0), "the letter picks its button");
}
