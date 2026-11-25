//! The Keyboard Shortcuts dialog: a table of the catalog with recording, conflicts, undo, named
//! sets, import review and a summary, over a [`Keymap`] and optionally [`Profiles`]. It edits the
//! keymap in place, so menus show new keys at once, and Cancel puts everything back as it was when
//! the dialog opened.
//!
//! Adopt it in three steps:
//!
//! 1. Build the `Keymap` from the catalog and keep the built-in sets (`Builtins`), as
//!    `craft_keymap` describes.
//! 2. While the dialog is open, keep a [`DialogState`] and call [`ShortcutsDialog::show`] every
//!    pass. An app that hosts the dialog in its own preferences shows it from a [`DialogTab`] of
//!    its own pattern.
//! 3. Act on the [`ShortcutsOutput::outcome`] it returns, and drop the state when
//!    [`DialogState::is_open`] turns false.
//!
//! Apps that import other applications' sets read each file into a
//! [`ForeignSet`] and call [`ShortcutsDialog::review_import`].
//!
//! ```rust,ignore
//! use craft_menu::shortcuts::{DialogState, Options, Outcome, ShortcutsDialog};
//!
//! // "Keyboard Shortcuts…" opens it; "Edit Shortcut…" also calls `state.select(&command)`.
//! self.shortcuts = Some(DialogState::new());
//!
//! // Every pass while it is open:
//! if let Some(state) = &mut self.shortcuts {
//!     let dialog = ShortcutsDialog::new(&mut self.keymap).options(&self.shortcut_options);
//!     let dialog = dialog.sets(&mut self.settings.shortcut_sets, &BUILTINS);
//!     match dialog.show(ui, state).outcome {
//!         Some(Outcome::Ok) => self.save_settings(),
//!         // After each edit, and after a Cancel that undid edits.
//!         Some(Outcome::Changed) => self.rebuild_menus(),
//!         Some(Outcome::Import { formats, .. }) => self.pick_import_file(formats),
//!         Some(Outcome::Export { name, json, .. }) => self.save_file(&name, json),
//!         Some(Outcome::Summary { name, html, .. }) => self.show_summary(&name, html),
//!         Some(Outcome::Remembered { targets, codes }) => self.settings.import_answers.extend(targets, codes),
//!         _ => {}
//!     }
//!     if !state.is_open() {
//!         self.shortcuts = None;
//!     }
//! }
//! ```
//!
//! Outcomes:
//!
//! | Outcome | The app |
//! |---|---|
//! | `Ok` | Saves `Profiles` (with sets) or `keymap.overrides()`. |
//! | `Ok` in [`Layout::CommandLists`] | Close: there is no Cancel, each edit is applied at once and reported as `Changed`, so closing keeps them. |
//! | `Cancel` | Nothing to save: the keymap and the sets are as they were at open. When Cancel undid edits, `Changed` follows. |
//! | `Changed` | Updates its menus, or saves at once if it saves live. It comes after each edit and after a Cancel that undid edits. When a pass also has another outcome, `Changed` comes on the next pass; `is_open()` stays true until it has. |
//! | `Import { formats }` | Opens a file of one of the formats, then calls [`ShortcutsDialog::import_json`] (craft-keymap JSON, [`JSON_FORMAT`]) or reads it into a `ForeignSet` and calls [`ShortcutsDialog::review_import`]. |
//! | `Export { name, json }`, `ExportAs { format, name, keymap }` | Writes the set where the user chooses (a download on the web). |
//! | `Summary { name, html, text }` | Shows, saves or prints the summary. |
//! | `Remembered { targets, codes }` | Keeps the user's import answers for the next import (`ImportOptions::remembered` and `ImportOptions::codes`). |
//!
//! The crate never opens a file dialog or touches the file system. When the app's own step fails
//! while the dialog is open (a file it cannot read), [`DialogState::show_error`] shows the
//! translated message under the table until the user dismisses it or the next action replaces it.
//!
//! Key dispatch while the dialog is open: the runtime's key stage gives a recording component
//! every key, so the chord the user presses reaches the recorder and runs nothing. A native macOS
//! menu handles its key equivalents before the pass sees them, so Cmd+Q, Cmd+, and the other menu
//! chords never reach the recorder; while [`DialogState::is_recording`] is true, pause them.
//!
//! The dialog reads its words from [`ShortcutWords`], which the app translates like any word set;
//! key names come from the display words, and the direction from the environment: a right-to-left
//! interface mirrors the layout and moves the text around a chord to the other side. Command
//! labels and tooltips come from the Host's `Commands`: install a `Host` whose commands are
//! `KeymapCommands::new(i18n, lang)`, or the `CommandDef` labels show as written. Menu path parts
//! and scope names are looked up in the Host's catalog.
//!
//! [`Options`] sets the conflict mode, double-press recording, area tabs, the on-screen keyboard,
//! read-only rows for keys outside the catalog, the app's import and export formats, an `accept`
//! rule for which keys a command may get, and a `visible` rule for which catalog entries the table
//! lists (hidden entries keep their keys and still count in conflicts).

