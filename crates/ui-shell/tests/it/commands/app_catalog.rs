//! Commands an app defines at run time (`AppCatalog`): their chords fire them into the frame's output under the key
//! rules every app command follows, and a frame builds a keymap only for a catalog generation it has not built.
// The catalog is given through `Frames::commands`, which keeps it for every later frame.

use craft_ui::contract::{Bind, Props as _, Show as _};
use craft_ui::runtime::commands::{AppCatalog, Binding, CommandDef, CommandInput, CommandKind, FirePhase, Scope, keymap_builds};
use craft_ui::runtime::env::WindowKey;
use craft_ui::runtime::frame::take_output;
use craft_ui::shell::AppShell;
use craft_ui::testing::focus::key_release;
use craft_ui::testing::input::{key_press, text};
use craft_ui::testing::pointer::{moved, press, release};
use craft_ui::testing::{Frames, trial};
use craft_ui::text::TextField;
use egui::{Id, Key, Modifiers, Rect, Ui};
use std::cell::{Cell, RefCell};

const EXPORT: &str = "trialCatalog.export";
const PEEK: &str = "trialCatalog.peek";
const QUICK: &str = "trialCatalog.quick";
const RENAME: &str = "trialCatalog.rename";

/// An app-scope definition of `id` bound to `chord`.
fn def(id: &str, label: &str, chord: &str) -> CommandDef {
    CommandDef::new(id, label, Scope::app()).with_default(Binding::parse(chord).expect("the trial's chord parses"))
}

/// The app's catalog at `generation`: from generation 2 on it also holds Rename.
fn catalog(generation: u64) -> CommandInput {
    let mut defs = vec![def(EXPORT, "Export", "Cmd+Alt+U"), def(PEEK, "Peek", "F9").with_kind(CommandKind::Momentary), def(QUICK, "Quick Mask", "Q")];
    if generation >= 2 {
        defs.push(def(RENAME, "Rename", "Cmd+Alt+P"));
    }
    CommandInput::new().catalog(AppCatalog::new(defs, generation).expect("the trial's catalog is valid"))
}

/// The commands fired since the last call, as `(id, target, phase)`.
fn fired(frames: &Frames) -> Vec<(String, Option<Id>, FirePhase)> {
    take_output(&frames.ctx).fired.into_iter().map(|fired| (fired.command.id, fired.target, fired.phase)).collect()
}

fn hit(id: &str, phase: FirePhase) -> (String, Option<Id>, FirePhase) {
    (id.to_owned(), None, phase)
}

/// A catalog command's chord fires it as an app command, with no target; a momentary catalog command fires again on
/// its key's release; a plain-letter chord fires with nothing focused, and waits for the text while a text field has
/// the focus, which types the letter.
#[trial]
fn an_app_catalog_command_fires_by_its_chord_and_a_focused_text_field_keeps_a_typing_chord() {
    let name = RefCell::new(String::new());
    let field = Cell::new(Rect::NOTHING);
    let mut show = |ui: &mut Ui| {
        AppShell::new(WindowKey::main())
            .body(|ui, _, _| {
                field.set(TextField::new("Name", Bind::bound(&mut *name.borrow_mut())).id_salt("name").show(ui).response.rect);
            })
            .show(ui, &mut ());
    };
    let mut frames = Frames::new();
    frames.commands(catalog(1));
    frames.idle(3, &mut show);
    let chord = Modifiers::COMMAND | Modifiers::ALT;
    frames.frame(vec![key_press(Key::U, chord)], &mut show);
    frames.frame(vec![key_release(Key::U, chord)], &mut show);
    assert_eq!(fired(&frames), [hit(EXPORT, FirePhase::Press)], "the catalog command's chord fires it into the output");
    frames.frame(vec![key_press(Key::F9, Modifiers::NONE)], &mut show);
    frames.frame(vec![key_release(Key::F9, Modifiers::NONE)], &mut show);
    assert_eq!(
        fired(&frames),
        [hit(PEEK, FirePhase::Press), hit(PEEK, FirePhase::Release)],
        "a momentary catalog command fires on press and release"
    );
    frames.frame(vec![key_press(Key::Q, Modifiers::NONE)], &mut show);
    frames.frame(vec![key_release(Key::Q, Modifiers::NONE)], &mut show);
    assert_eq!(fired(&frames), [hit(QUICK, FirePhase::Press)], "with nothing focused the plain chord fires");

    let at = field.get().center();
    frames.frame(vec![moved(at)], &mut show);
    frames.frame(vec![press(at)], &mut show);
    frames.frame(vec![release(at)], &mut show);
    frames.frame(vec![], &mut show);
    let _ = fired(&frames);
    frames.frame(vec![key_press(Key::Q, Modifiers::NONE), text("q")], &mut show);
    frames.frame(vec![key_release(Key::Q, Modifiers::NONE)], &mut show);
    assert!(fired(&frames).is_empty(), "the plain chord waits while the text field has the focus");
    assert_eq!(*name.borrow(), "q", "the focused field types the letter");
}

/// Count relation: frames given a catalog of the generation already built, even as a new value, build no keymap; a new
/// generation builds one, in its first frame, and its new command fires by its chord.
#[trial]
fn an_unchanged_catalog_builds_no_keymap() {
    let mut show = |ui: &mut Ui| {
        AppShell::new(WindowKey::main()).show(ui, &mut ());
    };
    let mut frames = Frames::new();
    frames.commands(catalog(1));
    frames.idle(2, &mut show);
    let before = keymap_builds();
    for _ in 0..4 {
        frames.commands(catalog(1));
        frames.frame(vec![], &mut show);
    }
    assert_eq!(keymap_builds() - before, 0, "a catalog of the built generation builds no keymap");
    frames.commands(catalog(2));
    let before = keymap_builds();
    frames.idle(3, &mut show);
    assert_eq!(keymap_builds() - before, 1, "a new generation builds one keymap");
    let chord = Modifiers::COMMAND | Modifiers::ALT;
    frames.frame(vec![key_press(Key::P, chord)], &mut show);
    frames.frame(vec![key_release(Key::P, chord)], &mut show);
    assert_eq!(fired(&frames), [hit(RENAME, FirePhase::Press)], "the new generation's command fires by its chord");
}
