//! Commands an app defines at run time (`AppCatalog`): their chords fire them into the frame's output under the key
//! rules every app command follows, and a frame builds a keymap only for a catalog generation it has not built.
// The catalog is given through `Frames::commands`, which keeps it for every later frame.

use craft_ui::contract::{Bind, Props as _, Show as _, Tracked};
use craft_ui::runtime::commands::{
    AppCatalog, Binding, CommandDef, CommandInput, CommandKind, CommandRef, CommandState, CommandStates, EditCommands, FirePhase, Scope,
    keymap_builds,
};
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
const BRUSH: &str = "trialCatalog.brush";
const RENAME: &str = "trialCatalog.rename";

/// An app-scope definition of `id` bound to `chord`.
fn def(id: &str, label: &str, chord: &str) -> CommandDef {
    CommandDef::new(id, label, Scope::app()).with_default(Binding::parse(chord).expect("the trial's chord parses"))
}

/// The app's catalog at `generation`: from generation 2 on it also holds Rename.
fn catalog(generation: u64) -> CommandInput {
    let mut defs = vec![
        def(EXPORT, "Export", "Cmd+Alt+U"),
        def(PEEK, "Peek", "F9").with_kind(CommandKind::Momentary),
        def(QUICK, "Quick Mask", "Q"),
        def(BRUSH, "Brush", "J").with_kind(CommandKind::Tool { group: None }),
    ];
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

/// A catalog command's chord fires it as an app command, with no target; a momentary or tool catalog command fires
/// again on its key's release; a plain-letter chord fires with nothing focused, and waits for the text while a text field has
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
    frames.frame(vec![key_press(Key::J, Modifiers::NONE)], &mut show);
    frames.frame(vec![key_release(Key::J, Modifiers::NONE)], &mut show);
    assert_eq!(fired(&frames), [hit(BRUSH, FirePhase::Press), hit(BRUSH, FirePhase::Release)], "a tool's key fires on press and release");

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

/// A key's press and its release fire with the input times they arrived at, whichever frame runs them: a hold is
/// read from the key's own times. Keys behind a pointer press wait for the next frame (one press per pass) and keep
/// the time they arrived at.
#[trial]
fn a_keys_press_and_release_fire_with_the_times_they_arrived_at() {
    let mut show = |ui: &mut Ui| {
        AppShell::new(WindowKey::main()).show(ui, &mut ());
    };
    let mut frames = Frames::new();
    frames.commands(catalog(1));
    frames.idle(3, &mut show);
    let times =
        |frames: &Frames| take_output(&frames.ctx).fired.into_iter().map(|fired| (fired.command.id, fired.phase, fired.time)).collect::<Vec<_>>();
    let now = |frames: &Frames| frames.ctx.input(|input| input.time);
    frames.frame(vec![key_press(Key::J, Modifiers::NONE)], &mut show);
    let pressed = now(&frames);
    frames.frame(vec![key_release(Key::J, Modifiers::NONE)], &mut show);
    let released = now(&frames);
    assert!(pressed < released);
    assert_eq!(times(&frames), [(BRUSH.to_owned(), FirePhase::Press, pressed), (BRUSH.to_owned(), FirePhase::Release, released)]);

    let at = egui::pos2(400.0, 300.0);
    frames.frame(vec![moved(at)], &mut show);
    let _ = times(&frames);
    frames.frame(vec![press(at), key_press(Key::J, Modifiers::NONE), key_release(Key::J, Modifiers::NONE)], &mut show);
    let arrived = now(&frames);
    assert!(times(&frames).is_empty(), "the keys after the pointer press wait for the next frame");
    frames.frame(vec![release(at)], &mut show);
    assert!(now(&frames) > arrived);
    assert_eq!(
        times(&frames),
        [(BRUSH.to_owned(), FirePhase::Press, arrived), (BRUSH.to_owned(), FirePhase::Release, arrived)],
        "the held keys fire with the time they arrived at"
    );
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

/// A key event with no physical key, as egui_kittest and some platforms send it.
fn logical(key: Key, pressed: bool, modifiers: Modifiers) -> egui::Event {
    egui::Event::Key { key, physical_key: None, pressed, repeat: false, modifiers }
}

/// A press with no physical key runs its command by the logical key, once, whether its release comes in a later pass or
/// in the same one (a quick tap during a slow frame), for a plain letter and a Cmd chord alike.
#[trial]
fn a_press_without_a_physical_key_fires_its_command_once() {
    let mut show = |ui: &mut Ui| {
        AppShell::new(WindowKey::main()).show(ui, &mut ());
    };
    let mut frames = Frames::new();
    frames.commands(catalog(1));
    frames.idle(3, &mut show);
    let chord = Modifiers::COMMAND | Modifiers::ALT;
    for (key, mods, id) in [(Key::Q, Modifiers::NONE, QUICK), (Key::U, chord, EXPORT)] {
        frames.frame(vec![logical(key, true, mods)], &mut show);
        frames.frame(vec![logical(key, false, mods)], &mut show);
        assert_eq!(fired(&frames), [hit(id, FirePhase::Press)], "{id}: press and release on separate passes");
        let one_pass = vec![
            egui::Event::ModifiersChanged(mods),
            logical(key, true, mods),
            logical(key, false, mods),
            egui::Event::ModifiersChanged(Modifiers::NONE),
        ];
        frames.frame(one_pass, &mut show);
        frames.idle(2, &mut show);
        assert_eq!(fired(&frames), [hit(id, FirePhase::Press)], "{id}: press and release in one pass");
    }
}

/// An app catalog's def of a registry command adds its keys beside the registry's: F2 does what Cmd+X does (Photoshop's
/// second keys for Cut, Copy and Paste).
#[trial]
fn an_app_catalogs_alternate_key_for_a_registry_command_runs_it() {
    let cut = CommandRef::from(EditCommands::Cut);
    let alternate = CommandDef::new(cut.clone(), "Cut", Scope::app()).with_default(Binding::parse("F2").expect("the trial's chord parses"));
    let mut states = Tracked::new(CommandStates::new());
    states.set(cut.clone(), CommandState::new());
    let input = || CommandInput::new().catalog(AppCatalog::new(vec![alternate.clone()], 1).expect("the trial's catalog is valid")).states(&states);
    let mut show = |ui: &mut Ui| {
        AppShell::new(WindowKey::main()).show(ui, &mut ());
    };
    let mut frames = Frames::new();
    frames.commands(input());
    frames.idle(3, &mut show);
    frames.frame(vec![key_press(Key::X, Modifiers::COMMAND)], &mut show);
    frames.frame(vec![key_release(Key::X, Modifiers::COMMAND)], &mut show);
    let by_registry_key = fired(&frames);
    assert_eq!(by_registry_key, [(cut.id.clone(), None, FirePhase::Press)], "the registry's key runs Cut");
    frames.frame(vec![key_press(Key::F2, Modifiers::NONE)], &mut show);
    frames.frame(vec![key_release(Key::F2, Modifiers::NONE)], &mut show);
    assert_eq!(fired(&frames), by_registry_key, "the catalog's alternate key does what the registry's key does");
}

const PANEL: &str = "trialCatalog.panel";
const CLOSE: &str = "trialCatalog.close";

/// A def of the parameterised `PANEL` command with `params` (a JSON object text), bound to `chord`.
fn panel(params: &str, label: &str, chord: &str) -> CommandDef {
    let command = CommandRef::parse_key(&format!("{PANEL} {params}")).expect("the trial's key parses");
    CommandDef::new(command, label, Scope::app()).with_default(Binding::parse(chord).expect("the trial's chord parses"))
}

/// Two defs with one id and different params build, each with its own chord and label; the bare id's def answers a ref
/// with other params; two defs with one key are refused.
#[trial]
fn parameterised_catalog_rows_keep_their_own_kinds_and_defaults() {
    let layers = panel(r#"{"panel":"layers"}"#, "Layers", "Cmd+Alt+1");
    let info = panel(r#"{"panel":"info"}"#, "Info", "Cmd+Alt+2");
    let bare = def(PANEL, "Panel", "Cmd+Alt+3");
    assert!(AppCatalog::new(vec![layers.clone(), layers.clone()], 1).is_err(), "one key twice is a duplicate");
    let catalog = AppCatalog::new(vec![layers.clone(), info.clone(), bare], 1).expect("two params of one id are two rows");
    let mut show = |ui: &mut Ui| {
        AppShell::new(WindowKey::main()).show(ui, &mut ());
    };
    let mut frames = Frames::new();
    frames.commands(CommandInput::new().catalog(catalog));
    frames.idle(3, &mut show);
    for (key, def) in [(Key::Num1, &layers), (Key::Num2, &info), (Key::Num3, &CommandDef::new(PANEL, "Panel", Scope::app()))] {
        let chord = Modifiers::COMMAND | Modifiers::ALT;
        frames.frame(vec![key_press(key, chord)], &mut show);
        frames.frame(vec![key_release(key, chord)], &mut show);
        let got: Vec<String> = take_output(&frames.ctx).fired.into_iter().map(|fired| fired.command.key()).collect();
        assert_eq!(got, [def.command.key()], "the chord fires the row's own ref");
    }
    let label = |command: &CommandRef| craft_ui::runtime::commands::resolve(&frames.ctx, command.clone()).label.to_string();
    assert_eq!(label(&layers.command), "Layers");
    assert_eq!(label(&info.command), "Info");
    let unknown = CommandRef::parse_key(&format!(r#"{PANEL} {{"panel":"nope"}}"#)).expect("the trial's key parses");
    assert_eq!(label(&unknown), "Panel", "the bare id's def answers a ref with other params");
}

/// The runtime's keymap reserves the system chords: a catalog default on one is not bound, and the keymap answers what
/// `bindings` reads. It is `None` before the first frame.
#[trial]
fn the_runtime_keymap_reserves_the_system_chords() {
    let untouched = Frames::uninstalled();
    assert!(craft_ui::runtime::commands::keymap(&untouched.ctx).is_none(), "no keymap before the first frame");

    let close = CommandRef::new(CLOSE);
    let defs = vec![def(CLOSE, "Alt+F4", "Alt+F4"), def(QUICK, "Quick Mask", "Q")];
    let mut frames = Frames::new();
    craft_ui::testing::windows::set_os(&frames.ctx, craft_host::target::Os::Windows);
    frames.commands(CommandInput::new().catalog(AppCatalog::new(defs, 1).expect("the trial's catalog is valid")));
    let mut show = |ui: &mut Ui| {
        AppShell::new(WindowKey::main()).show(ui, &mut ());
    };
    frames.idle(3, &mut show);
    let keymap = craft_ui::runtime::commands::keymap(&frames.ctx).expect("a keymap after the first frame");
    assert!(keymap.bindings(&close).is_empty(), "Alt+F4 is reserved on Windows");
    assert_eq!(craft_ui::runtime::commands::bindings(&frames.ctx, &close), keymap.bindings(&close));
    assert!(!keymap.bindings(&CommandRef::new(QUICK)).is_empty(), "an ordinary chord stays bound");
    frames.frame(vec![key_press(Key::F4, Modifiers::ALT)], &mut show);
    frames.frame(vec![key_release(Key::F4, Modifiers::ALT)], &mut show);
    assert!(fired(&frames).is_empty(), "the reserved chord fires nothing");
}

/// A holder recorded in a pass is shown in the next, never in its own or before any pass.
#[trial]
fn a_command_shown_in_view_is_found_and_one_under_a_layer_is_not() {
    use craft_ui::runtime::commands::{holds, shown};
    let command = CommandRef::new(QUICK);
    let rect = Rect::from_min_size(egui::pos2(20.0, 20.0), egui::vec2(60.0, 20.0));
    let seen = RefCell::new(Vec::new());
    let mut show = |ui: &mut Ui| {
        seen.borrow_mut().push(shown(ui.ctx(), &command));
        holds(ui.ctx(), ui.layer_id(), rect, &std::sync::Arc::from([command.clone()]), "Quick".into());
    };
    let mut frames = Frames::new();
    frames.idle(3, &mut show);
    let seen = seen.into_inner();
    assert_eq!(seen[0], None, "before any pass");
    let last = seen.last().expect("passes ran").clone().expect("shown after a pass that recorded it");
    assert!(last.holder, "a holder's rect");
    assert_eq!(last.rect, rect);
}

/// A story's commands: only an install that shows stories loads them.
#[derive(Clone, Copy, Debug, PartialEq, Eq, craft_ui::contract::CommandSet)]
#[command(prefix = "trialStory", story)]
#[non_exhaustive]
enum StoryCommand {
    #[command(label = "Story Eraser", default = "Cmd+Alt+Shift+E")]
    Eraser,
}

/// A story's command set binds its keys in an install that shows stories (the story fixtures, the gallery) and is absent
/// from an app's install, so a story's keys never take an app's.
#[trial]
fn a_story_command_set_loads_only_in_an_install_that_shows_stories() {
    let eraser = CommandRef::from(StoryCommand::Eraser);
    let mut stories = Frames::new();
    stories.idle(2, &mut |_: &mut Ui| {});
    assert!(!craft_ui::runtime::commands::bindings(&stories.ctx, &eraser).is_empty(), "the story fixture loads the story's set");

    let mut app = Frames::uninstalled();
    let spec = craft_ui::InstallSpec::new(craft_host::target::ShellInfo::default(), std::sync::Arc::new(craft_ui::testing::windows::NoWindows));
    craft_ui::install(&app.ctx, spec).expect("the app's install");
    app.idle(2, &mut |_: &mut Ui| {});
    assert!(craft_ui::runtime::commands::bindings(&app.ctx, &eraser).is_empty(), "an app's install has no story's set");
    let save = CommandRef::new("trial.save");
    assert!(!craft_ui::runtime::commands::bindings(&app.ctx, &save).is_empty(), "the app's install keeps the other sets");
}