use std::cell::RefCell;
use std::collections::BTreeMap;
use std::ops::RangeInclusive;

use craft_behavior::edit_history::Step;
use craft_keymap::{Binding, Builtins, CommandDef, CommandRef, ForeignSet, ImportOptions, Keymap, ProfileError, Profiles, Trigger};
use craft_ui::contract::{Animate as _, Bind, NoValue, Pattern, SaveKey, ShowContainer as _};
use craft_ui::dialog::{Dialog, DialogOutput};
use craft_ui::runtime::commands::Commands as _;
use craft_ui::runtime::overlay::CloseReason;
use craft_ui::sections::{DialogRole, FooterButton};
use egui::{Response, Ui};

/// A plain word of the dialog as an owned string.
macro_rules! word {
    ($env:expr, $field:ident) => {
        $env.text.words().$field.as_str().to_owned()
    };
}

/// A template word of the dialog filled from `(name, value)` pairs.
macro_rules! fill {
    ($env:expr, $field:ident, $($args:tt)*) => {
        $env.text.words().$field.fill(&[$($args)*])
    };
}

mod codec;
mod import_offer;
pub(crate) mod keyboard;
mod pages;
pub(crate) mod recorder;
mod review;
mod rows;
mod sets;
mod state;
#[cfg(feature = "suite")]
pub(crate) mod stories;
pub mod summary;
pub(crate) mod view;
mod words;

pub use codec::ChordCodec;
pub use import_offer::{ImportOffer, ImportOfferWords, NoOffer, OfferAnswer, OfferRecord};
pub use pages::{DialogSnapshot, DialogTab, ReviewCode, ReviewEntry, ReviewSnapshot};
pub use state::DialogState;
pub use words::{ShortcutWords, kys_error_text, trigger_text};

use state::Env;
use words::Say;

/// What the dialog reports for a pass.
#[derive(Clone, Debug)]
#[non_exhaustive]
pub enum Outcome {
    /// Closed with OK: save `Profiles` (with sets) or `keymap.overrides()`.
    Ok,
    /// Closed with Cancel: the keymap and profiles are back as they were when it opened. When that
    /// undid edits, `Changed` follows on the next frame.
    Cancel,
    /// An edit (for live menus and live saving). When a frame also has another outcome, such as
    /// `Import` after the Save changes prompt, `Changed` follows on the next frame.
    Changed,
    /// The user asked to import: the app opens a file of one of these formats and calls
    /// [`ShortcutsDialog::import_json`] or [`ShortcutsDialog::review_import`].
    #[non_exhaustive]
    Import {
        /// Ids of the formats the user may pick from.
        formats: Vec<&'static str>,
    },
    /// A set as craft-keymap JSON, for the app to write.
    #[non_exhaustive]
    Export {
        /// The set's name.
        name: String,
        /// The set as craft-keymap JSON.
        json: String,
    },
    /// A set in one of [`Options::export_formats`], for the app to write.
    #[non_exhaustive]
    ExportAs {
        /// The [`Format::id`] to write.
        format: &'static str,
        /// The set's name.
        name: String,
        /// The keymap with the set's overrides applied.
        keymap: Box<Keymap>,
    },
    /// The shortcut summary, for the app to show, save or print.
    #[non_exhaustive]
    Summary {
        /// The set's name.
        name: String,
        /// The summary as HTML.
        html: String,
        /// The summary as plain text.
        text: String,
    },
    /// Import answers the user gave, for the app to keep for later imports.
    #[non_exhaustive]
    Remembered {
        /// The target picked by remember key (`None`: No equivalent), feeding `ImportOptions::remembered`.
        targets: BTreeMap<String, Option<CommandRef>>,
        /// The answer for each undecided code by `key:N` / `mods:N`, feeding `ImportOptions::codes`.
        codes: BTreeMap<String, Option<String>>,
    },
}

