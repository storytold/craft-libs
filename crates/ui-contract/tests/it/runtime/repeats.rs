//! A held key's repeats (`RootEnv::shortcuts_repeat`): an app that turns them on runs a bound command again on each
//! repeat, as a held Cmd+= keeps zooming in; a momentary command's repeats leave its key held, so the release ends it.

use craft_ui::runtime::commands::{AppCatalog, Binding, CommandDef, CommandInput, CommandKind, FirePhase, Scope};
use craft_ui::runtime::env::RootEnv;
use craft_ui::runtime::frame::take_output;
use craft_ui::testing::focus::key_release;
use craft_ui::testing::input::{key_press, key_repeat};
use craft_ui::testing::{Frames, trial};
use egui::{Key, Modifiers, Ui};

const ZOOM: &str = "repeatTrial.zoom";
const PAN: &str = "repeatTrial.pan";

/// The phases `key` fired its command in over a press, two repeats and the release, with nothing focused, under
/// `shortcuts_repeat` set to `repeats`.
fn fired_over_a_hold(key: Key, repeats: bool) -> Vec<FirePhase> {
    let defs = [(ZOOM, "F9", CommandKind::Command), (PAN, "F8", CommandKind::Momentary)].map(|(id, chord, kind)| {
        let binding = Binding::parse(chord).expect("the trial's chord parses");
        CommandDef::new(id, id, Scope::app()).with_default(binding).with_kind(kind)
    });
    let catalog = AppCatalog::new(defs.to_vec(), 1).expect("the trial's catalog is taken");
    let mut frames = Frames::new();
    let mut root = RootEnv::new();
    root.shortcuts_repeat = repeats;
    frames.root(root);
    frames.commands(CommandInput::new().catalog(catalog));
    let mut show = |_: &mut Ui| {};
    frames.idle(2, &mut show);
    take_output(&frames.ctx);
    let mut phases = Vec::new();
    for event in
        [key_press(key, Modifiers::NONE), key_repeat(key, Modifiers::NONE), key_repeat(key, Modifiers::NONE), key_release(key, Modifiers::NONE)]
    {
        frames.frame(vec![event], &mut show);
        frames.idle(1, &mut show);
        phases.extend(take_output(&frames.ctx).fired.into_iter().map(|fired| fired.phase));
    }
    phases
}

#[trial]
fn a_held_keys_repeats_run_its_command_again_only_when_the_app_says_so() {
    assert_eq!(fired_over_a_hold(Key::F9, false), vec![FirePhase::Press], "by default the press runs the command once");
    assert_eq!(
        fired_over_a_hold(Key::F9, true),
        vec![FirePhase::Press, FirePhase::Repeat, FirePhase::Repeat],
        "with repeats on, each repeat runs it again"
    );
    assert_eq!(
        fired_over_a_hold(Key::F8, true),
        vec![FirePhase::Press, FirePhase::Repeat, FirePhase::Repeat, FirePhase::Release],
        "a momentary command repeats and still ends at the release"
    );
}
