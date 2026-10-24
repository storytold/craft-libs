//! The Keyboard Shortcuts dialog (feature `ui`): a table of the catalog with recording,
//! conflicts, undo, named sets and import review, over [`Keymap`] and optionally [`Profiles`].
//!
//! The app keeps a [`DialogState`] while the dialog is open and calls [`ShortcutsDialog::show`]
//! (or [`ShortcutsDialog::ui`] inside its own container) every frame. While
//! [`DialogState::is_open`] is true the app skips its own key dispatch; the dialog takes the
//! frame's key events itself while it records. The app saves on [`Outcome::Ok`] (or on
//! [`Outcome::Changed`] for live saving) and drops the state once it is closed.

use std::collections::BTreeMap;

use crate::chord::Trigger;
use crate::command::CommandRef;
use crate::import::{ForeignSet, ImportOptions};
use crate::keymap::{Bindable, Binding, Keymap};
use crate::profiles::{Builtins, ProfileError, Profiles};

mod capture;
mod rows;
mod state;
mod style;
pub mod summary;
mod text;
mod view;

pub use state::DialogState;
pub use style::DialogStyle;
pub use text::{English, I18nText, PLURALS, STRINGS, Text, trigger_text};

use state::Env;

/// What the dialog reports for a frame.
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
    Import { formats: Vec<&'static str> },
    /// A set as craft-keymap JSON, for the app to write.
    Export { name: String, json: String },
    /// A set in one of [`Options::export_formats`], for the app to write.
    ExportAs { format: &'static str, name: String, keymap: Box<Keymap> },
    /// The shortcut summary, for the app to show, save or print.
    Summary { name: String, html: String, text: String },
    /// Import answers the user gave (item → command), for the app to keep for later imports.
    Remembered(BTreeMap<String, CommandRef>),
}

/// What happens when a recorded chord is held by other commands.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub enum ConflictMode {
    /// Ask with the conflict bar: Take It, Take It and Go to Conflict, Keep Both, Cancel.
    #[default]
    Ask,
    /// Take the chord from the other commands, then say so.
    Steal,
    /// Keep both bindings, then say so.
    Keep,
}

/// An app's own Show entry.
#[derive(Clone, Copy, Debug)]
pub struct Area {
    pub id: &'static str,
    pub label: &'static str,
    pub rows: fn(&Bindable) -> bool,
}

/// A read-only row for a key outside the catalog.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FixedRow {
    pub group: String,
    pub label: String,
    /// The keys as shown.
    pub keys: String,
    /// The trigger, when the row has one, so a catalog binding on it can be pointed out.
    pub chord: Option<Trigger>,
}

/// A file format an app reads or writes itself.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Format {
    pub id: &'static str,
    pub label: &'static str,
    pub extensions: &'static [&'static str],
}

/// A rule an app adds for which bindings a command may get; `Err` carries the message shown.
pub type Accept = Box<dyn Fn(&Bindable, &Binding) -> Result<(), String>>;
/// Called for every control once per frame with (automation id, rect, label).
pub type OnWidget = Box<dyn Fn(&str, egui::Rect, &str)>;

/// How the dialog looks and behaves. [`Options::default`] gives the defaults noted on each field;
/// set fields on it, or with [`Options::with`].
#[non_exhaustive]
pub struct Options {
    /// "Keyboard Shortcuts": the window title [`ShortcutsDialog::show`] draws, and the summary's
    /// heading. [`ShortcutsDialog::ui`] draws no title; the host's container shows its own.
    pub title: &'static str,
    /// true; false shows the table read-only with Close and Summarize.
    pub editable: bool,
    /// "Custom": the name edits of a built-in set are saved under.
    pub custom_set_name: &'static str,
    /// Ask.
    pub conflicts: ConflictMode,
    /// true: Keep Both is offered.
    pub allow_keep: bool,
    /// false: a recorded chord waits [`DOUBLE_PRESS`](crate::DOUBLE_PRESS) for a second press.
    pub double_press: bool,
    /// false: Backspace or Delete without modifiers removes the binding being recorded over.
    pub backspace_clears: bool,
    /// false: Enter when idle is OK.
    pub enter_is_ok: bool,
    /// false: rows sort by translated label instead of catalog order.
    pub sort_by_label: bool,
    /// false: Show as tabs instead of a dropdown.
    pub area_tabs: bool,
    /// The app's own Show entries.
    pub areas: Vec<Area>,
    /// `None`: every catalog entry is listed. Otherwise the table, its Show entries and search
    /// list only the entries it returns true for; the others keep their bindings and still count
    /// in conflicts.
    pub visible: Option<fn(&Bindable) -> bool>,
    /// false: the on-screen keyboard pane.
    pub keyboard: bool,
    pub fixed: Vec<FixedRow>,
    pub import_formats: Vec<Format>,
    pub export_formats: Vec<Format>,
    /// true: Summarize is offered.
    pub summary: bool,
    pub accept: Option<Accept>,
    pub on_widget: Option<OnWidget>,
    /// `None`: [`DialogStyle::from_visuals`].
    pub style: Option<DialogStyle>,
}