/// What happens when a recorded chord is held by other commands.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub enum ConflictMode {
    /// Ask with the conflict bar: Take It, Accept and Go to Conflict, Keep Both, Cancel. Assign
    /// in the detail pane takes the chord without asking, as Steal does: the pane has already
    /// listed its holders under "Currently assigned to".
    #[default]
    Ask,
    /// Take the chord from the other commands, then say so.
    Steal,
    /// Keep both bindings, then say so.
    Keep,
}

/// An app's own Show entry.
#[derive(Clone, Copy, Debug)]
#[non_exhaustive]
pub struct Area {
    /// The entry's id; the dialog's `area` in `DialogSnapshot` holds it while selected.
    pub id: &'static str,
    /// The entry's text in the Show dropdown or tabs.
    pub label: &'static str,
    /// Whether a catalog entry is listed under this area.
    pub rows: fn(&CommandDef) -> bool,
}

impl Area {
    /// An entry with the id `id`, shown as `label`, listing the catalog entries `rows` returns true for.
    pub const fn new(id: &'static str, label: &'static str, rows: fn(&CommandDef) -> bool) -> Self {
        Area { id, label, rows }
    }
}

/// A read-only row for a key outside the catalog.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct FixedRow {
    /// The group the row is listed under.
    pub group: String,
    /// The row's text.
    pub label: String,
    /// The keys as shown.
    pub keys: String,
    /// The trigger, when the row has one, so a catalog binding on it can be pointed out.
    pub chord: Option<Trigger>,
}

impl FixedRow {
    /// A row under `group` that shows `label` with the keys `keys`.
    pub fn new(group: impl Into<String>, label: impl Into<String>, keys: impl Into<String>) -> Self {
        FixedRow { group: group.into(), label: label.into(), keys: keys.into(), chord: None }
    }

    /// The trigger of the row, so a catalog binding on it can be pointed out.
    #[must_use]
    pub fn chord(mut self, chord: Trigger) -> Self {
        self.chord = Some(chord);
        self
    }
}

/// A file format an app reads or writes itself.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct Format {
    /// The format's id, as [`Outcome::Import`] and [`Outcome::ExportAs`] report it.
    pub id: &'static str,
    /// The format's name as shown.
    pub label: &'static str,
    /// File extensions of the format, without the dot.
    pub extensions: &'static [&'static str],
}

impl Format {
    /// A format with the id `id`, shown as `label`, in files with the extensions `extensions`.
    pub const fn new(id: &'static str, label: &'static str, extensions: &'static [&'static str]) -> Self {
        Format { id, label, extensions }
    }
}

/// How the dialog arranges its body.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub enum Layout {
    /// The table of the catalog beside the detail pane.
    #[default]
    Table,
    /// Word's Customize Keyboard: a Categories list beside a Commands list, then the command's keys,
    /// the New Shortcut recorder, the holder line and the Assign and Remove buttons.
    CommandLists,
}

/// A rule an app adds for which bindings a command may get; `Err` carries the message shown.
pub type Accept = Box<dyn Fn(&CommandDef, &Binding) -> Result<(), String>>;

