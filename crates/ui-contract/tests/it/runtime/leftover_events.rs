//! `runtime::keys::leftover_events`: the keys of a pass that nothing took, for a host that runs shortcuts of its own on
//! a window's input.

use craft_ui::runtime::commands::{AppCatalog, Binding, CommandDef, CommandInput, Scope};
use craft_ui::runtime::frame::take_output;
use craft_ui::runtime::keys::leftover_events;
use craft_ui::testing::focus::key_release;
use craft_ui::testing::input::{key_press, key_repeat, text};
use craft_ui::testing::{Frames, trial};
use egui::{Event, Key, Modifiers, Ui};
use std::cell::{Cell, RefCell};

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

/// An event a widget took from egui's input does not pull a clip too late: the events the input still holds take their
/// own places among the pass's, equal ones apart.
#[trial]
fn a_clip_keeps_its_place_beside_equal_events_a_widget_took() {
    let mut frames = Frames::new();
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
