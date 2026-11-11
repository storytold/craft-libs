//! Stories of the Keyboard Shortcuts dialog: one per state the dialog is required to show, over one keymap.

use craft_keymap::{
    Binding, Chord, CommandDef, ContextRule, Flavor, ForeignEntry, ForeignKey, ForeignSet, ImportOptions, Key, Keymap, Overrides, Platform, Profiles,
    Scope, Scopes,
};
use craft_ui::contract::Show as _;
use craft_ui::display::Text;
use craft_ui::testing::{Drive, StoryData, StoryOutput, story};
use egui::Ui;

use super::state::{DialogState, Env, Target};
use super::words::Say;
use super::{DialogTab, Options, ShortcutsDialog};

#[allow(clippy::panic, reason = "the story's own fixed chords are valid")]
fn binding(text: &str) -> Binding {
    Binding::parse(text).unwrap_or_else(|e| panic!("{text}: {e}"))
}

/// A small app: file commands, two tools that both default to `M` (the conflict), and commands of the Timeline
/// panel.
pub(super) fn keymap() -> Keymap {
    let path = |parts: &[&str]| parts.iter().map(|s| (*s).to_owned()).collect::<Vec<_>>();
    let timeline = Scope::new("Timeline").unwrap_or_else(Scope::app);
    let catalog = vec![
        CommandDef::new("file.save", "Save", Scope::app()).with_path(path(&["File", "Save"])).with_default(binding("Cmd+S")),
        CommandDef::new("file.export", "Export", Scope::app()).with_path(path(&["File", "Export"])).with_default(binding("Cmd+E")),
        CommandDef::new("tool.marquee", "Marquee Tool", Scope::app()).with_default(binding("M")),
        CommandDef::new("tool.move", "Move Tool", Scope::app()).with_default(binding("M")),
        CommandDef::new("timeline.loop", "Loop", timeline.clone()).with_default(binding("L")),
        CommandDef::new("timeline.play", "Play", timeline).with_default(binding("Space")),
    ];
    Keymap::new(catalog, Scopes::new().with_panel("Timeline"), Platform::Linux)
}

/// What a story sets up besides its options.
#[derive(Clone, Copy, Default)]
struct Setup {
    /// The dialog shows named sets.
    sets: bool,
    /// The dialog has an app tab.
    tabs: bool,
}

/// The dialog in the state `prepare` leaves it in. The state is built each pass: the stories do not drive it.
fn scene(ui: &mut Ui, options: &Options, setup: Setup, prepare: impl FnOnce(&mut Env<'_>, &mut DialogState)) -> StoryOutput {
    let mut keymap = keymap();
    let mut profiles = Profiles::default();
    let builtins = vec![("Defaults", Overrides::new())];
    if setup.sets {
        let _ = profiles.save_as("Mine", Overrides::new(), &builtins);
    }
    let mut state = DialogState::new();
    let say = Say::of(ui);
    {
        let sets = setup.sets.then(|| (&mut profiles, &builtins[..]));
        let mut env = Env { keymap: &mut keymap, sets, options, tabs: None, text: &say };
        state.begin(&mut env);
        prepare(&mut env, &mut state);
    }
    let mut tabs = [DialogTab::new("menus", "Menus", |ui| {
        Text::new("Menu colours").show(ui);
        false
    })];
    let mut dialog = ShortcutsDialog::new(&mut keymap).options(options);
    if setup.sets {
        dialog = dialog.sets(&mut profiles, &builtins);
    }
    if setup.tabs {
        dialog = dialog.tabs(&mut tabs);
    }
    dialog.show(ui, &mut state).into()
}

fn chord(text: &str) -> Chord {
    Chord::parse(text).unwrap_or(Chord::from(Key::F35))
}

#[story(ShortcutsDialog<'static>, covers = [], drive = Drive::Never)]
fn shortcuts_dialog(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    scene(ui, &Options::default(), Setup::default(), |_, _| {})
}

#[story(ShortcutsDialog<'static>, covers = [], drive = Drive::Never)]
fn recording(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    scene(ui, &Options::default(), Setup::default(), |env, state| {
        state.record(env, Target::Cell { command: "file.save".into(), old: Some(binding("Cmd+S")) });
    })
}

#[story(ShortcutsDialog<'static>, covers = [], drive = Drive::Never)]
fn conflict_bar(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    scene(ui, &Options::default(), Setup::default(), |env, state| {
        state.record(env, Target::Cell { command: "file.export".into(), old: None });
        state.feed(env, &[chord("Cmd+S")], 0.0);
    })
}

#[story(ShortcutsDialog<'static>, covers = [], drive = Drive::Never)]
fn message(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    scene(ui, &Options::default(), Setup::default(), |_, state| state.show_error("The file could not be read."))
}

#[story(ShortcutsDialog<'static>, covers = [], drive = Drive::Never)]
fn detail_pane(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    scene(ui, &Options::default(), Setup::default(), |_, state| state.select(&"file.save".into()))
}

#[story(ShortcutsDialog<'static>, covers = [], drive = Drive::Never)]
fn import_review(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    scene(ui, &Options::default(), Setup { sets: true, ..Setup::default() }, |env, state| {
        let read = |text: &str| ForeignKey::read(Some("Default"), text, Flavor::Windows);
        let set = ForeignSet::new(vec![
            ForeignEntry::new(Some("File"), ["Save"]).with_key(read("Ctrl+Alt+S")).at_line(3),
            ForeignEntry::new(Some("File"), ["Frobnicate"]).with_key(read("Ctrl+F")).at_line(4),
        ])
        .with_name("Theirs");
        let options = ImportOptions { contexts: vec![("Default".into(), ContextRule::Home)], ..ImportOptions::default() };
        state.review_import(env, set, options);
    })
}

#[story(ShortcutsDialog<'static>, covers = [], drive = Drive::Never)]
fn keyboard_pane(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    scene(ui, &Options::default().with(|o| o.keyboard = true), Setup::default(), |_, _| {})
}

#[story(ShortcutsDialog<'static>, covers = [], drive = Drive::Never)]
fn read_only(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    scene(ui, &Options::default().with(|o| o.editable = false), Setup::default(), |_, _| {})
}

#[story(ShortcutsDialog<'static>, covers = [], drive = Drive::Never)]
fn named_sets(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    scene(ui, &Options::default(), Setup { sets: true, ..Setup::default() }, |_, _| {})
}

#[story(ShortcutsDialog<'static>, covers = [], drive = Drive::Never)]
fn app_tabs(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    scene(ui, &Options::default(), Setup { tabs: true, ..Setup::default() }, |_, _| {})
}