/// How the dialog looks and behaves. [`Options::default`] gives the defaults noted on each field;
/// set fields on it, or with [`Options::with`].
#[non_exhaustive]
pub struct Options {
    /// The window title [`ShortcutsDialog::show`] draws and the summary's heading, already translated.
    /// `None`: the dialog's own "Keyboard Shortcuts".
    pub title: Option<String>,
    /// `None`: an editable dialog opens at the room the window gives it, and a read-only one at a moderate fixed size.
    /// Otherwise the dialog opens at this size, in points, until the user resizes it; a size that is not finite is
    /// ignored.
    pub size: Option<egui::Vec2>,
    /// true; false shows the table read-only with Close and Summarize.
    pub editable: bool,
    /// The name, already translated, that edits of a built-in set are saved under. `None`: the dialog's own
    /// "Custom".
    pub custom_set_name: Option<String>,
    /// Ask.
    pub conflicts: ConflictMode,
    /// true: Keep Both is offered.
    pub allow_keep: bool,
    /// false: a recorded chord waits [`DOUBLE_PRESS`](craft_keymap::DOUBLE_PRESS) for a second press.
    pub double_press: bool,
    /// false: Backspace or Delete without modifiers removes the binding being recorded over.
    pub backspace_clears: bool,
    /// false: Enter closes nothing; true: Enter when idle is OK.
    pub enter_is_ok: bool,
    /// false: rows sort by translated label instead of catalog order.
    pub sort_by_label: bool,
    /// false: Show as tabs instead of a dropdown.
    pub area_tabs: bool,
    /// The app's own Show entries.
    pub areas: Vec<Area>,
    /// false: Show lists All Commands, each menu, Tools, each panel and Other before the app's own
    /// [`areas`](Self::areas); true: it lists only the app's own entries (and Fixed Keys), and the
    /// dialog opens on the first of them.
    pub own_areas: bool,
    /// `None`: every catalog entry is listed. Otherwise the table, its Show entries and search
    /// list only the entries it returns true for; the others keep their bindings and still count
    /// in conflicts.
    pub visible: Option<fn(&CommandDef) -> bool>,
    /// false: the on-screen keyboard pane.
    pub keyboard: bool,
    /// Read-only rows for keys outside the catalog.
    pub fixed: Vec<FixedRow>,
    /// Formats the app can read, besides the built-in ones.
    pub import_formats: Vec<Format>,
    /// Formats the app can write, besides craft-keymap JSON.
    pub export_formats: Vec<Format>,
    /// true: Summarize is offered.
    pub summary: bool,
    /// The app's extra rule for which bindings a command may get.
    pub accept: Option<Accept>,
    /// [`Layout::Table`]: how the body is arranged. The keymap, the `accept` rule, the conflict mode
    /// and undo are the same in every layout.
    pub layout: Layout,
}

impl Default for Options {
    fn default() -> Options {
        Options {
            title: None,
            size: None,
            editable: true,
            custom_set_name: None,
            conflicts: ConflictMode::Ask,
            allow_keep: true,
            double_press: false,
            backspace_clears: false,
            enter_is_ok: false,
            sort_by_label: false,
            area_tabs: false,
            areas: Vec::new(),
            own_areas: false,
            visible: None,
            keyboard: false,
            fixed: Vec::new(),
            import_formats: Vec::new(),
            export_formats: Vec::new(),
            summary: true,
            accept: None,
            layout: Layout::Table,
        }
    }
}

impl Options {
    /// `self` with `f` applied, for options built in one expression:
    /// `Options::default().with(|o| o.keyboard = true)`.
    pub fn with(mut self, f: impl FnOnce(&mut Options)) -> Options {
        f(&mut self);
        self
    }
}

/// The format id of craft-keymap's own JSON in [`Outcome::Import`].
pub const JSON_FORMAT: &str = "craft-keymap";

/// What [`ShortcutsDialog::show`] reports for a pass.
#[derive(Debug)]
#[non_exhaustive]
pub struct ShortcutsOutput {
    /// The dialog's response.
    pub response: Response,
    /// The pass's outcome: a closing or file outcome first, else `Changed` after an edit.
    pub outcome: Option<Outcome>,
}

impl craft_ui::contract::Outcome for ShortcutsOutput {
    type Value = NoValue;

    fn response(&self) -> &Response {
        &self.response
    }
}

