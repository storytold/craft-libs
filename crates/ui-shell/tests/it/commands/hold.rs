//! The hold wait: a key with a single and a hold binding runs one or the other by how long it stayed down.

use craft_ui::runtime::commands::{
    AppCatalog, Binding, CommandDef, CommandGate, CommandInput, CommandKind, CommandRef, FirePhase, Scope, Trigger, keymap_builds,
};
use craft_ui::runtime::env::WindowKey;
use craft_ui::runtime::frame::take_output;
use craft_ui::shell::AppShell;
use craft_ui::testing::focus::key_release;
use craft_ui::testing::input::key_press;
use craft_ui::testing::{Frames, trial};
use egui::{Key, Modifiers, Ui};

const TAP: &str = "trialHold.tap";
const HOLD: &str = "trialHold.hold";
const OTHER: &str = "trialHold.other";
const MOMENT: &str = "trialHold.moment";
const DELAY: f64 = 0.5;

fn def(id: &str, chord: &str) -> CommandDef {
    CommandDef::new(id, id, Scope::app()).with_default(Binding::parse(chord).expect("the trial's chord parses"))
}

/// The hold binding of `chord`.
fn hold_def(id: &str, chord: &str) -> CommandDef {
    let chord = Binding::parse(chord).expect("the trial's chord parses").trigger.chord;
    CommandDef::new(id, id, Scope::app()).with_default(Binding::new(Trigger::hold(chord)))
}

/// X taps `TAP` and holds `HOLD`; Y runs `OTHER`; M is a momentary command.
fn input(delay: Option<f64>) -> CommandInput {
    let defs = vec![def(TAP, "X"), hold_def(HOLD, "X"), def(OTHER, "Y"), def(MOMENT, "M").with_kind(CommandKind::Momentary)];
    CommandInput::new().catalog(AppCatalog::new(defs, 1).expect("the trial's catalog is valid")).holds(delay)
}

fn show(ui: &mut Ui) {
    AppShell::new(WindowKey::main()).show(ui, &mut ());
}

/// A frame source with a still clock, the catalog in and a few frames run.
fn started(input: CommandInput) -> Frames {
    let mut frames = Frames::new();
    frames.still_clock();
    frames.commands(input);
    frames.idle(3, show);
    frames
}

fn fired(frames: &Frames) -> Vec<(String, FirePhase)> {
    take_output(&frames.ctx).fired.into_iter().map(|fired| (fired.command.id, fired.phase)).collect()
}

fn pressed(id: &str) -> (String, FirePhase) {
    (id.to_owned(), FirePhase::Press)
}

fn down(frames: &mut Frames, key: Key) {
    frames.frame(vec![key_press(key, Modifiers::NONE)], show);
}

fn up(frames: &mut Frames, key: Key) {
    frames.frame(vec![key_release(key, Modifiers::NONE)], show);
}

/// A release before the delay runs the tap, a release after it the hold; a key still down at the deadline runs the hold
/// once, and its later release runs nothing.
#[trial]
fn a_tap_runs_the_key_and_a_hold_runs_the_hold_once() {
    let mut frames = started(input(Some(DELAY)));
    down(&mut frames, Key::X);
    assert!(fired(&frames).is_empty(), "the press waits");
    frames.advance(0.1);
    up(&mut frames, Key::X);
    assert_eq!(fired(&frames), [pressed(TAP)], "a release before the delay is a tap");

    down(&mut frames, Key::X);
    frames.advance(DELAY + 0.2);
    up(&mut frames, Key::X);
    assert_eq!(fired(&frames), [pressed(HOLD)], "a release after the delay is a hold");

    down(&mut frames, Key::X);
    frames.advance(DELAY + 0.2);
    frames.frame(vec![], show);
    assert_eq!(fired(&frames), [pressed(HOLD)], "the deadline fires the hold with the key still down");
    frames.frame(vec![], show);
    up(&mut frames, Key::X);
    assert!(fired(&frames).is_empty(), "the hold fired once");
}

/// A second key pressed while a wait runs settles the wait first: the tap fires before the second key's command.
#[trial]
fn keys_act_in_the_order_they_went_down() {
    let mut frames = started(input(Some(DELAY)));
    down(&mut frames, Key::X);
    frames.advance(0.1);
    down(&mut frames, Key::Y);
    assert_eq!(fired(&frames), [pressed(TAP), pressed(OTHER)]);
    up(&mut frames, Key::X);
    assert!(fired(&frames).is_empty(), "the settled key's release runs nothing");
}