impl Default for Options {
    fn default() -> Options {
        Options {
            title: "Keyboard Shortcuts",
            editable: true,
            custom_set_name: "Custom",
            conflicts: ConflictMode::Ask,
            allow_keep: true,
            double_press: false,
            backspace_clears: false,
            enter_is_ok: false,
            sort_by_label: false,
            area_tabs: false,
            areas: Vec::new(),
            visible: None,
            keyboard: false,
            fixed: Vec::new(),
            import_formats: Vec::new(),
            export_formats: Vec::new(),
            summary: true,
            accept: None,
            on_widget: None,
            style: None,
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

/// The dialog for one frame: what it edits, borrowed, and how it looks.
pub struct ShortcutsDialog<'a> {
    keymap: &'a mut Keymap,
    text: &'a dyn Text,
    sets: Option<(&'a mut Profiles, &'a Builtins<'a>)>,
    options: Option<&'a Options>,
}

impl<'a> ShortcutsDialog<'a> {
    pub fn new(keymap: &'a mut Keymap, text: &'a dyn Text) -> Self {
        ShortcutsDialog { keymap, text, sets: None, options: None }
    }

    /// Shows and edits named sets.
    pub fn sets(mut self, profiles: &'a mut Profiles, builtins: &'a Builtins<'a>) -> Self {
        self.sets = Some((profiles, builtins));
        self
    }

    pub fn options(mut self, options: &'a Options) -> Self {
        self.options = Some(options);
        self
    }

    fn with_env<R>(self, f: impl FnOnce(&mut Env) -> R) -> R {
        let fallback;
        let options = match self.options {
            Some(o) => o,
            None => {
                fallback = Options::default();
                &fallback
            }
        };
        let mut env = Env { keymap: self.keymap, sets: self.sets, options, text: self.text };
        f(&mut env)
    }

    /// A window with title, body and Cancel/OK, sized to the screen (at least 640×480 where the
    /// screen allows, at most 1320×900). Closing it with × is Cancel.
    pub fn show(self, ctx: &egui::Context, state: &mut DialogState) -> Option<Outcome> {
        self.with_env(|env| {
            if !state.is_showing() {
                return state.finish();
            }
            let title = env.text.visual(&env.text.ui(env.options.title));
            let screen = ctx.content_rect().size();
            let max = egui::vec2(1320.0, 900.0).min(screen - egui::vec2(16.0, 16.0)).max(egui::vec2(240.0, 180.0));
            let min = egui::vec2(640.0, 480.0).min(max);
            let mut open = true;
            let mut out = None;
            egui::Window::new(title)
                .id(egui::Id::new("craft_keymap.shortcuts"))
                .collapsible(false)
                .resizable(true)
                .open(&mut open)
                .default_size((screen * 0.8).clamp(min, max))
                .min_size(min)
                .max_size(max)
                .show(ctx, |ui| out = frame(ui, env, state));
            if !open && state.is_showing() {
                state.cancel(env);
                out = state.finish();
            }
            if state.change_pending() {
                ctx.request_repaint();
            }
            out
        })
    }

    /// The body only, for a host's own modal, preferences page or backstage, which draws the
    /// title. It fills the available rectangle.
    pub fn ui(self, ui: &mut egui::Ui, state: &mut DialogState) -> Option<Outcome> {
        self.with_env(|env| frame(ui, env, state))
    }

    /// OK, for hosts that draw their own button.
    pub fn ok(self, state: &mut DialogState) {
        self.with_env(|env| {
            state.begin(env);
            state.ok(env);
        });
    }

    /// Cancel, for hosts that draw their own button.
    pub fn cancel(self, state: &mut DialogState) {
        self.with_env(|env| {
            state.begin(env);
            state.cancel(env);
        });
    }

    /// Reads a craft-keymap JSON set the app loaded after [`Outcome::Import`]. With sets it becomes
    /// a new set, which is loaded; without, its overrides replace the keymap's as one undo step.
    /// Returns the set's name.
    pub fn import_json(self, state: &mut DialogState, text: &str) -> Result<String, ProfileError> {
        self.with_env(|env| {
            state.begin(env);
            state.import_json(env, text)
        })
    }

    /// Opens the review of a foreign set the app read: what will change, what did not match and
    /// why, with suggestions to pick from.
    pub fn review_import(self, state: &mut DialogState, set: ForeignSet, options: ImportOptions) {
        self.with_env(|env| {
            state.begin(env);
            state.review_import(env, set, options);
        });
    }
}

/// One frame: take recorded keys before anything else sees them, draw, report.
fn frame(ui: &mut egui::Ui, env: &mut Env, state: &mut DialogState) -> Option<Outcome> {
    state.begin(env);
    state.input(env, ui.ctx());
    if state.is_showing() {
        view::body(ui, env, state);
    }
    let out = state.finish();
    if state.change_pending() {
        ui.ctx().request_repaint();
    }
    out
}
