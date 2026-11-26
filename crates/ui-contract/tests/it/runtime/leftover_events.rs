//! `runtime::keys::leftover_events`: the keys of a pass that nothing took, for a host that runs shortcuts of its own on
//! a window's input.

use craft_keymap::Overrides;
use craft_ui::contract::Tracked;
use craft_ui::runtime::commands::{
    AppCatalog, Binding, CommandDef, CommandInput, CommandRef, CommandSet as _, CommandState, CommandStates, EditCommands, Scope, Trigger,
};
use craft_ui::runtime::frame::take_output;
use craft_ui::runtime::keys::leftover_events;
use craft_ui::testing::focus::key_release;
use craft_ui::testing::input::{key_press, key_repeat, text};
use craft_ui::testing::{Frames, trial};
use egui::{Event, Key, Modifiers, Ui};
use std::cell::{Cell, RefCell};
use std::sync::Arc;

/// The presses and releases of `events`, in order, each with whether it is a press.
fn key_events(events: Vec<Event>) -> Vec<(Key, bool)> {
    events.into_iter().filter_map(|event| if let Event::Key { key, pressed, .. } = event { Some((key, pressed)) } else { None }).collect()
}

/// The texts of `events` the host pastes, in order.
fn pasted(events: Vec<Event>) -> Vec<String> {
    events.into_iter().filter_map(|event| if let Event::Paste(text) = event { Some(text) } else { None }).collect()
}

/// With nothing focused, a host reading the leftover events after the window's content gets the press and release of
/// a key no command is bound to, and neither the press nor the release of a key whose command fired.
#[trial]
fn leftover_events_holds_the_keys_no_command_took() {
    let mut frames = Frames::new();
    let binding = Binding::parse("F1").expect("the trial's chord parses");
    let help = CommandDef::new("leftoverTrial.help", "Help", Scope::app()).with_default(binding);
    frames.commands(CommandInput::new().catalog(AppCatalog::new(vec![help], 1).expect("the trial's catalog is valid")));
    let left = RefCell::new(Vec::new());
    let mut show = |ui: &mut Ui| {
        left.borrow_mut().extend(leftover_events(ui.ctx()));
    };
    frames.idle(2, &mut show);
    take_output(&frames.ctx);
    left.borrow_mut().clear();
    let keys = |events: Vec<Event>| -> Vec<(Key, bool)> {
        events.into_iter().filter_map(|event| if let Event::Key { key, pressed, .. } = event { Some((key, pressed)) } else { None }).collect()
    };

    frames.frame(vec![key_press(Key::F1, Modifiers::NONE)], &mut show);
    frames.frame(vec![key_release(Key::F1, Modifiers::NONE)], &mut show);
    assert_eq!(take_output(&frames.ctx).fired.len(), 1, "the command fired");
    assert!(keys(left.take()).is_empty(), "its press and release are the command's");

    frames.frame(vec![key_press(Key::F4, Modifiers::NONE)], &mut show);
    frames.frame(vec![key_release(Key::F4, Modifiers::NONE)], &mut show);
    assert_eq!(keys(left.take()), [(Key::F4, true), (Key::F4, false)], "an unbound key's are left over");
}

/// With nothing focused and nothing bound, both pastes of a pass are the host's: the second one reaches it as the first
/// does, though its chord is the shortcut the first one made.
#[trial]
fn leftover_events_holds_both_pastes_of_a_pass() {
    let mut frames = Frames::new();
    // The library's own Paste is bound to Cmd+V by default, and a paste with nothing focused stands for that chord: the
    // trial unbinds it, so no command takes the pastes, as in a window that runs no Paste of its own.
    let mut unbound = Overrides::new();
    unbound.insert(EditCommands::Paste.id(), Vec::new());
    frames.commands(CommandInput::new().overrides(Arc::new(unbound)));
    let left = RefCell::new(Vec::new());
    let mut show = |ui: &mut Ui| {
        left.borrow_mut().extend(leftover_events(ui.ctx()));
    };
    frames.idle(2, &mut show);
    take_output(&frames.ctx);
    left.borrow_mut().clear();

    frames.frame(vec![Event::Paste("a".to_owned()), Event::Paste("b".to_owned())], &mut show);
    assert_eq!(pasted(left.take()), ["a".to_owned(), "b".to_owned()], "both pastes are left over, in the order they arrived");
}

