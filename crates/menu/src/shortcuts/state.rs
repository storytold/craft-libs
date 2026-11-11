//! The dialog's persistent state and every action it takes, as calls on [`EditSession`],
//! [`Recorder`], [`editor::holders`] and [`Profiles`].

use std::collections::BTreeSet;

use craft_keymap::editor::{self, DOUBLE_PRESS, EditSession, Recorded, Recorder};
use craft_keymap::import::{self, ForeignSet, ImportOptions, ImportPlan, ImportReport};
use craft_keymap::profiles::{Builtins, ProfileError, Profiles, set_name};
use craft_keymap::{
    AssignError, Assigned, Binding, BindingError, Chord, CommandRef, Holder, Key, Keymap, MAX_BINDINGS, Overrides, ParseError, Policy, Scope, Trigger,
};

use super::keyboard::KeyboardState;
use super::pages::TabSet;
use super::rows::{self, AreaEntry, AreaId, Built, Filter, RowCache, View};
use super::words::Say;
use super::{ConflictMode, JSON_FORMAT, Options, Outcome, summary};

/// What the dialog edits this frame.
pub(crate) struct Env<'e> {
    pub keymap: &'e mut Keymap,
    pub sets: Option<(&'e mut Profiles, &'e Builtins<'e>)>,
    pub options: &'e Options,
    pub tabs: Option<&'e mut dyn TabSet>,
    pub text: &'e Say,
}

impl Env<'_> {
    fn chord_text(&self, t: &Trigger) -> String {
        self.text.trigger(t, self.keymap.platform())
    }

    fn label(&self, command: &CommandRef) -> String {
        self.keymap.bindable(command).map_or_else(|| command.id.clone(), |b| self.text.command(b))
    }

    /// `label (Scope)` for each holder, joined.
    fn holder_labels(&self, holders: &[Holder]) -> String {
        let mut out: Vec<String> = Vec::new();
        for h in holders {
            let label = self.label(&h.command);
            let s = match &h.binding.scope {
                Some(scope) => fill!(self, label_scope, ("label", &label), ("scope", &self.text.segment(scope.as_str()))),
                None => label,
            };
            if !out.contains(&s) {
                out.push(s);
            }
        }
        out.join(", ")
    }

    /// The window title and the summary's heading: [`Options::title`], else the dialog's own words.
    pub(crate) fn title(&self) -> String {
        self.options.title.clone().unwrap_or_else(|| self.text.words().keyboard_shortcuts.as_str().to_owned())
    }

    /// The name edits of a built-in set are saved under: [`Options::custom_set_name`], else the dialog's own words.
    pub(crate) fn custom_set_name(&self) -> String {
        self.options.custom_set_name.clone().unwrap_or_else(|| self.text.words().custom.as_str().to_owned())
    }
}

/// Where a recording goes.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Target {
    /// A row's shortcut cell: replaces `old`, or adds when `None`. Commits at once.
    Cell { command: CommandRef, old: Option<Binding> },
    /// The detail pane's New Shortcut field: the chord waits for Assign.
    Field { command: CommandRef },
    /// Search by key: the chord's text becomes the query.
    Search,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Recording {
    pub target: Target,
    /// With [`Options::double_press`]: a single press waiting [`DOUBLE_PRESS`] for its second press.
    pub waiting: Option<(Trigger, f64)>,
    /// The recorder field has been shown since the recording began, so it took key focus.
    pub started: bool,
}

/// The New Shortcut field's chord, waiting for Assign.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct FieldChord {
    pub command: CommandRef,
    pub binding: Binding,
    /// Commands whose bindings would clash ("Currently assigned to").
    pub holders: Vec<Holder>,
    /// Commands with the same trigger in another scope that do not clash ("both work").
    pub shadows: Vec<Holder>,
}

/// The conflict bar, for one pending chord.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct ConflictBar {
    pub command: CommandRef,
    pub old: Option<Binding>,
    pub binding: Binding,
    pub holders: Vec<Holder>,
    /// "{chord} is used by {label}".
    pub text: String,
}

/// A line under the table after an action.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Message {
    pub text: String,
    /// Nothing changed: shown as a warning.
    pub refused: bool,
    /// Offers Go to Conflict.
    pub goto: Option<CommandRef>,
    /// Offers Undo.
    pub undo: bool,
    /// The chord text inside `text`, which stays where it is when the text around it changes sides.
    pub chord: Option<String>,
}

