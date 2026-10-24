# Craft Keymap

`craft-keymap` holds the data and logic for keyboard bindings in the Craft apps: chords and their
text forms, bindings from chords to commands, scopes, conflicts, user overrides, named sets, an
import pipeline for other applications' shortcut sets, and an editor model. Each application
supplies its command catalog, its file-format readers and its UI.

## Boundaries

- Data and logic, plus a shared Keyboard Shortcuts dialog behind the `ui` feature. The apps keep
  the command catalog (ids, labels, menu paths, defaults), the readers for other applications'
  files, file dialogs and running the commands.
- Never crashes: no panics, no `unsafe`. Loading settings never fails; what cannot be used is
  kept or reported (`Keymap::problems`, the import report).
- Hostile input is capped: commands, bindings per command, text lengths, import entries, keys,
  path segments and fuzzy suggestions (the `MAX_*` constants). What is cut is counted.
- Reserved chords are opt-in. A keymap reserves no chord unless the app passes a list to
  `Keymap::with_reserved`, e.g. `reserved::system(platform)` for the operating system's window
  switching and closing chords. A reserved chord cannot be assigned or bound.
- Dependencies: `serde` and `serde_json`; `egui` with the `egui` feature; `craft-ui` and
  `craft-i18n` with the `ui` feature.

## A catalog and dispatch

A command with parameters has the key `window.panel {"panel":"layers"}` in settings and reports.
Tools of one group share a key and cycle. A binding can name a scope other than the command's
home scope.

```rust
use craft_keymap::{Active, Bindable, Binding, Chord, CommandRef, Keymap, Kind, Lookup, Platform, Scope, Scopes};
use serde_json::json;

let key = |text: &str| Binding::parse(text).unwrap();
let layers = CommandRef::with_params("window.panel", json!({"panel": "layers"}));
let select = Kind::Tool { group: Some("select".into()) };
let catalog = vec![
    Bindable::new("file.save", "Save", Scope::app()).with_path(vec!["File".into(), "Save".into()]).with_default(key("Cmd+S")),
    Bindable::new(layers.clone(), "Layers", Scope::app()).with_path(vec!["Window".into(), "Layers".into()]).with_default(key("F7")),
    Bindable::new("tool.marquee", "Marquee Tool", Scope::app()).with_kind(select.clone()).with_default(key("M")),
    Bindable::new("tool.ellipse", "Elliptical Marquee Tool", Scope::app()).with_kind(select).with_default(key("M")),
    // Home scope: the Timeline panel, where it wins over the app's L.
    Bindable::new("timeline.loop", "Loop", Scope::new("Timeline").unwrap()).with_default(key("L")),
    Bindable::new("layer.lock", "Lock Layer", Scope::app()).with_default(key("L")),
    // An app command whose binding works only in the Text context.
    Bindable::new("type.bold", "Bold", Scope::app()).with_default(key("Text: Cmd+B")),
];
let keymap = Keymap::new(catalog, Scopes::new().with_panel("Timeline").with_context("Text"), Platform::Mac);

// Dispatch: the chord pressed, whether it is a double press, and what has the keyboard.
let press = |text: &str, active: &Active| keymap.lookup(&Chord::parse(text).unwrap(), false, active);
assert_eq!(press("F7", &Active::NONE), Lookup::Run(layers));
assert_eq!(press("L", &Active::panel("Timeline")), Lookup::Run("timeline.loop".into()));
assert_eq!(press("L", &Active::NONE), Lookup::Run("layer.lock".into()));
assert_eq!(press("M", &Active::NONE), Lookup::Cycle(vec!["tool.marquee".into(), "tool.ellipse".into()]));
let editing = Active { contexts: &["Text"], typing: true, ..Active::NONE };
assert_eq!(press("Cmd+B", &editing), Lookup::Run("type.bold".into()));
assert_eq!(press("M", &editing), Lookup::None, "plain keys go to the text field");
```