/// A fresh press of a key a command fired for ends that command's hold where the press is: the pass's events before it,
/// the key's repeat and its release, stay the command's, and the press and its release are the host's.
#[trial]
fn a_fresh_press_ends_a_fired_key_hold_after_the_events_before_it() {
    let mut frames = Frames::new();
    let binding = Binding::parse("F1").expect("the trial's chord parses");
    let help = CommandDef::new("leftoverTrial.hold", "Help", Scope::app()).with_default(binding);
    frames.commands(CommandInput::new().catalog(AppCatalog::new(vec![help], 1).expect("the trial's catalog is valid")));
    let left = RefCell::new(Vec::new());
    let mut show = |ui: &mut Ui| {
        left.borrow_mut().extend(leftover_events(ui.ctx()));
    };
    frames.idle(2, &mut show);
    take_output(&frames.ctx);
    left.borrow_mut().clear();

    frames.frame(vec![key_press(Key::F1, Modifiers::NONE)], &mut show);
    assert_eq!(take_output(&frames.ctx).fired.len(), 1, "the command fired for the press");
    assert!(key_events(left.take()).is_empty(), "the press is the command's");

    let held = vec![
        key_repeat(Key::F1, Modifiers::NONE),
        key_release(Key::F1, Modifiers::NONE),
        key_press(Key::F1, Modifiers::SHIFT),
        key_release(Key::F1, Modifiers::SHIFT),
    ];
    frames.frame(held, &mut show);
    assert_eq!(
        key_events(left.take()),
        [(Key::F1, true), (Key::F1, false)],
        "the repeat and release before the fresh press stay the command's, and the fresh press and its release are left over"
    );
}

/// A fresh press of a key a command fired for ends that command's hold in a pass that leaves no clip: egui reads the
/// press as a repeat, as it does for a key whose release went to another window, and the press is the host's.
#[trial]
fn a_fresh_press_ends_a_fired_key_hold_in_a_pass_with_no_clip() {
    let mut frames = Frames::new();
    let binding = Binding::parse("F1").expect("the trial's chord parses");
    let help = CommandDef::new("leftoverTrial.holdNoClip", "Help", Scope::app()).with_default(binding);
    frames.commands(CommandInput::new().catalog(AppCatalog::new(vec![help], 1).expect("the trial's catalog is valid")));
    let left = RefCell::new(Vec::new());
    let mut show = |ui: &mut Ui| {
        left.borrow_mut().extend(leftover_events(ui.ctx()));
    };
    frames.idle(2, &mut show);
    take_output(&frames.ctx);
    left.borrow_mut().clear();

    frames.frame(vec![key_press(Key::F1, Modifiers::NONE)], &mut show);
    assert_eq!(take_output(&frames.ctx).fired.len(), 1, "the command fired for the press");
    assert!(key_events(left.take()).is_empty(), "the press is the command's");

    // No release of F1 comes between the passes, so the viewport still has F1 down and egui reads the second press as a
    // repeat of the first, as it reads a press whose key's release went to another window.
    frames.frame(vec![text("y"), key_press(Key::F1, Modifiers::SHIFT)], &mut show);
    assert!(take_output(&frames.ctx).fired.is_empty(), "the fresh press's chord is bound to nothing");
    assert_eq!(key_events(left.take()), [(Key::F1, true)], "the hold ends at the fresh press, which is left over");
}

/// An event a widget took from egui's input does not pull a clip too late: the events the input still holds take their
/// own places among the pass's, equal ones apart.
#[trial]
fn a_clip_keeps_its_place_beside_equal_events_a_widget_took() {
    let mut frames = Frames::new();
    // The trial is about where the paste is put back, not who takes it: the library's own Paste, which binds Cmd+V by
    // default, is unbound so it takes neither the paste nor its place.
    let mut unbound = Overrides::new();
    unbound.insert(EditCommands::Paste.id(), Vec::new());
    frames.commands(CommandInput::new().overrides(Arc::new(unbound)));
    let left = RefCell::new(Vec::new());
    // A widget of the app's own takes the first of the two texts in the one frame below by hand, as one that reads
    // the input in order takes the first match it meets (egui's own consumers take every equal event).
    let take = Cell::new(false);
    let mut show = |ui: &mut Ui| {
        if take.replace(false) {
            ui.ctx().input_mut(|input| {
                let mut taken = false;
                input.events.retain(|event| {
                    let mine = !taken && matches!(event, Event::Text(typed) if typed == "x");
                    taken |= mine;
                    !mine
                });
            });
        }
        left.borrow_mut().extend(leftover_events(ui.ctx()));
    };
    frames.idle(2, &mut show);
    take_output(&frames.ctx);
    left.borrow_mut().clear();

    take.set(true);
    frames.frame(vec![text("x"), Event::Paste("p".to_owned()), text("x")], &mut show);
    let events = left.take();
    assert!(
        matches!(events.as_slice(), [Event::Paste(pasted), Event::Text(typed)] if pasted == "p" && typed == "x"),
        "the paste stays before the text that followed it: {events:?}"
    );
}

