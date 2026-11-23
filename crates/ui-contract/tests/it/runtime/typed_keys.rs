//! What the key stage makes of the events that carry text or a chord without a plain press of their own: a text with
//! no press, a press an input method typed through with Alt, a second press of a held key, and a clipboard event with
//! Shift held. Each trial binds the window's Toggle Panels command to the chord under test and
//! reads whether it ran.

use craft_behavior::{EditCommand, Input};
use craft_keymap::{Chord, Key as ChordKey, Mods, Overrides};
use craft_ui::runtime::commands::{ChromeCommand, CommandInput, CommandSet as _};
use craft_ui::runtime::focus::{FocusTarget, KeyUse, request};
use craft_ui::runtime::frame::take_output;
use craft_ui::testing::focus::{Stop, intents_of, key_release, named_id, register_stop, widget};
use craft_ui::testing::input::{key_press, text};
use craft_ui::testing::{Frames, trial};
use egui::{Event, ImeEvent, Key, Modifiers, Ui};
use std::cell::RefCell;
use std::sync::Arc;

/// What the window's text field read in a pass: the texts egui's input held for it and the edits routed to it.
#[derive(Default)]
struct Field {
    typed: Vec<String>,
    edits: Vec<EditCommand>,
}

/// Frames of a window with a document canvas and a text field, whose Toggle Panels command is bound to `chord` alone.
fn frames_for(chord: Chord) -> Frames {
    let mut overrides = Overrides::default();
    overrides.insert(ChromeCommand::TogglePanels.id(), vec![chord.to_string()]);
    let mut frames = Frames::new();
    frames.commands(CommandInput::new().overrides(Arc::new(overrides)));
    frames
}

/// The window's pass: the canvas and the field, which records what it read into `field`.
fn window(field: &RefCell<Field>) -> impl FnMut(&mut Ui) + '_ {
    move |ui: &mut Ui| {
        let (canvas, id) = (named_id("typed_canvas"), named_id("typed_field"));
        widget(ui, canvas);
        register_stop(ui.ctx(), Stop::new(canvas, KeyUse::CANVAS));
        widget(ui, id);
        register_stop(ui.ctx(), Stop::new(id, KeyUse::of(Input::Text)).text());
        let mut field = field.borrow_mut();
        ui.input(|input| {
            field.typed.extend(input.events.iter().filter_map(|event| match event {
                Event::Text(text) => Some(text.clone()),
                _ => None,
            }))
        });
        let mut intents = intents_of(ui.ctx(), Some(id), false);
        while let Some(edit) = intents.take_edit() {
            field.edits.push(edit);
        }
    }
}

/// Runs one frame of `events` and returns whether the bound command ran, clearing what the field read before it.
fn ran(frames: &mut Frames, show: &mut impl FnMut(&mut Ui), field: &RefCell<Field>, events: Vec<Event>) -> bool {
    *field.borrow_mut() = Field::default();
    frames.frame(events, &mut *show);
    !take_output(&frames.ctx).chrome_toggles.is_empty()
}

/// Settles the window with the field focused, or nothing focused.
fn settle(frames: &mut Frames, show: &mut impl FnMut(&mut Ui), focus_field: bool) {
    frames.idle(2, &mut *show);
    if focus_field {
        request(&frames.ctx, FocusTarget::Id(named_id("typed_field")));
        frames.idle(2, &mut *show);
    }
    take_output(&frames.ctx);
}

/// A press of a key that is down already is a repeat, whatever the platform says: it runs no shortcut again. Guards
/// against a key stage that read the repeat flag from the raw event only.
#[trial]
fn a_second_press_of_a_held_key_runs_no_shortcut_again() {
    let field = RefCell::new(Field::default());
    let mut frames = frames_for(Chord::plain(ChordKey::S));
    let mut show = window(&field);
    settle(&mut frames, &mut show, false);
    assert!(ran(&mut frames, &mut show, &field, vec![key_press(Key::S, Modifiers::NONE)]), "the first press runs the command");
    assert!(!ran(&mut frames, &mut show, &field, vec![key_press(Key::S, Modifiers::NONE)]), "a second press with S still down is a repeat");
    frames.frame(vec![key_release(Key::S, Modifiers::NONE)], &mut show);
    assert!(ran(&mut frames, &mut show, &field, vec![key_press(Key::S, Modifiers::NONE)]), "a press after the release runs it again");
}

/// A Cut with Cmd and Shift held is Cmd+Shift+X, the app's chord, not the focused field's Cut; a plain Cmd+X Cut stays
/// the field's. Guards against a key stage that gave every clipboard event to a field that lists the edit, whatever
/// the modifiers.
#[trial]
fn a_cut_with_shift_held_is_the_apps_chord() {
    let field = RefCell::new(Field::default());
    let mut frames = frames_for(Chord::new(Mods { shift: true, ..Mods::CMD }, ChordKey::X));
    let mut show = window(&field);
    settle(&mut frames, &mut show, true);
    let shifted = vec![Event::ModifiersChanged(Modifiers::COMMAND | Modifiers::SHIFT), Event::Cut];
    assert!(ran(&mut frames, &mut show, &field, shifted), "Cmd+Shift+X ran its command");
    assert!(field.borrow().edits.is_empty(), "the field cut nothing: {:?}", field.borrow().edits);
    assert!(!ran(&mut frames, &mut show, &field, vec![Event::ModifiersChanged(Modifiers::COMMAND), Event::Cut]), "Cmd+X runs no command");
    assert_eq!(field.borrow().edits, [EditCommand::Cut], "a plain Cut is the field's");
}