While a text field has the keyboard (`Active::typing`), the keymap's `TypingRule` decides which
app and panel bindings wait: plain keys (the default), everything without the command modifier,
or everything. Under every rule, the platform's text-editing chords (`text_editing_chords`) go
to the field too: undo and redo, select all, and moving and deleting by word, line and text
(Cmd+Z, Alt+Left and Cmd+Backspace on macOS; Ctrl+Z, Ctrl+Y, Ctrl+Left and Ctrl+Home
elsewhere). Other command chords, such as Cmd+S, still fire. Bindings in an active context
always fire, so `Text: Cmd+B` works while typing, and a `Text: Cmd+Z` binding takes undo from
the field.

With the `egui` feature, `egui::Dispatcher` does this for a frame's key events (see Features).

## Chords

A [`Chord`](src/chord.rs) is a key plus the modifiers `cmd`, `ctrl`, `alt` and `shift`. `cmd` is
the command modifier: ⌘ on macOS, Ctrl on Windows and Linux. `ctrl` is the Mac Control key; off
macOS it folds into `cmd` (`Chord::effective`).

- Canonical text, as stored and sent: `Cmd+Ctrl+Alt+Shift+K`, `Cmd++` for Plus, `A A` for a
  double press.
- `Chord::parse` also reads common aliases (`⇧⌘Z`, `opt+left`, `Ctrl+Cmd+M`).
- `Chord::display(Platform)` gives `⇧⌘Z` on macOS and `Ctrl+Shift+Z` elsewhere.
- `chord::foreign::read_foreign` reads key text written by other applications (Windows or Mac
  modifier names, numeric keypad keys).

## Keyboard layouts

[`layout`](src/layout.rs) decides which chord a key press means, the same way for matching
(`Layout::candidates`), the shortcut recorder (`Layout::resolve`) and labels
(`Chord::display_for`):

1. Letters A–Z: the layout's Latin letter for the key. On a layout that is not Latin (Russian,
   Arabic, Hebrew, Greek, Thai), the suite's egui reports the Latin layout's character for each
   key: Ctrl+С on Dvorak + Russian is Dvorak's Ctrl+J, and Ctrl+Й, `'` on Dvorak, runs no letter
   binding. Without that report, the letter the toolkit reports for shortcuts, else the key's
   position. On a Latin layout (Dvorak, AZERTY), the letter typed. Arabic Ctrl+Shift+D is
   Ctrl+Shift+D, not the `]` it types.
2. Digits: the digit typed, else the key's position (AZERTY's `&é"` row). A digit-row key that
   types punctuation also matches its digit when the punctuation has no binding.
3. Punctuation: the character typed, without Shift when the layout needs Shift for it: German
   `/` is ⇧7, so ⇧⌘7 runs ⌘/.