/// What to do after the Save changes prompt.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Then {
    Switch(String),
    NewSet { name: String, based_on: Option<String> },
    Import,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Prompt {
    /// "Restore every default shortcut in this set?"
    ResetAll,
    /// "Delete the set “{set}”?"
    DeleteSet(String),
    /// "Save changes to “{set}”?" with Save, Don't Save and Cancel.
    SaveChanges { set: String, then: Then },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Answer {
    /// Save, or confirm.
    Yes,
    /// Don't Save.
    No,
    Cancel,
}

/// A foreign set under review.
#[derive(Clone, Debug)]
pub(crate) struct Review {
    pub set: ForeignSet,
    pub options: ImportOptions,
    pub plan: ImportPlan,
    /// The new set's name.
    pub name: String,
    /// After Import as New Set: the set's name and the report.
    pub done: Option<(String, ImportReport)>,
}

/// The set row's inline form: New Set… or Rename….
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct SetForm {
    pub kind: SetFormKind,
    pub name: String,
    /// The name field takes key focus on the form's first frame.
    pub focus: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) enum SetFormKind {
    /// `None`: based on the current shortcuts.
    New {
        based_on: Option<String>,
    },
    Rename,
}

#[derive(Clone, Debug)]
struct Opened {
    profiles: Option<Profiles>,
    overrides: Overrides,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Phase {
    New,
    Open,
    Closed,
}

/// The dialog's state between frames. Keep it while [`is_open`](Self::is_open) is true.
#[derive(Clone, Debug)]
pub struct DialogState {
    phase: Phase,
    opened: Option<Opened>,
    session: Option<EditSession>,
    recorder: Recorder,
    pub(crate) selected: Option<CommandRef>,
    /// Scroll the selected row into view on the next frame.
    pub(crate) reveal: bool,
    pub(crate) areas: Vec<AreaEntry>,
    pub(crate) area: AreaId,
    pub(crate) filter: Filter,
    pub(crate) query: String,
    pub(crate) collapsed: BTreeSet<String>,
    /// The detail pane's Context: `None` is the command's home scope.
    pub(crate) context: Option<Scope>,
    pub(crate) recording: Option<Recording>,
    pub(crate) field: Option<FieldChord>,
    pub(crate) conflict: Option<ConflictBar>,
    pub(crate) prompt: Option<Prompt>,
    pub(crate) message: Option<Message>,
    pub(crate) review: Option<Review>,
    rows: RowCache,
    changed: bool,
    outcome: Option<Outcome>,
    // ---- The view's own state ----
    /// The chip of the selected row that Enter and Delete act on.
    pub(crate) chip: usize,
    /// Typed entry in the detail pane, and the parse error shown under it.
    pub(crate) typing: Option<String>,
    /// The typed-entry field takes key focus on its first pass.
    pub(crate) typing_focus: bool,
    pub(crate) type_error: Option<String>,
    pub(crate) set_form: Option<SetForm>,
    pub(crate) keyboard: KeyboardState,
    /// The app tab shown (its [`DialogTab::id`]); `None` is Keyboard Shortcuts.
    pub(crate) tab: Option<&'static str>,
    /// The active set's name as of the last frame.
    pub(crate) set: Option<String>,
}

impl Default for DialogState {
    fn default() -> DialogState {
        DialogState::new()
    }
}

impl DialogState {
    /// A dialog about to open; it takes its snapshot on the first frame.
    pub fn new() -> DialogState {
        DialogState {
            phase: Phase::New,
            opened: None,
            session: None,
            recorder: Recorder::new(),
            selected: None,
            reveal: false,
            areas: Vec::new(),
            area: AreaId::All,
            filter: Filter::All,
            query: String::new(),
            collapsed: BTreeSet::new(),
            context: None,
            recording: None,
            field: None,
            conflict: None,
            prompt: None,
            message: None,
            review: None,
            rows: RowCache::default(),
            changed: false,
            outcome: None,
            chip: 0,
            typing: None,
            typing_focus: false,
            type_error: None,
            set_form: None,
            keyboard: KeyboardState::default(),
            tab: None,
            set: None,
        }
    }

    /// Whether the first frame has run.
    pub(crate) fn begun(&self) -> bool {
        self.phase != Phase::New
    }

    /// Records the active set's name for [`DialogState::snapshot`].
    pub(crate) fn sync_set(&mut self, env: &Env) {
        self.set = env.sets.as_ref().map(|(p, b)| p.active_name(b).to_string());
    }

    /// Shows an app tab (`None`: Keyboard Shortcuts); a chord being recorded is dropped.
    pub(crate) fn set_tab(&mut self, tab: Option<&'static str>) {
        if self.tab != tab {
            self.stop_recording();
            self.tab = tab;
        }
    }

    /// An app tab's body changed something.
    pub(crate) fn app_tab_changed(&mut self) {
        self.changed = true;
    }

    /// False once the dialog has closed with OK or Cancel and reported its last outcome: drop the
    /// state then. After a Cancel that undid edits it stays true for the frame that reports
    /// [`Outcome::Changed`].
    pub fn is_open(&self) -> bool {
        self.phase != Phase::Closed || self.changed
    }

    /// Whether the dialog is drawn: not yet closed with OK or Cancel.
    pub(crate) fn is_showing(&self) -> bool {
        self.phase != Phase::Closed
    }

    /// While true the app also pauses native menu key equivalents.
    pub fn is_recording(&self) -> bool {
        self.recording.is_some()
    }

    /// Selects a command and scrolls to it, as when the dialog opens from "Edit Shortcut…".
    pub fn select(&mut self, command: &CommandRef) {
        self.selected = Some(command.clone());
        self.reveal = true;
        self.context = None;
        self.field = None;
    }

    /// Shows `text`, already translated, as a warning under the table until the user dismisses it
    /// or the next action's message replaces it: for the app's own errors while the dialog is
    /// open, such as an import file it could not read.
    pub fn show_error(&mut self, text: impl Into<String>) {
        self.message = Some(Message { text: text.into(), refused: true, goto: None, undo: false, chord: None });
    }

    // ---- Frame ----

    /// The first frame: snapshot for Cancel, an edit session, the Show entries.
    pub(crate) fn begin(&mut self, env: &mut Env) {
        if self.phase != Phase::New {
            return;
        }
        self.phase = Phase::Open;
        self.opened = Some(Opened { profiles: env.sets.as_ref().map(|(p, _)| (**p).clone()), overrides: env.keymap.overrides().clone() });
        self.session = Some(EditSession::begin(env.keymap));
        self.areas = rows::areas(env.keymap, env.options, env.text);
        self.sync_set(env);
    }

    /// When a single press waiting for its second press commits, for the view to ask a repaint at.
    pub(crate) fn wake(&self) -> Option<f64> {
        self.recording.as_ref().and_then(|r| r.waiting).map(|(_, at)| at + DOUBLE_PRESS)
    }

    /// The frame's outcome: a closing or file outcome first, else Changed after an edit. An edit
    /// that comes with another outcome stays pending and is reported by the next call.
    pub(crate) fn finish(&mut self) -> Option<Outcome> {
        self.outcome.take().or_else(|| std::mem::take(&mut self.changed).then_some(Outcome::Changed))
    }

    /// Whether an edit waits for the next [`finish`](Self::finish).
    pub(crate) fn change_pending(&self) -> bool {
        self.changed
    }

    /// The table's lines for the current view.
    pub(crate) fn lines(&mut self, env: &Env) -> &Built {
        let view = View { query: &self.query, area: &self.area, filter: self.filter, collapsed: &self.collapsed };
        self.rows.get(env.keymap, env.options, env.text, &view)
    }

    // ---- Selection and view ----

    pub(crate) fn select_row(&mut self, command: &CommandRef) {
        if self.selected.as_ref() != Some(command) {
            self.selected = Some(command.clone());
            self.context = None;
            self.field = None;
            self.chip = 0;
            self.typing = None;
            self.type_error = None;
        }
    }

    pub(crate) fn set_area(&mut self, area: AreaId) {
        self.area = area;
    }

    pub(crate) fn set_filter(&mut self, filter: Filter) {
        self.filter = filter;
    }

    pub(crate) fn set_query(&mut self, query: String) {
        self.query = query;
    }

    pub(crate) fn toggle_group(&mut self, name: &str) {
        if !self.collapsed.remove(name) {
            self.collapsed.insert(name.to_string());
        }
    }

    /// Selects a command and shows its row: area All, no search, no filter.
    pub(crate) fn go_to(&mut self, command: &CommandRef) {
        self.select_row(command);
        self.area = AreaId::All;
        self.query.clear();
        self.filter = Filter::All;
        self.collapsed.clear();
        self.reveal = true;
    }

    /// The Context entries for a command: its home scope (`None`, shown as Default), Application,
    /// every panel and every context.
    pub(crate) fn contexts(env: &Env, command: &CommandRef) -> Vec<Option<Scope>> {
        let home = env.keymap.bindable(command).map(|b| b.scope.clone());
        let scopes = env.keymap.scopes();
        let mut out = vec![None];
        let names = std::iter::once(Scope::app()).chain(scopes.panels().iter().chain(scopes.contexts()).filter_map(|s| Scope::new(s.as_str())));
        for s in names {
            if Some(&s) != home.as_ref() && !out.contains(&Some(s.clone())) {
                out.push(Some(s));
            }
        }
        out
    }

    pub(crate) fn set_context(&mut self, env: &Env, scope: Option<Scope>) {
        self.context = scope;
        if let Some(f) = self.field.take() {
            self.set_field(env, f.command, f.binding.trigger);
        }
    }

    // ---- Recording ----

    /// Starts recording; drops the conflict bar and any message.
    pub(crate) fn record(&mut self, env: &Env, target: Target) {
        if !env.options.editable && target != Target::Search {
            return;
        }
        self.conflict = None;
        self.message = None;
        self.recorder.reset();
        if let Target::Cell { command, .. } | Target::Field { command } = &target {
            self.select_row(command);
        }
        self.recording = Some(Recording { target, waiting: None, started: false });
    }

    pub(crate) fn stop_recording(&mut self) {
        self.recording = None;
        self.recorder.reset();
    }

    /// Commits a single press whose double-press window has ended.
    pub(crate) fn tick(&mut self, env: &mut Env, now: f64) {
        let expired = self.recording.as_ref().and_then(|r| r.waiting).filter(|(_, at)| now < *at || now - *at >= DOUBLE_PRESS);
        if let Some((trigger, _)) = expired {
            self.commit_recorded(env, trigger);
        }
    }

    /// Feeds captured presses to the recorder.
    pub(crate) fn feed(&mut self, env: &mut Env, chords: &[Chord], now: f64) {
        for &chord in chords {
            self.tick(env, now);
            let Some(rec) = &mut self.recording else { return };
            if env.options.backspace_clears && chord.mods.is_none() && matches!(chord.key, Key::Backspace | Key::Delete) {
                let target = rec.target.clone();
                self.stop_recording();
                match target {
                    Target::Cell { command, old: Some(old) } => self.remove(env, &command, &old),
                    Target::Field { .. } => self.field = None,
                    _ => {}
                }
                continue;
            }
            match self.recorder.press(chord, now) {
                Recorded::Cancel => self.stop_recording(),
                Recorded::Single(t) if env.options.double_press => rec.waiting = Some((t, now)),
                Recorded::Single(t) | Recorded::Double(t) => self.commit_recorded(env, t),
            }
        }
    }

    /// The recorded trigger is final: ends the recording and sends it where it belongs.
    fn commit_recorded(&mut self, env: &mut Env, trigger: Trigger) {
        let Some(rec) = self.recording.take() else { return };
        self.recorder.reset();
        match rec.target {
            Target::Search => self.query = env.chord_text(&trigger),
            Target::Cell { command, old } => {
                let binding = match old.as_ref().and_then(|o| o.scope.clone()) {
                    Some(scope) => Binding::in_scope(trigger, scope),
                    None => Binding::new(trigger),
                };
                self.commit(env, &command, old, binding);
            }
            Target::Field { command } => self.set_field(env, command, trigger),
        }
    }

    /// Puts a chord into the New Shortcut field with the chosen Context, and finds who holds it.
    pub(crate) fn set_field(&mut self, env: &Env, command: CommandRef, trigger: Trigger) {
        let binding = match &self.context {
            Some(scope) => Binding::in_scope(trigger, scope.clone()),
            None => Binding::new(trigger),
        };
        let holders = editor::holders(env.keymap, &command, &binding);
        let platform = env.keymap.platform();
        let want = trigger.effective(platform);
        let mut shadows = Vec::new();
        for b in env.keymap.catalog() {
            if b.command == command {
                continue;
            }
            for x in env.keymap.bindings(&b.command) {
                if x.trigger.effective(platform) == want && !holders.iter().any(|h| h.command == b.command && h.binding == x) {
                    shadows.push(Holder { command: b.command.clone(), binding: x });
                }
            }
        }
        self.field = Some(FieldChord { command, binding, holders, shadows });
    }

    /// Assign in the detail pane: adds the field's chord to the command. The pane already lists
    /// its holders under "Currently assigned to", so Assign takes it from them without the
    /// conflict bar (one undo step), or keeps both with [`ConflictMode::Keep`].
    pub(crate) fn assign_field(&mut self, env: &mut Env) {
        if let Some(f) = self.field.take() {
            let mode = if env.options.conflicts == ConflictMode::Keep { ConflictMode::Keep } else { ConflictMode::Steal };
            self.commit_with(env, &f.command, None, f.binding, mode);
        }
    }

    /// Typed entry: the text as [`Binding::parse`] reads it, committed like a recorded chord.
    /// `Err` is the parse error for under the field, translated.
    pub(crate) fn type_binding(&mut self, env: &mut Env, command: &CommandRef, text: &str) -> Result<(), String> {
        let binding = Binding::parse(text).map_err(|e| binding_error(env, &e))?;
        self.select_row(command);
        if let Some(scope) = binding.scope.clone() {
            self.context = Some(scope);
        }
        self.set_field(env, command.clone(), binding.trigger);
        Ok(())
    }

    // ---- Commit and conflicts ----

    /// Reserved chord, then the app's rule, then holders: assign at once, or by
    /// [`Options::conflicts`].
    pub(crate) fn commit(&mut self, env: &mut Env, command: &CommandRef, old: Option<Binding>, binding: Binding) {
        let mode = env.options.conflicts;
        self.commit_with(env, command, old, binding, mode);
    }

    fn commit_with(&mut self, env: &mut Env, command: &CommandRef, old: Option<Binding>, binding: Binding, mode: ConflictMode) {
        self.conflict = None;
        self.message = None;
        let chord = env.chord_text(&binding.trigger);
        if env.keymap.is_reserved(&binding.trigger.chord) {
            self.refuse_chord(fill!(env, chord_is_reserved_by_the_system, ("chord", &chord)), chord);
            return;
        }
        if let (Some(accept), Some(b)) = (&env.options.accept, env.keymap.bindable(command))
            && let Err(text) = accept(b, &binding)
        {
            self.refuse(text);
            return;
        }
        let holders = editor::holders(env.keymap, command, &binding);
        if holders.is_empty() {
            self.apply(env, command, old, binding, Policy::Steal);
            return;
        }
        match mode {
            ConflictMode::Ask => {
                let text = fill!(env, chord_is_used_by_label, ("chord", &chord), ("label", &env.holder_labels(&holders)));
                self.conflict = Some(ConflictBar { command: command.clone(), old, binding, holders, text });
            }
            ConflictMode::Steal => self.apply_and_say(env, command, old, binding, Policy::Steal),
            ConflictMode::Keep => self.apply_and_say(env, command, old, binding, Policy::Keep),
        }
    }

    fn refuse(&mut self, text: String) {
        self.message = Some(Message { text, refused: true, goto: None, undo: false, chord: None });
    }

    fn refuse_chord(&mut self, text: String, chord: String) {
        self.message = Some(Message { text, refused: true, goto: None, undo: false, chord: Some(chord) });
    }

    fn apply(&mut self, env: &mut Env, command: &CommandRef, old: Option<Binding>, binding: Binding, policy: Policy) -> Option<Assigned> {
        let chord = env.chord_text(&binding.trigger);
        let km = &mut *env.keymap;
        let session = self.session.get_or_insert_with(|| EditSession::begin(km));
        let result = match &old {
            Some(o) => session.replace(km, command, o, binding, policy),
            None => session.assign(km, command, binding, policy),
        };
        match result {
            Ok(a) => {
                self.edited(env);
                Some(a)
            }
            Err(e) => {
                let text = assign_error(env, &e);
                self.refuse_chord(text, chord);
                None
            }
        }
    }

    /// Applies with Steal or Keep and says what it did, with Go to Conflict (and Undo for Steal).
    fn apply_and_say(&mut self, env: &mut Env, command: &CommandRef, old: Option<Binding>, binding: Binding, policy: Policy) {
        let chord = env.chord_text(&binding.trigger);
        let Some(a) = self.apply(env, command, old, binding, policy) else { return };
        let (holders, steal) = if policy == Policy::Steal { (&a.taken_from, true) } else { (&a.clashes, false) };
        if holders.is_empty() {
            return;
        }
        let label = env.holder_labels(holders);
        let text = if steal {
            fill!(env, chord_was_removed_from_label, ("chord", &chord), ("label", &label))
        } else {
            fill!(env, also_used_by_label, ("label", &label))
        };
        let goto = holders.first().map(|h| h.command.clone());
        self.message = Some(Message { text, refused: false, goto, undo: policy == Policy::Steal, chord: Some(chord) });
    }

    /// Take It.
    pub(crate) fn take_it(&mut self, env: &mut Env) {
        if let Some(c) = self.conflict.take() {
            self.apply_and_say(env, &c.command, c.old, c.binding, Policy::Steal);
        }
    }

    /// Accept and Go to Conflict.
    pub(crate) fn take_it_and_go(&mut self, env: &mut Env) {
        if let Some(c) = self.conflict.take() {
            let first = c.holders.first().map(|h| h.command.clone());
            self.apply_and_say(env, &c.command, c.old, c.binding, Policy::Steal);
            if let Some(holder) = first {
                self.go_to(&holder);
            }
        }
    }

    /// Keep Both.
    pub(crate) fn keep_both(&mut self, env: &mut Env) {
        if !env.options.allow_keep {
            return;
        }
        if let Some(c) = self.conflict.take() {
            self.apply_and_say(env, &c.command, c.old, c.binding, Policy::Keep);
        }
    }

    /// ✕ on the conflict bar: nothing was applied, so nothing changes.
    pub(crate) fn dismiss_conflict(&mut self) {
        self.conflict = None;
    }

    // ---- Edits ----

    /// After any edit: drop the conflict bar, refresh the field's holders, report Changed.
    fn edited(&mut self, env: &Env) {
        self.changed = true;
        self.conflict = None;
        if let Some(f) = self.field.take() {
            self.set_field(env, f.command, f.binding.trigger);
        }
    }

    /// Runs one session edit; true when it changed something.
    fn edit(&mut self, env: &mut Env, f: impl FnOnce(&mut EditSession, &mut Keymap) -> bool) -> bool {
        self.message = None;
        let km = &mut *env.keymap;
        let done = f(self.session.get_or_insert_with(|| EditSession::begin(km)), km);
        if done {
            self.edited(env);
        }
        done
    }

    /// × on a chip, or Remove.
    pub(crate) fn remove(&mut self, env: &mut Env, command: &CommandRef, binding: &Binding) {
        self.edit(env, |s, km| s.remove(km, command, binding));
    }

    /// Clear: no shortcut.
    pub(crate) fn clear(&mut self, env: &mut Env, command: &CommandRef) {
        self.edit(env, |s, km| s.clear(km, command));
    }

    /// Use Default, ↺.
    pub(crate) fn use_default(&mut self, env: &mut Env, command: &CommandRef) {
        self.edit(env, |s, km| s.reset(km, command));
    }

    /// Whether Use Default does anything.
    pub(crate) fn can_use_default(env: &Env, command: &CommandRef) -> bool {
        env.keymap.overrides().get(&command.key()).is_some()
    }

    /// Reset All…: asks first.
    pub(crate) fn ask_reset_all(&mut self) {
        self.prompt = Some(Prompt::ResetAll);
    }

    fn reset_all(&mut self, env: &mut Env) {
        self.edit(env, |s, km| {
            let before = km.overrides().clone();
            s.reset_all(km);
            *km.overrides() != before
        });
    }

    pub(crate) fn undo(&mut self, env: &mut Env) {
        self.edit(env, |s, km| s.undo(km));
    }

    pub(crate) fn redo(&mut self, env: &mut Env) {
        self.edit(env, |s, km| s.redo(km));
    }

    pub(crate) fn can_undo(&self) -> bool {
        self.session.as_ref().is_some_and(EditSession::can_undo)
    }

    pub(crate) fn can_redo(&self) -> bool {
        self.session.as_ref().is_some_and(EditSession::can_redo)
    }

    // ---- OK, Cancel, Escape, Enter ----

    /// OK: with sets, stores the overrides (a built-in's edits become a copy), then closes.
    pub(crate) fn ok(&mut self, env: &mut Env) {
        if let Err(e) = self.store(env) {
            let text = profile_error(env, &e);
            self.refuse(text);
            return;
        }
        self.close(Outcome::Ok);
    }

    /// Cancel, window ×, or Escape when idle: everything back as it was when the dialog opened.
    /// When that undoes edits, Changed follows, so the app rebuilds its menus (and saves, if it
    /// saved the edits live).
    pub(crate) fn cancel(&mut self, env: &mut Env) {
        let mut undone = false;
        if let Some(opened) = self.opened.take() {
            undone = *env.keymap.overrides() != opened.overrides;
            env.keymap.set_overrides(opened.overrides);
            if let (Some((p, _)), Some(profiles)) = (env.sets.as_mut(), opened.profiles) {
                undone |= **p != profiles;
                **p = profiles;
            }
        }
        self.close(Outcome::Cancel);
        self.changed = undone;
    }

    fn close(&mut self, outcome: Outcome) {
        self.phase = Phase::Closed;
        self.recording = None;
        self.conflict = None;
        self.prompt = None;
        // Ok and Cancel cover every edit.
        self.changed = false;
        self.outcome = Some(outcome);
    }

    /// Escape when nothing records: closes the innermost open thing (conflict bar, prompt, import
    /// review, the field's chord), else cancels. Rename fields and dropdowns are the view's.
    pub(crate) fn escape(&mut self, env: &mut Env) {
        if self.recording.is_some() {
            self.stop_recording();
        } else if self.conflict.is_some() {
            self.conflict = None;
        } else if self.prompt.is_some() {
            self.prompt = None;
        } else if self.set_form.is_some() {
            self.set_form = None;
        } else if self.review.is_some() {
            self.review = None;
        } else if self.field.is_some() {
            self.field = None;
        } else {
            self.cancel(env);
        }
    }
}

impl DialogState {
    // ---- Sets ----

    /// Whether the keymap's overrides differ from the stored active set.
    pub(crate) fn is_modified(env: &Env) -> bool {
        env.sets.as_ref().is_some_and(|(p, b)| p.active_overrides(b) != env.keymap.overrides())
    }

    fn custom_name(env: &Env) -> String {
        env.custom_set_name()
    }

    /// Stores the overrides in the active set; a built-in's edits become a new set.
    fn store(&mut self, env: &mut Env) -> Result<(), ProfileError> {
        let custom = Self::custom_name(env);
        if let Some((p, b)) = env.sets.as_mut()
            && p.active_overrides(b) != env.keymap.overrides()
        {
            p.store(env.keymap.overrides().clone(), b, &custom)?;
            self.changed = true;
        }
        Ok(())
    }

    /// Makes `name` active and loads it, with a fresh undo history.
    fn load_set(&mut self, env: &mut Env, name: &str) -> Result<(), ProfileError> {
        let Some((p, b)) = env.sets.as_mut() else { return Err(ProfileError::NotFound) };
        p.select(name, b)?;
        self.load_active(env);
        Ok(())
    }

    /// Loads the active set (no overrides when none is left), with a fresh undo history.
    fn load_active(&mut self, env: &mut Env) {
        if let Some((p, b)) = env.sets.as_ref() {
            env.keymap.set_overrides(p.active_overrides(b).clone());
        }
        self.session = Some(EditSession::begin(env.keymap));
        self.recording = None;
        self.conflict = None;
        self.field = None;
        self.prompt = None;
        self.changed = true;
    }

    /// A prompt or the import review is open: the set row, the table and the edit actions wait.
    pub(crate) fn modal(&self) -> bool {
        self.prompt.is_some() || self.review.is_some()
    }

    fn report(&mut self, env: &Env, result: Result<(), ProfileError>) {
        match result {
            Ok(()) => self.message = None,
            Err(e) => self.refuse(profile_error(env, &e)),
        }
    }

    /// The set picker. With unsaved edits it asks to save them first.
    pub(crate) fn switch_set(&mut self, env: &mut Env, name: &str) {
        self.prompt = None;
        self.after_save_prompt(env, Then::Switch(name.to_string()));
    }

    /// New Set…: a copy of the current shortcuts (`based_on: None`) or of a set. A name that is
    /// taken (ignoring case) is refused, before any prompt to save.
    pub(crate) fn new_set(&mut self, env: &mut Env, name: &str, based_on: Option<&str>) {
        if Self::name_taken(env, name) {
            self.refuse(profile_error(env, &ProfileError::NameTaken));
            return;
        }
        let then = Then::NewSet { name: name.to_string(), based_on: based_on.map(str::to_string) };
        if based_on.is_none() {
            self.run(env, then);
        } else {
            self.after_save_prompt(env, then);
        }
    }

    /// Import…: asks to save unsaved edits first, then asks the app for a file.
    pub(crate) fn request_import(&mut self, env: &mut Env) {
        self.after_save_prompt(env, Then::Import);
    }

    fn after_save_prompt(&mut self, env: &mut Env, then: Then) {
        if Self::is_modified(env)
            && let Some((p, b)) = env.sets.as_ref()
        {
            self.prompt = Some(Prompt::SaveChanges { set: p.active_name(b).to_string(), then });
            return;
        }
        self.run(env, then);
    }

    fn run(&mut self, env: &mut Env, then: Then) {
        let result = match then {
            Then::Switch(name) => self.load_set(env, &name),
            Then::NewSet { name, based_on } => {
                // Saving edits to a built-in first can have made a set of this name.
                let taken = Self::name_taken(env, &name);
                let made = match (env.sets.as_mut(), based_on) {
                    _ if taken => Err(ProfileError::NameTaken),
                    (None, _) => Err(ProfileError::NotFound),
                    (Some((p, b)), None) => p.save_as(&name, env.keymap.overrides().clone(), b),
                    (Some((p, b)), Some(from)) => p.duplicate(&from, &name, b),
                };
                made.and_then(|n| self.load_set(env, &n))
            }
            Then::Import => {
                let formats = std::iter::once(JSON_FORMAT).chain(env.options.import_formats.iter().map(|f| f.id)).collect();
                self.outcome = Some(Outcome::Import { formats });
                Ok(())
            }
        };
        self.report(env, result);
    }

    fn name_taken(env: &Env, name: &str) -> bool {
        let (Some((p, b)), Some(name)) = (env.sets.as_ref(), set_name(name)) else { return false };
        p.taken(&name, b)
    }

    /// Rename… (not for built-ins).
    pub(crate) fn rename_set(&mut self, env: &mut Env, to: &str) {
        let result = match env.sets.as_mut() {
            Some((p, b)) => {
                let from = p.active_name(b).to_string();
                p.rename(&from, to, b).map(|_| ())
            }
            None => Err(ProfileError::NotFound),
        };
        if result.is_ok() {
            self.changed = true;
        }
        self.report(env, result);
    }

    /// Delete: asks first.
    pub(crate) fn ask_delete_set(&mut self, env: &Env) {
        if let Some((p, b)) = env.sets.as_ref() {
            let name = p.active_name(b);
            if !Profiles::is_builtin(name, b) {
                self.prompt = Some(Prompt::DeleteSet(name.to_string()));
            }
        }
    }

    /// Deletes a user set. Deleting the active set loads the one that becomes active, or no
    /// overrides when no set is left.
    fn delete_set(&mut self, env: &mut Env, name: &str) {
        let result = match env.sets.as_mut() {
            Some((p, b)) => {
                let was_active = p.active_name(b) == name;
                p.delete(name, b).map(|()| was_active)
            }
            None => Err(ProfileError::NotFound),
        };
        match result {
            Ok(true) => self.load_active(env),
            Ok(false) => self.changed = true,
            Err(_) => {}
        }
        self.report(env, result.map(|_| ()));
    }

    /// Save: stores the edits; a built-in's become a copy. The undo history stays.
    pub(crate) fn save_set(&mut self, env: &mut Env) {
        let result = self.store(env);
        self.report(env, result);
    }

    /// Revert Set: back to the stored set, as one undo step.
    pub(crate) fn revert_set(&mut self, env: &mut Env) {
        let Some(stored) = env.sets.as_ref().map(|(p, b)| p.active_overrides(b).clone()) else { return };
        self.edit(env, |s, km| {
            let before = km.overrides().clone();
            s.edit(km, |k| k.set_overrides(stored));
            *km.overrides() != before
        });
    }

    /// Export ▾ → Keyboard Shortcuts (JSON): the set as shown, edits included.
    pub(crate) fn export_set(&mut self, env: &mut Env) {
        let custom = Self::custom_name(env);
        let result = match env.sets.as_ref() {
            Some((p, b)) => {
                let mut tmp = (**p).clone();
                tmp.store(env.keymap.overrides().clone(), b, &custom).and_then(|_| {
                    let name = tmp.active_name(b).to_string();
                    tmp.export(&name, b).map(|json| (name, json))
                })
            }
            None => {
                let mut tmp = Profiles::default();
                tmp.save_as(&custom, env.keymap.overrides().clone(), &[]).and_then(|name| tmp.export(&name, &[]).map(|json| (name, json)))
            }
        };
        match result {
            Ok((name, json)) => self.outcome = Some(Outcome::Export { name, json }),
            Err(e) => self.refuse(profile_error(env, &e)),
        }
    }

    /// Export ▾ → an app format.
    pub(crate) fn export_as(&mut self, env: &Env, format: &'static str) {
        let name = match env.sets.as_ref() {
            Some((p, b)) => p.active_name(b).to_string(),
            None => Self::custom_name(env),
        };
        self.outcome = Some(Outcome::ExportAs { format, name, keymap: Box::new(env.keymap.clone()) });
    }

    /// A craft-keymap JSON set the app read: a new set, loaded; without sets its overrides replace
    /// the keymap's as one undo step.
    pub(crate) fn import_json(&mut self, env: &mut Env, text: &str) -> Result<String, ProfileError> {
        let result = match env.sets.as_mut() {
            Some((p, b)) => p.import(text, b).and_then(|name| self.load_set(env, &name).map(|()| name)),
            None => {
                let mut tmp = Profiles::default();
                let name = tmp.import(text, &[]);
                if name.is_ok() {
                    let overrides = tmp.active_overrides(&[]).clone();
                    self.edit(env, |s, km| {
                        s.edit(km, |k| k.set_overrides(overrides));
                        true
                    });
                }
                name
            }
        };
        if let Err(e) = &result {
            self.refuse(profile_error(env, e));
        }
        result
    }

    /// Summarize…: every command with a shortcut, as HTML and plain text.
    pub(crate) fn summarize(&mut self, env: &Env) {
        let title = env.title();
        let (name, heading) = match env.sets.as_ref() {
            Some((p, b)) => {
                let name = p.active_name(b).to_string();
                let heading = fill!(env, title_set, ("title", &title), ("set", &name));
                (name, heading)
            }
            None => (title.clone(), title),
        };
        let html = summary::html_in(env.keymap, env.text, &heading);
        let text = summary::text_in(env.keymap, env.text, &heading);
        self.outcome = Some(Outcome::Summary { name, html, text });
    }

    /// Answers the open prompt.
    pub(crate) fn answer(&mut self, env: &mut Env, answer: Answer) {
        let Some(prompt) = self.prompt.take() else { return };
        match (prompt, answer) {
            (_, Answer::Cancel) => {}
            (Prompt::ResetAll, Answer::Yes) => self.reset_all(env),
            (Prompt::DeleteSet(name), Answer::Yes) => self.delete_set(env, &name),
            (Prompt::SaveChanges { then, .. }, Answer::Yes) => match self.store(env) {
                Ok(()) => self.run(env, then),
                Err(e) => self.refuse(profile_error(env, &e)),
            },
            // Don't Save reloads the stored set first, so an import does not start from the edits.
            (Prompt::SaveChanges { set, then }, Answer::No) => match self.load_set(env, &set) {
                Ok(()) => self.run(env, then),
                Err(e) => self.refuse(profile_error(env, &e)),
            },
            (Prompt::ResetAll | Prompt::DeleteSet(_), Answer::No) => {}
        }
    }

    // ---- Import review ----

    pub(crate) fn review_import(&mut self, env: &mut Env, set: ForeignSet, options: ImportOptions) {
        let plan = import::plan(&set, env.keymap, &options);
        let name = plan.report.set.clone().unwrap_or_else(|| word!(env, imported));
        self.stop_recording();
        self.conflict = None;
        self.review = Some(Review { set, options, plan, name, done: None });
    }

    /// Picks candidate `candidate` for suggestion `suggestion`, re-plans, and reports the answers
    /// for the app to keep.
    pub(crate) fn pick_suggestion(&mut self, env: &Env, suggestion: usize, candidate: usize) {
        let Some(r) = &mut self.review else { return };
        let Some(s) = r.plan.report.suggestions.get(suggestion) else { return };
        let Some(command) = s.candidates.get(candidate).cloned() else { return };
        let Some(entry) = r.set.entries.iter().find(|e| e.item() == s.item && e.line == s.line) else { return };
        r.options.remembered.insert(entry.remember_key(), command);
        r.plan = import::plan(&r.set, env.keymap, &r.options);
        self.outcome = Some(Outcome::Remembered(r.options.remembered.clone()));
    }

    /// Import as New Set (with sets), else applies the plan to the keymap as one undo step.
    pub(crate) fn import_reviewed(&mut self, env: &mut Env) {
        let Some(mut r) = self.review.take() else { return };
        let result = match env.sets.as_mut() {
            Some((p, b)) => p.import_plan(&r.plan, env.keymap, Some(&r.name), b),
            None => {
                let plan = &r.plan;
                let mut report = None;
                self.edit(env, |s, km| {
                    report = Some(s.edit(km, |k| plan.apply(k)));
                    true
                });
                Ok((r.name.clone(), report.unwrap_or_default()))
            }
        };
        match result {
            Ok((name, report)) => {
                if env.sets.is_some()
                    && let Err(e) = self.load_set(env, &name)
                {
                    self.refuse(profile_error(env, &e));
                }
                r.done = Some((name, report));
            }
            Err(e) => self.refuse(profile_error(env, &e)),
        }
        self.review = Some(r);
    }

    pub(crate) fn close_review(&mut self) {
        self.review = None;
    }
}

fn assign_error(env: &Env, e: &AssignError) -> String {
    match e {
        AssignError::Reserved(t) => fill!(env, chord_is_reserved_by_the_system, ("chord", &env.chord_text(t))),
        AssignError::TooMany => fill!(env, a_command_can_have_at_most, ("n", &MAX_BINDINGS.to_string())),
        AssignError::UnknownScope(s) => fill!(env, the_scope_scope_is_unknown, ("scope", s.as_str())),
        AssignError::Full => word!(env, too_many_shortcuts_are_assigned),
        AssignError::UnknownCommand => word!(env, this_command_is_unknown),
        AssignError::Taken(holders) => fill!(env, also_used_by_label, ("label", &env.holder_labels(holders))),
        _ => e.to_string(),
    }
}

/// Why a binding text does not parse, translated.
pub(crate) fn binding_error(env: &Env, e: &BindingError) -> String {
    match e {
        BindingError::TooLong => word!(env, the_shortcut_is_too_long),
        BindingError::Scope => word!(env, the_scope_name_is_not_valid),
        BindingError::Trigger(e) => match e {
            ParseError::Empty => word!(env, empty_shortcut),
            ParseError::TooLong => word!(env, the_shortcut_is_too_long),
            ParseError::MissingKey => word!(env, the_shortcut_has_no_key),
            ParseError::UnknownKey(k) => fill!(env, unknown_key_key, ("key", k)),
            ParseError::UnknownModifier(m) => fill!(env, unknown_modifier_key, ("key", m)),
            ParseError::DuplicateModifier(m) => fill!(env, modifier_key_given_twice, ("key", m)),
            ParseError::Sequence => word!(env, key_sequences_are_not_supported),
            _ => e.to_string(),
        },
        _ => e.to_string(),
    }
}

fn profile_error(env: &Env, e: &ProfileError) -> String {
    match e {
        ProfileError::BadName => word!(env, the_name_is_empty_too_long),
        ProfileError::NameTaken => word!(env, a_set_with_this_name_exists),
        ProfileError::NotFound => word!(env, the_set_was_not_found),
        ProfileError::BuiltIn => word!(env, built_in_sets_cannot_be_changed),
        ProfileError::TooMany => word!(env, there_are_too_many_sets),
        ProfileError::BadFile => word!(env, this_is_not_a_shortcut_set),
        ProfileError::WrongFormat => word!(env, this_is_not_a_craft_keymap),
        ProfileError::NewerVersion(v) => fill!(env, this_file_is_from_a_newer, ("n", &v.to_string())),
        ProfileError::TooLarge => word!(env, the_file_is_too_large),
        _ => e.to_string(),
    }
}
