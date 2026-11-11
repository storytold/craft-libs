//! A press the key stage takes from egui takes its release with it, whether the release comes in the same pass or in
//! a later one; the release of a key the stage left alone still reaches egui.

use craft_keymap::{Chord, Mods};
use craft_ui::runtime::focus::{FocusTarget, KeyUse, request};
use craft_ui::testing::focus::{Stop, key_release, named_id, register_stop, take_claimed, widget};
use craft_ui::testing::input::key_press;
use craft_ui::testing::{Frames, trial};
use egui::{Event, Key, Modifiers, Ui};
use std::cell::Cell;

const CLAIMS: &[Chord] = &[Chord::new(Mods::CMD, craft_keymap::Key::K)];

fn is_release(event: &Event, key: Key) -> bool {
    matches!(event, Event::Key { key: up, pressed: false, .. } if *up == key)
}

/// Runs a pass in which a focused stop claims Cmd+K; `seen` collects whether a release of K reached egui.
fn rig(frames: &mut Frames, seen: &Cell<bool>, claimed: &Cell<bool>) {
    let mut show = |ui: &mut Ui| {
        let id = named_id("canvas");
        widget(ui, id);
        register_stop(ui.ctx(), Stop::new(id, KeyUse::CANVAS.with_claims(CLAIMS)));
        claimed.set(claimed.get() || take_claimed(ui.ctx(), id, CLAIMS[0]));
        seen.set(seen.get() || ui.input(|i| i.events.iter().any(|e| is_release(e, Key::K))));
    };
    frames.idle(2, &mut show);
    request(&frames.ctx, FocusTarget::Id(named_id("canvas")));
    frames.idle(2, &mut show);
    frames.frame(vec![key_press(Key::K, Modifiers::COMMAND)], &mut show);
    assert!(claimed.get(), "the shortcut took the press");
    frames.frame(vec![key_release(Key::K, Modifiers::COMMAND)], &mut show);
}

#[trial]
fn the_key_up_of_a_consumed_press_in_a_later_pass_reaches_no_component() {
    let mut frames = Frames::new();
    let (seen, claimed) = (Cell::new(false), Cell::new(false));
    rig(&mut frames, &seen, &claimed);
    assert!(!seen.get(), "the release of the consumed press was dropped");
}