4. Whether a layout is Latin is decided for the layout, not per key.
5. With Shift held on a Latin layout, a press that egui reports as the letter at its own
   position, because the character typed has no key (Dvorak's ⇧, types `<`), runs nothing when
   that position has been seen typing something other than its letter: once a `,` has been
   typed, Ctrl+Shift+, never runs Ctrl+Shift+W. A change in the reported Latin status clears
   what the positions typed; an app that learns of a layout switch itself calls
   `Layout::forget_letters`.
6. Labels show letters and digits by name, and a punctuation key as the character the layout
   types at its position when that character has no key of its own (`Ctrl+Ü` for Ctrl+[ on
   German).

A `KeyInput` holds what the toolkit reports for one press: modifiers, the key typed, the
physical key, and, when the platform reports them, the layout's Latin letter (Windows
virtual-key code, macOS ⌘ table, Linux Latin layout group) and whether Shift was needed. A
`Layout` holds whether the layout is Latin, as the platform reports it or as plain typing shows
it (`Layout::observe`), and the characters it types at punctuation positions.

## Storing overrides

`Overrides` is a field of the app's settings type. It is stored as a map of one text per command
(`{ "file.save": "Cmd+Alt+S", "edit.copy": "" }`, `""` for no shortcut) while no command has more
than one binding, so apps that stored that map before keep reading it in older builds. Once a
command has several bindings it is stored as an array of `{ command, keys }` tables
(`[{ command = "file.save", keys = ["Cmd+Alt+S", "F12"] }]`). Both forms load, and in the map form
a list of texts loads too.

In a craft-settings type, put `#[serde(with = "craft_keymap::keymap::overrides_list")]` on the
field, which always writes the array: craft-settings merges tables key by key with what the file
holds but replaces an array whole, so only the array lets a reset command (an entry removed) stay
removed after saving. Hand-written `[[shortcuts]]` blocks load as the array.

Texts the keymap cannot use (a key name from a newer version, an unknown scope, a reserved chord)
stay in the entry when the command is edited, unless the edit leaves it without bindings.
`Keymap::problems` lists them.

```rust
use craft_keymap::{Binding, Keymap, Overrides, Platform, Policy, Scopes};
use craft_settings::{Location, Settings, SettingsStore};
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Default, Clone, PartialEq)]
#[serde(default)]
struct Prefs {
    #[serde(with = "craft_keymap::keymap::overrides_list")]
    shortcuts: Overrides,
}

impl Settings for Prefs {
    const APP_ID: &'static str = "acmecraft";
    const SCHEMA_VERSION: u32 = 1;
}

// `catalog` is the app's Vec<Bindable>.
let (mut store, _report) = SettingsStore::<Prefs>::open(Location::Auto);
let mut keymap = Keymap::new(catalog, Scopes::new(), Platform::current()).with_overrides(store.get().shortcuts.clone());
// Entries for unknown commands and texts that do not parse are kept and listed here.
let _problems = keymap.problems();

// After each change in the shortcut editor:
keymap.set_bindings(&"file.save".into(), vec![Binding::parse("Cmd+Alt+S").unwrap()], Policy::Steal).ok();
store.update(|p| p.shortcuts = keymap.overrides().clone());
```

## Editing

- `set_bindings(&command, bindings, policy)` replaces a command's bindings. An editor with one
  shortcut per command calls it with the one binding (`vec![binding]`); `assign` adds a binding
  to those the command has.
- `remove` takes one binding away, `clear` leaves the command without shortcuts, `reset` gives it
  its defaults back and `reset_all` does so for every command.
- `Policy` says what happens to other commands holding the chord: `Steal` takes it from them,
  `Keep` lets both keep it and reports the clash, `Refuse` changes nothing and returns
  `AssignError::Taken`.
- `conflicts()` lists every pair of commands on one chord; `conflicts_with(&command)` lists the
  commands that share one command's chords, with that command as `a`.

## Shortcut sets

`Profiles` holds the user's named sets of overrides and which one is active. The app's built-in
sets (`&[(&str, Overrides)]`, the first one the default) are passed to each call and never change.
An app with sets stores only `Profiles`, in place of the `Overrides` field above: the keymap's
overrides are the active set's, and the dialog's OK stores them there. The sets, and each set's
overrides, are stored as arrays, so a deleted set or entry leaves a craft-settings file.

```rust
use craft_keymap::{Keymap, Platform, Profiles, Scopes};
use craft_settings::{Location, SettingsStore};

#[derive(Serialize, Deserialize, Default, Clone, PartialEq)]
#[serde(default)]
struct Prefs {
    shortcut_sets: Profiles,
}

// `builtins` is the app's &[(&str, Overrides)], the first set the default.
let (mut store, _report) = SettingsStore::<Prefs>::open(Location::Auto);
let mut sets = store.get().shortcut_sets.clone();
sets.reconcile(&builtins);
let keymap = Keymap::new(catalog, Scopes::new(), Platform::current()).with_overrides(sets.active_overrides(&builtins).clone());

// The dialog edits `sets` and `keymap`. On Outcome::Ok (or Changed, to save live):
store.update(|p| p.shortcut_sets = sets.clone());
```

- `names`, `get`, `active_name`, `active_overrides` (hand them to `Keymap::set_overrides`);
  `sets` (the user's sets) and `active` (the stored name, which `active_name` resolves to a set).
- `select` switches sets; `duplicate(from, to)` copies a built-in or user set; `save_as`,
  `rename` and `delete` work on user sets only.
- `store(overrides, builtins, copy_name)` saves the editor's result to the active set. Editing a
  built-in set makes a user copy named after `copy_name`, which becomes active.
- New names are made unique (`Custom 2`, `Custom 3`) and compare without case. A user set named
  like a built-in added later is renamed by `reconcile`.
- `export(name)` writes one set as JSON
  (`{"format":"craft-keymap","version":1,"name":…,"overrides":…}`); `import(text)` reads it into
  a new set. A file from a newer version is refused (`ProfileError::NewerVersion`).
- Loading never fails. `problems()` lists what loading dropped: sets with a bad or repeated name,
  overrides that are neither a list nor a map, sets past `MAX_SETS`, and a `version` newer than
  this crate writes. The app shows them before the next save writes the sets without them.
  Stored `Profiles` carry `version = 1`; a file without it reads as version 1.
- `import_plan(&plan, &keymap, name)` applies an import plan (see Importing a shortcut set) to a
  copy of the keymap and keeps the result as a new set, so another application's set becomes one
  the user can switch to and away from.

```rust
use craft_keymap::{Overrides, Profiles};

let mut classic = Overrides::new();
classic.insert("edit.copy", vec!["Cmd+Shift+C".into()]);
let builtins = [("Craft", Overrides::new()), ("Classic", classic)];
let mut sets = Profiles::default();
assert_eq!(sets.active_name(&builtins), "Craft");

sets.duplicate("Classic", "Mine", &builtins).unwrap();
let text = sets.export("Mine", &builtins).unwrap();
assert_eq!(sets.import(&text, &builtins), Ok("Mine 2".into()));
assert_eq!(sets.active_overrides(&builtins), sets.get("Mine", &builtins).unwrap());
```

## Importing a shortcut set

The app's reader turns a file (an InDesign Show Set, an Illustrator `.kys`) into an
[`import::ForeignSet`](src/import.rs): entries with a menu, a path and keys read by
`ForeignKey::read`. The reader builds it with `ForeignSet::new(entries)`, `with_name` and
`with_truncated`; the struct is `#[non_exhaustive]`, so struct literals outside the crate do not
compile. `import::plan` matches each entry to one command and changes nothing;
`ImportPlan::apply` applies the plan and returns an `ImportReport` (serde, camelCase). Report
rows carry the entry's `item` (its path) and `line`.

```rust
use craft_keymap::{Bindable, Binding, ContextRule, Flavor, ForeignEntry, ForeignKey, ForeignSet, ImportOptions, Keymap, Platform};
use craft_keymap::{Scope, Scopes, plan};

let save = Bindable::new("file.save", "Save", Scope::app())
    .with_path(vec!["File".into(), "Save".into()])
    .with_default(Binding::parse("Cmd+S").unwrap());
let mut keymap = Keymap::new(vec![save], Scopes::new(), Platform::Windows);

let read = |text: &str| ForeignKey::read(Some("Default"), text, Flavor::Windows);
let set = ForeignSet::new(vec![
    ForeignEntry::new(Some("File"), ["Save"]).with_key(read("Ctrl+Alt+S")).at_line(12),
    ForeignEntry::new(Some("File"), ["Frobnicate"]).with_key(read("Ctrl+F")).at_line(13),
])
.with_name("My Set");
let options = ImportOptions { contexts: vec![("Default".into(), ContextRule::Home)], ..ImportOptions::default() };
let planned = plan(&set, &keymap, &options);
// A review UI can show planned.commands and planned.report here.
let report = planned.apply(&mut keymap);
assert_eq!(report.changed[0].to, ["Cmd+Alt+S"]);
assert_eq!((report.unmatched[0].item.as_str(), report.unmatched[0].line), ("File > Frobnicate", Some(13)));
```

- Matching, most specific first, each step needing exactly one command: the user's remembered
  answers, the full menu path, the joined submenu segments as a label or alias in that menu, the
  label in that menu, the joined segments as a label or alias anywhere, the label anywhere. An
  entry with no menu and several segments matches only by its full path, unless the reader marks
  it `in_panel_menu()`. Tool entries match only tools.
- Names are compared after `import::normalize_label`: case, `…`, `...`, `&`, colons and extra
  spaces aside, so the alias `Preferences: General…` matches `Edit > Preferences > General...`.
  Aliases (`Bindable::with_alias`) count wherever labels do, and import paths
  (`Bindable::with_import_path`, such as `["Tools", label]` for a tool) wherever menu paths do;
  the dialog makes no areas or groups from import paths.
- Panel menus: sources list panel-menu items with the panel's name in front (InDesign's Panel
  Menus area writes `Character: All Caps`; Illustrator and Photoshop sets have similar areas).
  The reader splits the name into segments and marks the entry `ForeignEntry::in_panel_menu()`.
  Such an entry also matches a catalog path that ends with its segments (with the command's
  label or the path's last segment last), and its last segment alone as a label. The last
  segment alone counts only when the first segment, the panel, is a segment of some catalog
  path, so `History: New Document` matches nothing in an app without a History panel. An entry
  without keys that matched only these ways leaves the command's shortcuts alone and is listed
  in `matched_without_change` (reason `PanelMenu`).
- Show/Hide: a source often lists one toggle as two items, `Show Rulers` and `Hide Rulers`. Names
  are matched as written first. When no step finds a command, the steps run again on commands
  marked `Bindable::toggle()`, with a leading `Show `, `Hide ` or `Show/Hide ` dropped from the
  entry's label and from the command's label, aliases and path labels. A command not marked as a
  toggle never matches through a dropped prefix, so `Hide Conveyor` does not match `Conveyor`.
- `ImportOptions.contexts` says what each source context becomes (home scope, another scope, the
  home scope only without a home entry, or skipped).
- The set wins: a chord it gives to a command is taken from commands outside it (except twins
  sharing it as a default), a later entry takes a chord from an earlier one (as when the entries
  are applied in file order; the earlier key is listed in `skipped`), and an entry without keys
  can clear its command. It clears it only when no entry matched to the same command has keys,
  whatever the order of the entries.
- Keys read approximately (a numeric keypad key bound as the main key) are listed in
  `approximated`.
- With `ImportOptions.suggest`, unmatched entries get fuzzy candidates (`import::namematch`);
  they are reported, never applied. Only the first 200 unmatched entries with keys get them, and
  only the first 200 characters of a name are compared. Keys in `ContextRule::Skip` contexts do
  not count: an unmatched entry with only such keys counts in `unmatched_without_shortcut`.
- `truncated` counts what was cut at the caps (`MAX_IMPORT_ENTRIES` and the others), plus the
  entries the app's reader dropped at its own limits, which it passes as `ForeignSet.truncated`.

## Editor

`editor` is the model behind a shortcut dialog; the app draws it.

- `EditSession::begin(&keymap)` when the dialog opens. Each change is one undo step:
  `assign`, `replace` (record over one key of a command), `remove`, `clear`, `reset`, `reset_all`,
  or any keymap edit through `session.edit(&mut keymap, |km| …)`. `undo`, `redo`, `cancel` (back
  to the state at open) and `is_modified`.
- Conflicts: `editor::holders(&keymap, &command, &binding)` lists the commands that hold the key
  while the user records; `Keymap::conflicts_with(&command)` marks the rows whose keys are
  shared after a `Keep`. Assigning with `Policy::Refuse` returns `AssignError::Taken` and changes
  nothing; the user's answer is a second call with `Policy::Steal` or `Policy::Keep`.
- `Recorder::press(chord, time)` gives `Single`, `Double` (the same chord again within
  `DOUBLE_PRESS`, 0.6 s) or `Cancel` (Escape).
- `editor::search(&keymap, query, platform)` filters the catalog by label, alias, menu path,
  command id and key text, canonical or as shown on `platform`.

```rust
use craft_keymap::editor::holders;
use craft_keymap::{AssignError, Binding, Chord, EditSession, Policy, Recorded, Recorder};

// `keymap` is the app's Keymap, `now` the input time in seconds.
let mut session = EditSession::begin(&keymap);
let mut recorder = Recorder::new();
let save = "file.save".into();
// The user presses Cmd+C in the shortcut field of Save.
if let Recorded::Single(trigger) = recorder.press(Chord::parse("Cmd+C").unwrap(), now) {
    let binding = Binding::new(trigger);
    let _shown = holders(&keymap, &save, &binding);
    if let Err(AssignError::Taken(_)) = session.assign(&mut keymap, &save, binding.clone(), Policy::Refuse) {
        // The user chose to take the key from Copy.
        session.assign(&mut keymap, &save, binding, Policy::Steal).ok();
    }
}
// OK: store `keymap.overrides()` (`Profiles::store` for sets). Cancel:
session.cancel(&mut keymap);
```

## Shortcuts dialog

With the `ui` feature, `ui::ShortcutsDialog` draws the Keyboard Shortcuts dialog over a `Keymap`
and, optionally, `Profiles`: a table of the catalog grouped by menu, recording, conflicts, undo,
named sets, import review and a summary. It edits the keymap in place, so menus show new keys at
once, and Cancel puts everything back as it was when the dialog opened.

Adopt it in three steps:

1. Build the `Keymap` from the catalog and keep the built-in sets (`Builtins`), as above.
2. While the dialog is open, keep a `DialogState` and call `show` (a window) or `ui` (the body,
   for the app's own modal or preferences page) every frame.
3. Act on the `Outcome` it returns, and drop the state when `is_open()` turns false.

Apps that import other applications' sets read each file into a `ForeignSet` built with
`ForeignSet::new` ([Importing a shortcut set](#importing-a-shortcut-set)).

```rust,ignore
use craft_keymap::ui::{DialogState, I18nText, Options, Outcome, ShortcutsDialog};

// "Keyboard Shortcuts…" opens it; "Edit Shortcut…" also calls `state.select(&command)`.
self.shortcuts = Some(DialogState::new());

// Every frame, before the app's own dispatch:
if let Some(state) = &mut self.shortcuts {
    let text = I18nText { i18n: &self.i18n, lang: self.lang };
    let dialog = ShortcutsDialog::new(&mut self.keymap, &text).options(&self.shortcut_options);
    let dialog = dialog.sets(&mut self.settings.shortcut_sets, &BUILTINS);
    match dialog.show(ctx, state) {
        Some(Outcome::Ok) => self.save_settings(),
        // After each edit, and after a Cancel that undid edits.
        Some(Outcome::Changed) => self.rebuild_menus(),
        Some(Outcome::Import { formats }) => self.pick_import_file(formats),
        Some(Outcome::Export { name, json }) => self.save_file(&name, json),
        Some(Outcome::Summary { name, html, .. }) => self.show_summary(&name, html),
        Some(Outcome::Remembered(answers)) => self.settings.import_answers.extend(answers),
        _ => {}
    }
    if !state.is_open() {
        self.shortcuts = None;
    }
}
```

Outcomes:

| Outcome | The app |
|---|---|
| `Ok` | Saves `Profiles` (with sets) or `keymap.overrides()`. |
| `Cancel` | Nothing to save: the keymap and the sets are as they were at open. When Cancel undid edits, `Changed` follows. |
| `Changed` | Updates its menus, or saves at once if it saves live. It comes after each edit and after a Cancel that undid edits. When a frame also has another outcome, `Changed` comes on the next frame; `is_open()` stays true until it has. |
| `Import { formats }` | Opens a file of one of the formats, then calls `import_json` (craft-keymap JSON, `ui::JSON_FORMAT`) or reads it into a `ForeignSet` and calls `review_import`. |
| `Export { name, json }`, `ExportAs { format, name, keymap }` | Writes the set where the user chooses (a download on the web). |
| `Summary { name, html, text }` | Shows, saves or prints the summary. |
| `Remembered(answers)` | Keeps the user's import answers for the next import (`ImportOptions::remembered`). |

The crate never opens a file dialog or touches the file system. When the app's own step fails
while the dialog is open (a file it cannot read), `state.show_error(text)` shows the translated
message under the table until the user dismisses it or the next action replaces it.

Key dispatch while the dialog is open:

- Skip `Dispatcher::dispatch` while `DialogState::is_open()` is true. The dialog takes the
  frame's key, text and clipboard events itself while it records, so the chord the user presses
  reaches the recorder and runs nothing.
- A native macOS menu handles its key equivalents before egui sees them, so Cmd+Q, Cmd+, and
  the other menu chords never reach the recorder. While `is_recording()` is true, pause them:
  feed them to egui as key presses, or rebuild the menu without accelerators.

Words come from `ui::Text`. Apps on craft-i18n pass `I18nText`; others implement `ui` and
`plural` over their own lookup, and `rtl` and `visual` (the app's bidi reordering) for
right-to-left languages, which mirror the layout. Chord text is never reordered. `ui::STRINGS`
and `ui::PLURALS` list every string the dialog looks up, for the translation catalog.
`Options` sets the conflict mode, double-press recording, area tabs, the on-screen keyboard,
read-only rows for keys outside the catalog, the app's import and export formats, an `accept`
rule for which keys a command may get, and a `visible` rule for which catalog entries the table
lists (hidden entries keep their keys and still count in conflicts).

## Features

- `egui`: conversions to and from egui keys and modifiers; `KeyInput::from_egui`;
  `egui::matches`, which accepts the shifted key egui reports for a Shift binding (`Shift+;`
  matches `:`); and `egui::Dispatcher`, which runs a frame's key presses against the keymap,
  reading them by the layout it learns from typing or is given (`with_layout`, `layout()`):

  ```rust,ignore
  let hits = ctx.input_mut(|i| dispatcher.dispatch(i, &keymap, &active));
  for hit in hits {
      app.run(&hit.command);
  }
  ```

  A press that runs a command is removed from egui's events, with the text it typed, so widgets
  do not see it too. Each `Hit` carries the `trigger` that matched and `exact`, which is false
  when the press matched through a stand-in (`:` for `Shift+;`, `+` for `=`).
  `dispatch_filtered` takes a predicate that can decline a hit, for example a chord the native
  menu has already handled: the press and its text then stay in egui's events, in their place.
  While a text field has the keyboard, plain clipboard key presses and presses that typed text in the
  same frame stay with it. That covers AltGr, which a browser on Windows reports as Ctrl+Alt
  followed by the text; a Ctrl+Alt press that types nothing is a command chord.
  `Dispatcher::new().with_repeat(true)` also runs commands on key repeats (held arrows).

  On the desktop, egui-winit sends egui's Copy, Cut and Paste events in place of the key press
  for any chord of Cmd with C, X or V (Ctrl off macOS); the web build sends the press and then
  the event. For the plain chords Cmd+C, Cmd+X and Cmd+V (and Cmd+Insert, Shift+Delete,
  Shift+Insert), the events stay with the app by default, so a binding on those chords runs only
  with `with_clipboard_events(true)`. Then the events count as their chords outside text fields,
  and a Paste event's text moves to `Hit::pasted`. On the web, the press runs the command and
  takes the event, with its text, along. An app whose paste command asks egui for a paste gets a
  Paste event back, which runs that command again; such an app keeps the default. A clipboard
  event with more modifiers held (Cmd+Shift+V, Cmd+Alt+C) always counts as its chord, also while
  typing, and stays when that chord has no binding. egui-winit sends no Paste event when the
  clipboard is empty or holds an image, so a binding on a Cmd+V chord cannot run then on the
  desktop.

- `ui`: the Shortcuts dialog (see above). Implies `egui`.
- `snapshot` (development): `tests/ui_snapshot.rs` writes PNGs of the dialog to
  `CRAFT_KEYMAP_SNAPSHOT_DIR` for review. egui's default fonts lack ⇧ ⌥ ⌃ and the arrows; the
  test adds the font in `CRAFT_KEYMAP_SNAPSHOT_FONT`, DejaVu Sans when it is installed, and BIZ
  UDPGothic from a craft-fonts checkout in `CRAFT_FONTS_DIR` (⇧ ⌘ and arrows only).
