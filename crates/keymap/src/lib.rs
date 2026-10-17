#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

//! Keyboard bindings for the Craft suite.
//!
//! [`chord`] holds the key and modifier types, the canonical text form that settings and control
//! protocols store, display text per platform, and a reader for other applications' key text.
//! [`command`] names what a binding runs, [`scope`] where it works, and [`keymap`] holds an app's
//! catalogue with default bindings, the user's [`Overrides`], dispatch ([`Keymap::lookup`]),
//! editing and conflicts. [`profiles`] keeps named sets of overrides; [`reserved`] lists the
//! chords each operating system takes. [`import`] brings in other applications' shortcut sets.
//! [`editor`] is the model of a shortcut editor: undo, a chord recorder and search.
//! With the `egui` feature, [`egui`](mod@crate::egui) converts to and from egui keys, matches
//! key presses against bindings and dispatches a frame's key events.
//!
//! ```
//! use craft_keymap::{Chord, Platform};
//!
//! let chord = Chord::parse("⇧⌘Z").unwrap();
//! assert_eq!(chord.to_string(), "Cmd+Shift+Z");
//! assert_eq!(chord.display(Platform::Mac), "⇧⌘Z");
//! assert_eq!(chord.display(Platform::Windows), "Ctrl+Shift+Z");
//! ```
//!
//! # A catalogue and dispatch
//!
//! The app lists every command it can bind. A command with parameters has the key
//! `window.panel {"panel":"layers"}` in settings and reports. Tools of one group share a key and
//! cycle. A binding can name a scope other than the command's home scope.
//!
//! ```
//! use craft_keymap::{Active, Bindable, Binding, Chord, CommandRef, Keymap, Kind, Lookup, Platform, Scope, Scopes};
//! use serde_json::json;
//!
//! let key = |text: &str| Binding::parse(text).unwrap();
//! let layers = CommandRef::with_params("window.panel", json!({"panel": "layers"}));
//! let select = Kind::Tool { group: Some("select".into()) };
//! let catalogue = vec![
//!     Bindable::new("file.save", "Save", Scope::app()).with_path(vec!["File".into(), "Save".into()]).with_default(key("Cmd+S")),
//!     Bindable::new(layers.clone(), "Layers", Scope::app()).with_path(vec!["Window".into(), "Layers".into()]).with_default(key("F7")),
//!     Bindable::new("tool.marquee", "Marquee Tool", Scope::app()).with_kind(select.clone()).with_default(key("M")),
//!     Bindable::new("tool.ellipse", "Elliptical Marquee Tool", Scope::app()).with_kind(select).with_default(key("M")),
//!     // Home scope: the Timeline panel, where it wins over the app's L.
//!     Bindable::new("timeline.loop", "Loop", Scope::new("Timeline").unwrap()).with_default(key("L")),
//!     Bindable::new("layer.lock", "Lock Layer", Scope::app()).with_default(key("L")),
//!     // An app command whose binding works only in the Text context.
//!     Bindable::new("type.bold", "Bold", Scope::app()).with_default(key("Text: Cmd+B")),
//! ];
//! let keymap = Keymap::new(catalogue, Scopes::new().with_panel("Timeline").with_context("Text"), Platform::Mac);
//! assert_eq!(layers.key(), r#"window.panel {"panel":"layers"}"#);
//!
//! // Dispatch: the chord pressed, whether it is a double press, and what has the keyboard.
//! let press = |text: &str, active: &Active| keymap.lookup(&Chord::parse(text).unwrap(), false, active);
//! assert_eq!(press("F7", &Active::NONE), Lookup::Run(layers));
//! assert_eq!(press("L", &Active::panel("Timeline")), Lookup::Run("timeline.loop".into()));
//! assert_eq!(press("L", &Active::NONE), Lookup::Run("layer.lock".into()));
//! assert_eq!(press("M", &Active::NONE), Lookup::Cycle(vec!["tool.marquee".into(), "tool.ellipse".into()]));
//! let editing = Active { contexts: &["Text"], typing: true, ..Active::NONE };
//! assert_eq!(press("Cmd+B", &editing), Lookup::Run("type.bold".into()));
//! assert_eq!(press("M", &editing), Lookup::None, "plain keys go to the text field");
//! ```
//!
//! With the `egui` feature, [`egui::Dispatcher`](crate::egui) does this for a frame's key events.
//!
//! # Storing overrides
//!
//! [`Overrides`] is a field of the app's craft-settings type. It is stored as an array of
//! `{ command, keys }` tables, not as a table keyed by command: craft-settings merges tables key
//! by key with what the file holds, but replaces an array whole, so a reset command (an entry
//! removed) stays removed after saving.
//!
//! ```no_run
//! use craft_keymap::{Binding, Keymap, Overrides, Platform, Policy, Profiles, Scopes};
//! use craft_settings::{Location, Settings, SettingsStore};
//! use serde::{Deserialize, Serialize};
//!
//! #[derive(Serialize, Deserialize, Default, Clone, PartialEq)]
//! #[serde(default)]
//! struct Prefs {
//!     shortcuts: Overrides,
//!     shortcut_sets: Profiles,
//! }
//!
//! impl Settings for Prefs {
//!     const APP_ID: &'static str = "acmecraft";
//!     const SCHEMA_VERSION: u32 = 1;
//! }
//!
//! # let catalogue = Vec::new();
//! let (mut store, _report) = SettingsStore::<Prefs>::open(Location::Auto);
//! let mut keymap = Keymap::new(catalogue, Scopes::new(), Platform::current()).with_overrides(store.get().shortcuts.clone());
//! // Entries for unknown commands and texts that do not parse are kept and listed here.
//! let _problems = keymap.problems();
//!
//! // After each change in the shortcut editor:
//! keymap.assign(&"file.save".into(), Binding::parse("Cmd+Alt+S").unwrap(), Policy::Steal).ok();
//! store.update(|p| p.shortcuts = keymap.overrides().clone());
//! ```
//!
//! # Importing a shortcut set
//!
//! The app's reader builds a [`ForeignSet`]; [`plan`] matches it without changing anything, so a
//! review UI can show [`ImportPlan::commands`] and the report first.
//!
//! ```
//! use craft_keymap::{Bindable, Binding, ContextRule, Flavor, ForeignEntry, ForeignKey, ForeignSet, ImportOptions, Keymap, Platform};
//! use craft_keymap::{Scope, Scopes, plan};
//!
//! let save = Bindable::new("file.save", "Save", Scope::app())
//!     .with_path(vec!["File".into(), "Save".into()])
//!     .with_default(Binding::parse("Cmd+S").unwrap());
//! let mut keymap = Keymap::new(vec![save], Scopes::new(), Platform::Windows);
//!
//! let read = |text: &str| ForeignKey::read(Some("Default"), text, Flavor::Windows);
//! let set = ForeignSet {
//!     name: Some("My Set".into()),
//!     entries: vec![
//!         ForeignEntry::new(Some("File"), ["Save"]).with_key(read("Ctrl+Alt+S")).at_line(12),
//!         ForeignEntry::new(Some("File"), ["Frobnicate"]).with_key(read("Ctrl+F")).at_line(13),
//!     ],
//! };
//! let options = ImportOptions { contexts: vec![("Default".into(), ContextRule::Home)], ..ImportOptions::default() };
//! let planned = plan(&set, &keymap, &options);
//! assert_eq!(planned.commands.len(), 1);
//!
//! let report = planned.apply(&mut keymap);
//! assert_eq!(report.changed[0].to, ["Cmd+Alt+S"]);
//! assert_eq!((report.unmatched[0].item.as_str(), report.unmatched[0].line), ("File > Frobnicate", Some(13)));
//! assert_eq!(keymap.bindings(&"file.save".into()), [Binding::parse("Cmd+Alt+S").unwrap()]);
//! ```

