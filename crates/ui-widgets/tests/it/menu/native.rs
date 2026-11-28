//! The hand-off of what the platform's menu queues: it reaches the frame that is running.

use craft_keymap::{Chord, Key, Mods};
use craft_ui::runtime::windows::{KeyEquivalent, WindowHost};
use craft_ui::testing::{Frames, trial};
use egui::{Ui, ViewportId};
use std::cell::Cell;

/// A key equivalent queued by a pass (the native menu's `poll` inside `App::update`) acts in that frame: the filter
/// repeats the pass with the key press in its input.
#[trial]
fn a_key_equivalent_queued_inside_the_frame_acts_in_that_frame() {
    let mut frames = Frames::new();
    let (queued, pressed) = (Cell::new(false), Cell::new(false));
    let mut show = |ui: &mut Ui| {
        if !queued.replace(true) {
            WindowHost::of(ui.ctx()).key_equivalent(ViewportId::ROOT, KeyEquivalent::Press(Chord::new(Mods::NONE, Key::F9)));
        }
        pressed.set(
            pressed.get()
                || ui.input(|input| input.events.iter().any(|event| matches!(event, egui::Event::Key { key: egui::Key::F9, pressed: true, .. }))),
        );
    };
    frames.idle(2, |_| {});
    queued.set(false);
    frames.frame(vec![], &mut show);
    assert!(pressed.get(), "the pass that queued the equivalent was repeated with the key press in its input");
}