/// A key whose only binding is a hold of a disabled command runs nothing on a quick press and release: the press and its
/// release are the host's, and the app still hears of the disabled hold the dispatcher passed over.
#[trial]
fn a_disabled_hold_leaves_the_press_and_its_release_to_the_host() {
    const HELD: &str = "leftoverTrial.disabledHold";
    let mut frames = Frames::new();
    let chord = Binding::parse("X").expect("the trial's chord parses").trigger.chord;
    let held = CommandDef::new(HELD, "Hold", Scope::app()).with_default(Binding::new(Trigger::hold(chord)));
    let mut states = Tracked::new(CommandStates::new());
    states.set(CommandRef::new(HELD), CommandState::new().enabled(false));
    let input = CommandInput::new().catalog(AppCatalog::new(vec![held], 1).expect("the trial's catalog is valid")).states(&states).holds(Some(0.5));
    frames.commands(input);
    let left = RefCell::new(Vec::new());
    let mut show = |ui: &mut Ui| {
        left.borrow_mut().extend(leftover_events(ui.ctx()));
    };
    frames.idle(2, &mut show);
    take_output(&frames.ctx);
    left.borrow_mut().clear();

    frames.frame(vec![key_press(Key::X, Modifiers::NONE)], &mut show);
    frames.frame(vec![key_release(Key::X, Modifiers::NONE)], &mut show);
    let output = take_output(&frames.ctx);
    assert!(output.fired.is_empty(), "the disabled hold fires nothing: {:?}", output.fired);
    assert_eq!(
        output.declined.iter().map(|declined| declined.command.id.as_str()).collect::<Vec<_>>(),
        [HELD],
        "the app hears of the disabled hold it passed over, as a disabled menu row's hint tells it"
    );
    assert_eq!(key_events(left.take()), [(Key::X, true), (Key::X, false)], "the press and its release are left over to the host");
}

/// Cmd+V pressed on `physical` and typed as V, with the platform's paste in the same pass: whether the library's Paste,
/// which binds Cmd+V, fired for the press, and the pastes the host was left.
fn paste_pass(physical: Key) -> (bool, Vec<String>) {
    let mut frames = Frames::new();
    let left = RefCell::new(Vec::new());
    let mut show = |ui: &mut Ui| {
        left.borrow_mut().extend(leftover_events(ui.ctx()));
    };
    frames.idle(2, &mut show);
    take_output(&frames.ctx);
    left.borrow_mut().clear();
    let press = Event::Key { key: Key::V, physical_key: Some(physical), pressed: true, repeat: false, modifiers: Modifiers::COMMAND };
    frames.frame(vec![press, Event::Paste("p".to_owned())], &mut show);
    let fired = take_output(&frames.ctx).fired.into_iter().any(|fired| fired.command.id == EditCommands::Paste.id());
    (fired, pasted(left.take()))
}

/// A paste whose chord was pressed on a key that types another key is the command's: the clipboard event stands for its
/// chord by its typed key, so it is not left over beside the press that fired the Paste command.
#[trial]
fn a_paste_stands_for_a_press_that_fired_its_chord_on_another_physical_key() {
    // Cmd+V pressed on the key I, as a layout that types V there reports it.
    let (fired, left) = paste_pass(Key::I);
    assert!(fired, "the Paste command fired for the press");
    assert!(left.is_empty(), "the paste is the command's, not the host's, which would paste it twice: {left:?}");
}

/// A paste whose chord was pressed on the key that types its letter is the command's, as it was before: a clipboard event
/// stands for its own press by the key both carry.
#[trial]
fn a_paste_stands_for_its_own_press() {
    // Cmd+V pressed on the key V, as a QWERTY layout sends it.
    let (fired, left) = paste_pass(Key::V);
    assert!(fired, "the Paste command fired for the press");
    assert!(left.is_empty(), "the paste stands for its own press: {left:?}");
}

/// A paste holds the key its chord was pressed on, not the letter it types: the release of a key the host holds from an
/// earlier pass, coming in the paste's pass, is left over to the host, whose key would otherwise stay held.
#[trial]
fn a_paste_holds_the_key_its_chord_was_pressed_on() {
    let mut frames = Frames::new();
    let left = RefCell::new(Vec::new());
    let mut show = |ui: &mut Ui| {
        left.borrow_mut().extend(leftover_events(ui.ctx()));
    };
    frames.idle(2, &mut show);
    take_output(&frames.ctx);
    left.borrow_mut().clear();

    // The host holds V from a press no command took, whose release is still to come.
    frames.frame(vec![key_press(Key::V, Modifiers::NONE)], &mut show);
    assert!(take_output(&frames.ctx).fired.is_empty(), "a plain V runs no command");
    assert_eq!(key_events(left.take()), [(Key::V, true)], "the press is the host's, and the host holds V");

    // Cmd+V pressed on the key I, as a layout that types V there reports it, with the platform's paste in the same pass
    // and the releases of both keys after it.
    let press = Event::Key { key: Key::V, physical_key: Some(Key::I), pressed: true, repeat: false, modifiers: Modifiers::COMMAND };
    frames.frame(
        vec![press, Event::Paste("p".to_owned()), key_release(Key::I, Modifiers::COMMAND), key_release(Key::V, Modifiers::COMMAND)],
        &mut show,
    );
    assert!(take_output(&frames.ctx).fired.iter().any(|fired| fired.command.id == EditCommands::Paste.id()), "the Paste command fired for the press");
    assert_eq!(
        key_events(left.take()),
        [(Key::V, false)],
        "the paste holds the key its press was made on, I, so the release of the V the host holds is left over to it and the I release stays the command's"
    );
}
