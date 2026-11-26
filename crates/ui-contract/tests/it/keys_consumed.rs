//! A press the key stage takes from egui takes its release with it, whether the release comes in the same pass or in
//! a later one; the release of a key the stage left alone still reaches egui. A captured press takes the text it
//! typed too, so a focused field types nothing for it.

use craft_keymap::{Chord, Mods};
use craft_ui::runtime::focus::{FocusTarget, KeyUse, request};
use craft_ui::testing::focus::{Stop, key_release, named_id, register_stop, take_claimed, widget};
use craft_ui::testing::input::{key_press, text};
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
    assert!(craft_ui::runtime::keys::focus_holds(&frames.ctx, CLAIMS[0]), "the focused stop names the chord it claims as its own");
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

#[trial]
fn a_claimed_press_takes_its_text_and_leaves_other_typing() {
    use craft_behavior::Input;
    let mut frames = Frames::new();
    let alt_7 = Chord::new(Mods::ALT, craft_keymap::Key::Num7);
    let (claimed, typed) = (Cell::new(false), std::cell::RefCell::new(Vec::<String>::new()));
    let mut show = |ui: &mut Ui| {
        let field = named_id("field");
        widget(ui, field);
        register_stop(ui.ctx(), Stop::new(field, KeyUse::of(Input::Text)));
        let owner = named_id("resample-keys");
        craft_ui::runtime::keys::capture(ui.ctx(), owner, alt_7);
        claimed.set(claimed.get() || take_claimed(ui.ctx(), owner, alt_7));
        ui.input(|i| {
            typed.borrow_mut().extend(i.events.iter().filter_map(|e| match e {
                Event::Text(t) => Some(t.clone()),
                _ => None,
            }))
        });
    };
    frames.idle(2, &mut show);
    request(&frames.ctx, FocusTarget::Id(named_id("field")));
    frames.idle(2, &mut show);
    frames.frame(vec![key_press(Key::Num7, Modifiers::ALT), text("¶"), key_press(Key::A, Modifiers::NONE), text("a")], &mut show);
    assert!(claimed.get(), "the capture took Alt+7");
    assert_eq!(*typed.borrow(), ["a"], "the captured press typed nothing; the next key's text reached the field");
}

/// An app consumes several canvas shortcuts in arrival order, rather than the order of its binding table.
#[trial]
fn an_app_reads_claimed_chords_in_order_without_taking_another_owners_or_excluded_keys() {
    use craft_ui::runtime::keys::{claim, take_first_claimed};
    use craft_ui::testing::measuring_run;

    const THREE: Chord = Chord::new(Mods::ALT, craft_keymap::Key::Num3);
    const FIVE: Chord = Chord::new(Mods::ALT, craft_keymap::Key::Num5);
    const SEVEN: Chord = Chord::new(Mods::ALT, craft_keymap::Key::Num7);
    const NINE: Chord = Chord::new(Mods::ALT, craft_keymap::Key::Num9);
    let mut frames = Frames::new();
    let canvas = named_id("ordered canvas");
    let other = named_id("other owner");
    let read = Cell::new(false);
    let mut show = |ui: &mut Ui| {
        widget(ui, canvas);
        register_stop(ui.ctx(), Stop::new(canvas, KeyUse::CANVAS));
        for chord in [THREE, SEVEN, NINE] {
            claim(ui.ctx(), canvas, chord);
        }
        craft_ui::runtime::keys::capture(ui.ctx(), other, FIVE);
        if read.replace(false) {
            // A measuring pass must leave the real pass's pending input untouched.
            assert_eq!(measuring_run(ui.ctx(), || take_first_claimed(ui.ctx(), canvas, &[THREE, NINE])), None);
            assert_eq!(take_first_claimed(ui.ctx(), other, &[THREE, NINE]), None, "another owner cannot take the canvas's input");
            assert_eq!(take_first_claimed(ui.ctx(), canvas, &[THREE, NINE]), Some(NINE));
            assert_eq!(take_first_claimed(ui.ctx(), canvas, &[THREE, NINE]), Some(THREE));
            assert_eq!(take_first_claimed(ui.ctx(), canvas, &[THREE, NINE]), None, "each press is consumed once");
            assert_eq!(take_first_claimed(ui.ctx(), canvas, &[FIVE]), None);
            assert_eq!(take_first_claimed(ui.ctx(), other, &[FIVE]), Some(FIVE), "the other owner's input remains");
            assert_eq!(take_first_claimed(ui.ctx(), canvas, &[SEVEN]), Some(SEVEN), "an excluded chord remains");
            assert_eq!(take_first_claimed(ui.ctx(), canvas, &[SEVEN]), None);
        }
    };
    frames.idle(2, &mut show);
    request(&frames.ctx, FocusTarget::Id(canvas));
    frames.idle(2, &mut show);
    read.set(true);
    frames.frame(
        vec![
            key_press(Key::Num9, Modifiers::ALT),
            key_release(Key::Num9, Modifiers::ALT),
            key_press(Key::Num3, Modifiers::ALT),
            key_release(Key::Num3, Modifiers::ALT),
            key_press(Key::Num7, Modifiers::ALT),
            key_release(Key::Num7, Modifiers::ALT),
            key_press(Key::Num5, Modifiers::ALT),
            key_release(Key::Num5, Modifiers::ALT),
            Event::ModifiersChanged(Modifiers::NONE),
        ],
        &mut show,
    );
    assert!(!read.get(), "the pass read the real key input");
}