/// The Hold Shortcuts setting the dialog shows and edits.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub struct HoldSetting {
    /// Whether hold shortcuts are on.
    pub on: bool,
    /// The delay, in seconds, before a held key counts as a hold.
    pub delay: f64,
    /// The delays the field accepts.
    pub range: RangeInclusive<f64>,
}

impl HoldSetting {
    /// Hold shortcuts `on`, holding after `delay` seconds, with the field accepting `range`.
    pub fn new(on: bool, delay: f64, range: RangeInclusive<f64>) -> Self {
        HoldSetting { on, delay, range }
    }
}

/// The dialog for one pass: what it edits, borrowed, and how it behaves.
#[must_use = "show the dialog with `show`"]
#[derive(Pattern)]
pub struct ShortcutsDialog<'a> {
    keymap: &'a mut Keymap,
    sets: Option<(&'a mut Profiles, &'a Builtins<'a>)>,
    options: Option<&'a Options>,
    tabs: Option<Box<dyn pages::TabSet + 'a>>,
    holds: Option<&'a mut HoldSetting>,
}

impl<'a> ShortcutsDialog<'a> {
    /// Shows the Hold Shortcuts switch, its delay and the explanation, and a Hold column recorded like the
    /// shortcut column. Cancel restores the setting; the app saves it on [`Outcome::Ok`].
    pub fn holds(mut self, holds: &'a mut HoldSetting) -> Self {
        self.holds = Some(holds);
        self
    }

    /// A dialog editing `keymap`; without further builder calls it has no sets and default options.
    pub fn new(keymap: &'a mut Keymap) -> Self {
        ShortcutsDialog { keymap, sets: None, options: None, tabs: None, holds: None }
    }

    /// Shows and edits named sets.
    pub fn sets(mut self, profiles: &'a mut Profiles, builtins: &'a Builtins<'a>) -> Self {
        self.sets = Some((profiles, builtins));
        self
    }

    /// Sets the value `field` names in `state`, as the user editing that control would
    /// (`search`, `area`, `command`, `pressed`; `assign` true presses Assign; `remove` takes the text
    /// of one of the command's keys, as [`DialogSnapshot::keys`] reports it), for automation. It builds the words `Env` that `DialogState::set_field` needs. `Err` says why
    /// the field was refused.
    pub fn set_json_field(self, ui: &Ui, state: &mut DialogState, field: &str, value: &serde_json::Value) -> Result<(), String> {
        use craft_ui::dialog::FieldRefusal;
        let rejects = || FieldRefusal::Rejects(value).message(field);
        let inactive = || FieldRefusal::Inactive.message(field);
        self.with_env(&Say::of(ui), |env| -> Result<(), String> {
            state.begin(env);
            match field {
                "search" => state.set_query(value.as_str().ok_or_else(rejects)?.to_owned()),
                "area" => {
                    let key = value.as_str().ok_or_else(rejects)?;
                    let area = state.areas.iter().find(|area| view::area_key(&area.id) == key).map(|area| area.id.clone()).ok_or_else(rejects)?;
                    state.set_area(area);
                }
                "command" => {
                    let command = value.as_str().and_then(CommandRef::parse_key).filter(|command| env.keymap.bindable(command).is_some());
                    state.select_row(&command.ok_or_else(rejects)?);
                }
                "pressed" => {
                    let trigger = Trigger::parse(value.as_str().ok_or_else(rejects)?).map_err(|_| rejects())?;
                    let command = state.selected.clone().ok_or_else(inactive)?;
                    state.set_field(env, command, trigger);
                }
                "assign" => match value {
                    serde_json::Value::Bool(true) => {
                        if state.field.is_none() {
                            return Err(inactive());
                        }
                        state.assign_field(env);
                    }
                    serde_json::Value::Bool(false) => {}
                    _ => return Err(rejects()),
                },
                "remove" => {
                    let wanted = Binding::parse(value.as_str().ok_or_else(rejects)?).map_err(|_| rejects())?;
                    let command = state.selected.clone().ok_or_else(inactive)?;
                    if !env.keymap.bindings(&command).contains(&wanted) {
                        return Err(rejects());
                    }
                    state.remove(env, &command, &wanted);
                }
                _ => return Err(FieldRefusal::Unknown.message(field)),
            }
            Ok(())
        })
    }