pub mod chord;
pub mod command;
pub mod editor;
#[cfg(feature = "egui")]
pub mod egui;
pub mod import;
pub mod keymap;
pub mod profiles;
pub mod reserved;
pub mod scope;

pub use chord::foreign::Flavor;
pub use chord::{Chord, Key, MAX_CHORD_TEXT, Mods, ParseError, Platform, Trigger};
pub use command::{CommandRef, MAX_KEY};
pub use editor::{DOUBLE_PRESS, EditSession, MAX_UNDO, MAX_UNDO_BYTES, Recorded, Recorder};
pub use import::{ContextRule, ForeignEntry, ForeignKey, ForeignSet, ImportOptions, ImportPlan, ImportReport, NoneDefined, Start, plan};
pub use keymap::{
    AssignError, Assigned, Bindable, Binding, BindingError, Conflict, ConflictKind, DropReason, Holder, Keymap, Kind, Lookup, MAX_BINDING_TEXT,
    MAX_BINDINGS, MAX_COMMANDS, MAX_CONFLICTS, Overrides, Policy, Problem, TypingRule,
};
pub use profiles::{Builtins, MAX_SET_NAME, MAX_SETS, ProfileError, Profiles};
pub use scope::{Active, MAX_SCOPE_NAME, Scope, ScopeKind, Scopes};
