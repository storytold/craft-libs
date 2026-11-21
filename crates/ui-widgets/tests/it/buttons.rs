//! Buttons: the press a button reports as held, and the chord a command button shows.

use craft_behavior::IconId;
use craft_ui::buttons::{Button, ButtonChord, IconButton};
use craft_ui::contract::{Props as _, Show as _};
use craft_ui::primitives::IconSpec;
use craft_ui::runtime::commands::{AppCatalog, Binding, CommandDef, CommandInput, CommandRef, Scope};
use craft_ui::testing::input::key_press;
use craft_ui::testing::layout::in_region;
use craft_ui::testing::pointer::{moved, press, release, window_blurred};
use craft_ui::testing::{Frames, trial};
use egui::{Event, Key, Modifiers, Pos2, Rect, Ui, pos2, vec2};
use std::cell::Cell;

/// Runs an icon button through idle frames, then one frame per step of `steps` (given the button's centre and a point
/// far off it), and returns `ButtonOutput::held` of each of those frames.
fn held_per_frame(steps: impl FnOnce(Pos2, Pos2) -> Vec<Vec<Event>>) -> Vec<bool> {
    let (held, rect) = (Cell::new(false), Cell::new(Rect::NOTHING));
    let mut show = |ui: &mut Ui| {
        let out = IconButton::new(IconSpec::svg(IconId::new("eyedropper"))).label("Hold").show(ui);
        held.set(out.held);
        rect.set(out.rect);
    };
    let mut frames = Frames::new();
    frames.idle(3, &mut show);
    let at = rect.get().center();
    let far = at + vec2(300.0, 300.0);
    steps(at, far)
        .into_iter()
        .map(|events| {
            frames.frame(events, &mut show);
            held.get()
        })
        .collect()
}

#[trial]
fn a_held_button_reports_its_press_until_the_release_or_a_cancel() {
    assert_eq!(held_per_frame(|at, _| vec![vec![], vec![moved(at)]]), [false, false], "an idle button is not held");
    assert_eq!(held_per_frame(|at, _| vec![vec![moved(at)], vec![press(at)]]), [false, true], "a press holds it");
    assert_eq!(
        held_per_frame(|at, far| vec![vec![moved(at)], vec![press(at)], vec![moved(far)]]),
        [false, true, true],
        "the pointer leaving the button does not end the press"
    );
    assert_eq!(
        held_per_frame(|at, far| vec![vec![moved(at)], vec![press(at)], vec![moved(far)], vec![release(far)]]),
        [false, true, true, false],
        "the release ends it"
    );
    assert_eq!(
        held_per_frame(|at, _| vec![vec![moved(at)], vec![press(at)], vec![key_press(Key::Escape, Modifiers::NONE)]]),
        [false, true, false],
        "Escape cancels it"
    );
    assert_eq!(
        held_per_frame(|at, _| vec![vec![moved(at)], vec![press(at)], vec![window_blurred()]]),
        [false, true, false],
        "losing the window's focus cancels it"
    );
    let by_key = held_per_frame(|_, _| vec![vec![key_press(Key::Tab, Modifiers::NONE)], vec![key_press(Key::Space, Modifiers::NONE)], vec![]]);
    assert_eq!(by_key, [false, false, false], "a key activation never holds the button");
}

/// The catalog of the chord trial: one command with a default chord, one without.
fn catalog() -> AppCatalog {
    let bound =
        CommandDef::new(CommandRef::new("trial.bound"), "Bound", Scope::app()).with_default(Binding::parse("F7").expect("the trial's chord parses"));
    let plain = CommandDef::new(CommandRef::new("trial.plain"), "Plain", Scope::app());
    AppCatalog::new(vec![bound, plain], 1).expect("the trial's catalog is valid")
}

/// A command button of `command` with `chord` in a room `width` wide: the texts it painted, joined in paint order and
/// split to (text, elided), and the button's width.
fn shown(command: &str, chord: ButtonChord, width: f32) -> (Vec<(String, bool)>, f32) {
    let button_width = Cell::new(0.0);
    let mut show = |ui: &mut Ui| {
        let room = Rect::from_min_size(pos2(0.0, 0.0), vec2(width, 40.0));
        button_width.set(in_region(ui, room, |ui| Button::command(CommandRef::new(command)).chord(chord).show(ui).rect.width()));
    };
    let mut frames = Frames::new();
    frames.commands(CommandInput::new().catalog(catalog()));
    frames.idle(3, &mut show);
    (frames.texts().iter().map(|text| (text.text.clone(), text.elided)).collect(), button_width.get())
}

#[trial]
fn a_command_button_shows_its_chord_until_the_room_is_short() {
    // PLAN: assumes a command button hugs its content in the room, so the width of the chordless button is a room that
    // fits the label and not the chord.
    let (wide, _) = shown("trial.bound", ButtonChord::Trailing, 400.0);
    let all: String = wide.iter().map(|(text, _)| text.as_str()).collect();
    let (label, chord) = (all.find("Bound"), all.find("F7"));
    assert!(label.is_some() && chord.is_some(), "the label and the chord show in a wide room: {wide:?}");
    assert!(label < chord, "the chord follows the label: {all:?}");

    let (_, label_width) = shown("trial.bound", ButtonChord::Hidden, 400.0);
    let (short, _) = shown("trial.bound", ButtonChord::Trailing, label_width);
    assert!(short.iter().all(|(text, _)| !text.contains("F7")), "a short room drops the chord: {short:?}");
    assert!(short.iter().any(|(text, elided)| text.contains("Bound") && !elided), "before it elides the label: {short:?}");

    let (plain_chord, _) = shown("trial.plain", ButtonChord::Trailing, 400.0);
    let (plain_hidden, _) = shown("trial.plain", ButtonChord::Hidden, 400.0);
    assert_eq!(plain_chord, plain_hidden, "a command with no chord shows none");
}