    /// Sets the dialog's [`Options`].
    pub fn options(mut self, options: &'a Options) -> Self {
        self.options = Some(options);
        self
    }

    fn with_env<R>(mut self, say: &Say, f: impl FnOnce(&mut Env<'_>) -> R) -> R {
        let fallback;
        let options = match self.options {
            Some(o) => o,
            None => {
                fallback = Options::default();
                &fallback
            }
        };
        let mut env =
            Env { keymap: self.keymap, sets: self.sets, options, tabs: self.tabs.as_deref_mut().map(pages::shorten), text: say, holds: self.holds };
        f(&mut env)
    }

    /// A modal, resizable dialog with the body, Undo, Redo, Reset All…, Cancel and OK; the window's close is
    /// Cancel. The dialog's place and size are remembered.
    pub fn show(self, ui: &mut Ui, state: &mut DialogState) -> ShortcutsOutput {
        let say = Say::of(ui);
        self.with_env(&say, |env| frame(ui, env, state))
    }

    /// OK, for hosts that draw their own button.
    pub fn ok(self, ui: &Ui, state: &mut DialogState) {
        self.with_env(&Say::of(ui), |env| {
            state.begin(env);
            state.ok(env);
        });
    }

    /// Cancel, for hosts that draw their own button.
    pub fn cancel(self, ui: &Ui, state: &mut DialogState) {
        self.with_env(&Say::of(ui), |env| {
            state.begin(env);
            state.cancel(env);
        });
    }

    /// Reads a craft-keymap JSON set the app loaded after [`Outcome::Import`]. With sets it becomes
    /// a new set, which is loaded; without, its overrides replace the keymap's as one undo step.
    /// Returns the set's name.
    pub fn import_json(self, ui: &Ui, state: &mut DialogState, text: &str) -> Result<String, ProfileError> {
        self.with_env(&Say::of(ui), |env| {
            state.begin(env);
            state.import_json(env, text)
        })
    }

    /// Opens the review of a foreign set the app read: what will change, what did not match and
    /// why, with suggestions to pick from.
    pub fn review_import(self, ui: &Ui, state: &mut DialogState, set: ForeignSet, options: ImportOptions) {
        self.with_env(&Say::of(ui), |env| {
            state.begin(env);
            state.review_import(env, set, options);
        });
    }
}

/// The width a read-only shortcuts dialog opens at, in points.
const READ_ONLY_WIDTH: f32 = 560.0;
/// The height a read-only shortcuts dialog opens at, in points.
const READ_ONLY_HEIGHT: f32 = 480.0;

/// What a footer button does.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Act {
    Undo,
    Redo,
    ResetAll,
    Summarize,
    Ok,
    Cancel,
    /// Close in [`Layout::CommandLists`]: it answers as OK.
    Close,
}

/// The footer's buttons and what each one does. Their enabled state is set after the body has run.
fn footer(env: &Env<'_>) -> (Vec<FooterButton>, Vec<Act>) {
    let mut buttons = Vec::new();
    let mut acts = Vec::new();
    let mut add = |act: Act, label: String, role: DialogRole, stays_open: bool| {
        buttons.push(FooterButton::new(label, role).stays_open(stays_open));
        acts.push(act);
    };
    let editable = env.options.editable;
    if env.options.layout == Layout::CommandLists {
        if editable {
            add(Act::ResetAll, word!(env, reset_all_more), DialogRole::Other, true);
        }
        add(Act::Close, word!(env, close), DialogRole::Accept, false);
        return (buttons, acts);
    }
    if editable {
        add(Act::Undo, word!(env, undo), DialogRole::Other, true);
        add(Act::Redo, word!(env, redo), DialogRole::Other, true);
        add(Act::ResetAll, word!(env, reset_all_more), DialogRole::Other, true);
    }
    // With sets, Summarize… is in the set row, which a read-only dialog does not show.
    if (env.sets.is_none() || !editable) && env.options.summary {
        add(Act::Summarize, word!(env, summarize_more), DialogRole::Other, true);
    }
    if editable {
        add(Act::Cancel, word!(env, cancel), DialogRole::Cancel, false);
        add(Act::Ok, word!(env, ok), DialogRole::Accept, false);
    } else {
        add(Act::Cancel, word!(env, close), DialogRole::Cancel, false);
    }
    (buttons, acts)
}