/// A text field that has the keys ends the wait: the press starts none and the release runs neither the tap nor the hold.
#[trial]
fn a_modal_typing_or_a_claim_ends_the_wait_without_firing() {
    use craft_ui::contract::{Bind, Props as _, Show as _};
    use craft_ui::testing::pointer::{moved, press, release};
    use craft_ui::text::TextField;
    use std::cell::{Cell, RefCell};

    let name = RefCell::new(String::new());
    let field = Cell::new(egui::Rect::NOTHING);
    let mut with_field = |ui: &mut Ui| {
        AppShell::new(WindowKey::main())
            .body(|ui, _, _| {
                field.set(TextField::new("Name", Bind::bound(&mut *name.borrow_mut())).id_salt("name").show(ui).response.rect);
            })
            .show(ui, &mut ());
    };
    let mut frames = Frames::new();
    frames.still_clock();
    frames.commands(input(Some(DELAY)));
    frames.idle(3, &mut with_field);
    let at = field.get().center();
    frames.frame(vec![moved(at)], &mut with_field);
    frames.frame(vec![press(at)], &mut with_field);
    frames.frame(vec![release(at)], &mut with_field);
    frames.idle(2, &mut with_field);
    let _ = fired(&frames);
    frames.frame(vec![key_press(Key::X, Modifiers::NONE)], &mut with_field);
    frames.advance(DELAY + 0.2);
    frames.frame(vec![], &mut with_field);
    frames.frame(vec![key_release(Key::X, Modifiers::NONE)], &mut with_field);
    assert!(fired(&frames).is_empty(), "a focused text field has the key: no tap and no hold");
}

/// Losing the window's focus drops the wait and ends a held momentary command once; the key's later release runs nothing.
#[trial]
fn focus_loss_drops_the_wait() {
    let mut frames = started(input(Some(DELAY)));
    down(&mut frames, Key::M);
    assert_eq!(fired(&frames), [pressed(MOMENT)]);
    down(&mut frames, Key::X);
    frames.frame(vec![egui::Event::WindowFocused(false)], show);
    assert_eq!(fired(&frames), [(MOMENT.to_owned(), FirePhase::Release)], "the momentary command ends at the focus loss");
    frames.advance(DELAY + 0.2);
    up(&mut frames, Key::X);
    up(&mut frames, Key::M);
    assert!(fired(&frames).is_empty(), "neither the tap, the hold nor a second release runs");
}

/// A wait keeps the delay it started with; the next press reads the new one.
#[trial]
fn the_delay_is_read_at_the_press() {
    let mut frames = started(input(Some(DELAY)));
    down(&mut frames, Key::X);
    frames.commands(input(Some(5.0)));
    frames.advance(DELAY + 0.2);
    frames.frame(vec![], show);
    assert_eq!(fired(&frames), [pressed(HOLD)], "the running wait keeps its deadline");
    up(&mut frames, Key::X);
    let _ = fired(&frames);
    down(&mut frames, Key::X);
    frames.advance(DELAY + 0.2);
    up(&mut frames, Key::X);
    assert_eq!(fired(&frames), [pressed(TAP)], "the next press waits the new delay");
}

/// With holds off the hold binding is ignored and the single binding fires on the press.
#[trial]
fn holds_off_fire_on_the_press() {
    let mut frames = started(input(None));
    down(&mut frames, Key::X);
    assert_eq!(fired(&frames), [pressed(TAP)], "no wait: the press fires");
    frames.advance(DELAY + 0.2);
    up(&mut frames, Key::X);
    assert!(fired(&frames).is_empty());
}

/// Turning holds off while a wait runs ends it without firing, and builds the keymap once.
#[trial]
fn turning_holds_off_ends_a_running_wait() {
    let mut frames = started(input(Some(DELAY)));
    down(&mut frames, Key::X);
    let before = keymap_builds();
    frames.commands(input(None));
    frames.advance(DELAY + 0.2);
    frames.frame(vec![], show);
    up(&mut frames, Key::X);
    assert!(fired(&frames).is_empty(), "no tap and no hold");
    assert_eq!(keymap_builds() - before, 1, "the keymap is rebuilt once");
}

/// A gate that refuses the commands of the key leaves the output empty, as for a plain press.
#[trial]
fn a_wait_on_a_key_whose_command_a_gate_refuses_fires_nothing() {
    let gate = CommandGate::new(true, |refuses: &bool, command: &CommandRef| !*refuses || ![TAP, HOLD].contains(&command.id.as_str()));
    let mut frames = started(input(Some(DELAY)).gate(gate));
    down(&mut frames, Key::X);
    frames.advance(DELAY + 0.2);
    up(&mut frames, Key::X);
    assert!(fired(&frames).is_empty(), "the refused hold fires nothing");
    down(&mut frames, Key::X);
    frames.advance(0.1);
    up(&mut frames, Key::X);
    assert!(fired(&frames).is_empty(), "the refused tap fires nothing");
}
