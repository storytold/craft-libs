# Craft Keymap

`craft-keymap` holds the data and logic for keyboard bindings in the Craft apps: chords and their
text forms, bindings from chords to commands, scopes, conflicts, user overrides, named sets, an
import pipeline for other applications' shortcut sets, and an editor model. Each application
supplies its command catalogue, its file-format readers and its UI.

## Boundaries

- Data and logic only. The apps keep the command catalogue (ids, labels, menu paths, defaults),
  the readers for other applications' files, the shortcut editor UI and running the commands.
- Never crashes: no panics, no `unsafe`. Loading settings never fails; what cannot be used is
  kept or reported (`Keymap::problems`, the import report).
- Hostile input is capped: commands, bindings per command, text lengths, import entries, keys,
  path segments and fuzzy suggestions (the `MAX_*` constants). What is cut is counted.
- Dependencies: `serde` and `serde_json`; `egui` with the `egui` feature.

## A catalogue and dispatch

A command with parameters has the key `window.panel {"panel":"layers"}` in settings and reports.
Tools of one group share a key and cycle. A binding can name a scope other than the command's
home scope.

```rust
use craft_keymap::{Active, Bindable, Binding, Chord, CommandRef, Keymap, Kind, Lookup, Platform, Scope, Scopes};
use serde_json::json;

let key = |text: &str| Binding::parse(text).unwrap();
let layers = CommandRef::with_params("window.panel", json!({"panel": "layers"}));
let select = Kind::Tool { group: Some("select".into()) };
let catalogue = vec![
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
let keymap = Keymap::new(catalogue, Scopes::new().with_panel("Timeline").with_context("Text"), Platform::Mac);

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

## Storing overrides

`Overrides` is a field of the app's craft-settings type. It is stored as an array of
`{ command, keys }` tables (`shortcuts = [{ command = "file.save", keys = ["Cmd+Alt+S"] }]`, or
`[[shortcuts]]` blocks), not as a table keyed by command: craft-settings merges tables key by key
with what the file holds, but replaces an array whole, so a reset command (an entry removed)
stays removed after saving. The map form `{ "<command>" = [keys] }` still loads.

```rust
use craft_keymap::{Binding, Keymap, Overrides, Platform, Policy, Profiles, Scopes};
use craft_settings::{Location, Settings, SettingsStore};
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Default, Clone, PartialEq)]
#[serde(default)]
struct Prefs {
    shortcuts: Overrides,
    shortcut_sets: Profiles,
}

impl Settings for Prefs {
    const APP_ID: &'static str = "acmecraft";
    const SCHEMA_VERSION: u32 = 1;
}

// `catalogue` is the app's Vec<Bindable>.
let (mut store, _report) = SettingsStore::<Prefs>::open(Location::Auto);
let mut keymap = Keymap::new(catalogue, Scopes::new(), Platform::current()).with_overrides(store.get().shortcuts.clone());
// Entries for unknown commands and texts that do not parse are kept and listed here.
let _problems = keymap.problems();

// After each change in the shortcut editor:
keymap.assign(&"file.save".into(), Binding::parse("Cmd+Alt+S").unwrap(), Policy::Steal).ok();
store.update(|p| p.shortcuts = keymap.overrides().clone());
```

## Importing a shortcut set

The app's reader turns a file (an InDesign Show Set, an Illustrator `.kys`) into an
[`import::ForeignSet`](src/import.rs): entries with a menu, a path and keys read by
`ForeignKey::read`. `import::plan` matches each entry to one command and changes nothing;
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
let set = ForeignSet {
    name: Some("My Set".into()),
    entries: vec![
        ForeignEntry::new(Some("File"), ["Save"]).with_key(read("Ctrl+Alt+S")).at_line(12),
        ForeignEntry::new(Some("File"), ["Frobnicate"]).with_key(read("Ctrl+F")).at_line(13),
    ],
};
let options = ImportOptions { contexts: vec![("Default".into(), ContextRule::Home)], ..ImportOptions::default() };
let planned = plan(&set, &keymap, &options);
// A review UI can show planned.commands and planned.report here.
let report = planned.apply(&mut keymap);
assert_eq!(report.changed[0].to, ["Cmd+Alt+S"]);
assert_eq!((report.unmatched[0].item.as_str(), report.unmatched[0].line), ("File > Frobnicate", Some(13)));
```

- Matching, most specific first, each step needing exactly one command: the user's remembered
  answers, the full menu path, the joined submenu segments as a label in that menu, the label in
  that menu, the label or an alias anywhere. Labels are compared after `import::normalize_label`.
  Tool entries match only tools.
- `ImportOptions.contexts` says what each source context becomes (home scope, another scope, the
  home scope only without a home entry, or skipped).
- The set wins: a chord it gives to a command is taken from commands outside it (except twins
  sharing it as a default), the first entry keeps a chord, and an entry without keys can clear
  its command.
- Keys read approximately (a numeric keypad key bound as the main key) are listed in
  `approximated`.
- With `ImportOptions.suggest`, unmatched entries get fuzzy candidates (`import::namematch`);
  they are reported, never applied. Only the first 200 unmatched entries get them, and only the
  first 200 characters of a name are compared.

## Editor

[`editor`](src/editor.rs) is the model behind a shortcut editor; the app draws it.

- `EditSession::begin(&keymap)` when the editor opens; each change goes through
  `session.edit(&mut keymap, |km| km.assign(..))`, which makes it an undo step. `undo`, `redo`,
  `cancel` (back to where the session began) and `is_modified`.
- `Recorder::press(chord, time)` gives `Single`, `Double` (the same chord again within
  `DOUBLE_PRESS`, 0.6 s) or `Cancel` (Escape).
- `editor::search(&keymap, query, platform)` filters the catalogue by label, alias, menu path,
  command id and key text.

## Features

- `egui`: conversions to and from egui keys and modifiers; `egui::matches`, which accepts the
  shifted key egui reports for a Shift binding (`Shift+;` matches `:`); and `egui::Dispatcher`,
  which runs a frame's key presses against the keymap:

  ```rust,ignore
  let hits = ctx.input_mut(|i| dispatcher.dispatch(i, &keymap, &active));
  for hit in hits {
      app.run(&hit.command);
  }
  ```

  A press that runs a command is removed from egui's events, with the text it typed, so widgets
  do not see it too. egui's Copy, Cut and Paste events count as Cmd+C, Cmd+X and Cmd+V outside
  text fields. While a text field has the keyboard, clipboard events, clipboard key presses,
  presses followed by text, and off macOS Ctrl+Alt with a letter, digit, punctuation or Space
  stay with it: AltGr arrives as Ctrl+Alt on Windows and some Linux setups, and egui-winit sends
  no text while Ctrl is held.
  `Dispatcher::new().with_repeat(true)` also runs commands on key repeats (held arrows).