/// What the dialog's regions share during one pass: the environment and the state.
struct Session<'s, 'e> {
    env: &'s mut Env<'e>,
    state: &'s mut DialogState,
}

/// One pass: take the keys the body does not, show the dialog, close it as the user asked, report.
fn frame(ui: &mut Ui, env: &mut Env<'_>, state: &mut DialogState) -> ShortcutsOutput {
    state.begin(env);
    if !state.is_showing() {
        return ShortcutsOutput { response: ui.response(), outcome: state.finish() };
    }
    let (buttons, acts) = footer(env);
    let enter = env.options.enter_is_ok && !state.modal() && !state.is_recording();
    let title = env.title();
    let mut open = true;
    // A read-only reference opens at a fixed, moderate height (production's key list scrolls inside it), not at the
    // window's; the user's own resize is still remembered.
    let reference = !env.options.editable;
    let asked = env.options.size.filter(|size| size.is_finite());
    let size = asked.or(reference.then_some(egui::vec2(READ_ONLY_WIDTH, READ_ONLY_HEIGHT)));
    let session = RefCell::new(Session { env, state });
    let shown = Dialog::new(title)
        .resizable(true)
        .overlay(|overlay| match size {
            Some(size) => overlay.size(size),
            None => overlay,
        })
        .remember(SaveKey::new("craft_menu.shortcuts"))
        .outer_gesture(true)
        .collection_body()
        .close_on_enter(enter)
        .buttons(buttons)
        .settle_buttons(|_, buttons| {
            let guard = session.borrow();
            let (env, state) = (&*guard.env, &*guard.state);
            let idle = !state.modal();
            for (button, act) in buttons.iter_mut().zip(&acts) {
                button.set_enabled(match act {
                    Act::Undo => idle && state.can_undo(),
                    Act::Redo => idle && state.can_redo(),
                    Act::ResetAll => idle && !env.keymap.overrides().is_empty(),
                    Act::Ok | Act::Close => idle,
                    Act::Cancel => idle,
                    Act::Summarize => true,
                });
            }
        })
        // A footer button does not identify itself to the dialog's hooks, so the picked index is read after the
        // dialog ran. OK is the only button that closes: the state decides whether it may, so the dialog itself
        // never closes on it.
        .close_guard(|_, _| false)
        .opened(Bind::bound(&mut open))
        .show(ui, |ui, cx| {
            let mut guard = session.borrow_mut();
            let Session { env, state } = &mut *guard;
            while let Some(step) = cx.take_step(!state.modal() && state.can_undo(), !state.modal() && state.can_redo()) {
                match step {
                    Step::Undo => state.undo(env),
                    Step::Redo => state.redo(env),
                    _ => {}
                }
            }
            view::body(ui, cx, env, state);
            if state.change_pending() {
                cx.wake_at(cx.now());
            }
        });
    let Session { env, state } = session.into_inner();
    let DialogOutput { response, closed, picked, .. } = shown.output;
    match picked.and_then(|at| acts.get(at)) {
        Some(Act::Undo) => state.undo(env),
        Some(Act::Redo) => state.redo(env),
        Some(Act::ResetAll) => state.ask_reset_all(),
        Some(Act::Summarize) => state.summarize(env),
        Some(Act::Ok | Act::Close) => state.ok(env),
        Some(Act::Cancel) | None => {}
    }
    match closed {
        Some(CloseReason::Escape) => state.escape(env),
        Some(CloseReason::Requested | CloseReason::Outside | CloseReason::FocusLeft(_)) => state.cancel(env),
        _ => {}
    }
    state.sync_set(env);
    ShortcutsOutput { response, outcome: state.finish() }
}
