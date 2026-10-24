//! The dialog's persistent state and every action it takes, as calls on [`EditSession`],
//! [`Recorder`], [`editor::holders`] and [`Profiles`].

use std::collections::BTreeSet;

use crate::chord::{Chord, Key, ParseError, Trigger};
use crate::command::CommandRef;
use crate::editor::{self, DOUBLE_PRESS, EditSession, Recorded, Recorder};
use crate::import::{self, ForeignSet, ImportOptions, ImportPlan, ImportReport};
use crate::keymap::{AssignError, Assigned, Binding, BindingError, Holder, Keymap, MAX_BINDINGS, Overrides, Policy};
use crate::profiles::{Builtins, ProfileError, Profiles, set_name};
use crate::scope::Scope;
use crate::ui::rows::{AreaEntry, AreaId, Built, Filter, RowCache, View};
use crate::ui::text::{Text, trigger_text};
use crate::ui::view::keyboard::KeyboardState;
use crate::ui::{ConflictMode, JSON_FORMAT, Options, Outcome, capture, summary};

/// What the dialog edits this frame.
pub(crate) struct Env<'e> {
    pub keymap: &'e mut Keymap,
    pub sets: Option<(&'e mut Profiles, &'e Builtins<'e>)>,
    pub options: &'e Options,
    pub text: &'e dyn Text,
}

impl Env<'_> {
    fn chord_text(&self, t: &Trigger) -> String {
        trigger_text(self.text, t, self.keymap.platform())
    }

    fn label(&self, command: &CommandRef) -> String {
        self.keymap.bindable(command).map_or_else(|| command.id.clone(), |b| self.text.command(b).into_owned())
    }

    /// `label (Scope)` for each holder, joined.
    fn holder_labels(&self, holders: &[Holder]) -> String {
        let mut out: Vec<String> = Vec::new();
        for h in holders {
            let label = self.label(&h.command);
            let s = match &h.binding.scope {
                Some(scope) => self.ui("{label} ({scope})", &[("label", &label), ("scope", &self.text.segment(scope.as_str()))]),
                None => label,
            };
            if !out.contains(&s) {
                out.push(s);
            }
        }
        out.join(", ")
    }

    fn ui(&self, source: &str, args: &[(&str, &str)]) -> String {
        craft_i18n::fmt(&self.text.ui(source), args)
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
    /// "⚠ {chord} is used by {label}".
    pub text: String,
}

/// A line under the table after an action.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Message {
    pub text: String,
    /// Nothing changed: shown with ⚠.
    pub refused: bool,
    /// Offers Go to Conflict.
    pub goto: Option<CommandRef>,
    /// Offers Undo.
    pub undo: bool,
    /// The chord text inside `text`, which is never put through `Text::visual`.
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
    /// Focus the name field on the next frame.
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
    /// The table's share of the table/detail split.
    pub(crate) split: f32,
    /// The chip of the selected row that Enter and Delete act on.
    pub(crate) chip: usize,
    /// Typed entry in the detail pane, and the parse error shown under it.
    pub(crate) typing: Option<String>,
    pub(crate) type_error: Option<String>,
    /// Height of the bars and footer under the table, measured last frame.
    pub(crate) bottom_h: f32,
    /// One of the dialog's text fields had focus at the end of last frame.
    pub(crate) text_focus: bool,
    /// The table lines drawn last frame.
    pub(crate) visible: std::ops::Range<usize>,
    pub(crate) set_form: Option<SetForm>,
    pub(crate) keyboard: KeyboardState,
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
            split: 0.62,
            chip: 0,
            typing: None,
            type_error: None,
            bottom_h: 0.0,
            text_focus: false,
            visible: 0..0,
            set_form: None,
            keyboard: KeyboardState::default(),
        }
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
        self.areas = crate::ui::rows::areas(env.keymap, env.options, env.text);
    }

    /// Takes this frame's key input while recording and commits a double-press wait that ended.
    pub(crate) fn input(&mut self, env: &mut Env, ctx: &egui::Context) {
        let now = ctx.input(|i| i.time);
        self.tick(env, now);
        if self.is_recording() {
            let platform = env.keymap.platform();
            let chords = ctx.input_mut(|i| capture::take(i, platform));
            self.feed(env, &chords, now);
        }
        if let Some((_, at)) = self.recording.as_ref().and_then(|r| r.waiting) {
            ctx.request_repaint_after(std::time::Duration::from_secs_f64((at + DOUBLE_PRESS - now).max(0.0)));
        }
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

    /// The table's lines as last built by [`lines`](Self::lines).
    pub(crate) fn lines_cached(&self) -> &[crate::ui::rows::Line] {
        &self.rows.built().lines
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
        self.recording = Some(Recording { target, waiting: None });
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

    /// Assign in the detail pane: commits the field's chord.
    pub(crate) fn assign_field(&mut self, env: &mut Env) {
        if let Some(f) = self.field.take() {
            self.commit(env, &f.command, None, f.binding);
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
        self.conflict = None;
        self.message = None;
        let chord = env.chord_text(&binding.trigger);
        if env.keymap.is_reserved(&binding.trigger.chord) {
            self.refuse_chord(env.ui("{chord} is reserved by the system", &[("chord", &chord)]), chord);
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
        match env.options.conflicts {
            ConflictMode::Ask => {
                let text = env.ui("⚠ {chord} is used by {label}", &[("chord", &chord), ("label", &env.holder_labels(&holders))]);
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
        let (holders, text) =
            if policy == Policy::Steal { (&a.taken_from, "{chord} was removed from {label}") } else { (&a.clashes, "Also used by {label}") };
        if holders.is_empty() {
            return;
        }
        let text = env.ui(text, &[("chord", &chord), ("label", &env.holder_labels(holders))]);
        let goto = holders.first().map(|h| h.command.clone());
        self.message = Some(Message { text, refused: false, goto, undo: policy == Policy::Steal, chord: Some(chord) });
    }

    /// Take It.
    pub(crate) fn take_it(&mut self, env: &mut Env) {
        if let Some(c) = self.conflict.take() {
            self.apply_and_say(env, &c.command, c.old, c.binding, Policy::Steal);
        }
    }

    /// Take It and Go to Conflict.
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

    /// Enter when idle and no text field has focus: OK with [`Options::enter_is_ok`].
    pub(crate) fn enter(&mut self, env: &mut Env) {
        if env.options.enter_is_ok && self.recording.is_none() && self.prompt.is_none() {
            self.ok(env);
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
        env.text.ui(env.options.custom_set_name).into_owned()
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
        let title = env.text.ui(env.options.title).into_owned();
        let (name, heading) = match env.sets.as_ref() {
            Some((p, b)) => {
                let name = p.active_name(b).to_string();
                let heading = env.ui("{title}: {set}", &[("title", &title), ("set", &name)]);
                (name, heading)
            }
            None => (title.clone(), title),
        };
        let html = summary::html(env.keymap, env.text, &heading);
        let text = summary::text(env.keymap, env.text, &heading);
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
        let name = plan.report.set.clone().unwrap_or_else(|| env.text.ui("Imported").into_owned());
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
        AssignError::Reserved(t) => env.ui("{chord} is reserved by the system", &[("chord", &env.chord_text(t))]),
        AssignError::TooMany => env.ui("A command can have at most {n} shortcuts", &[("n", &MAX_BINDINGS.to_string())]),
        AssignError::UnknownScope(s) => env.ui("The scope {scope} is unknown", &[("scope", s.as_str())]),
        AssignError::Full => env.ui("Too many shortcuts are assigned", &[]),
        AssignError::UnknownCommand => env.ui("This command is unknown", &[]),
        AssignError::Taken(holders) => env.ui("Also used by {label}", &[("label", &env.holder_labels(holders))]),
    }
}

/// Why a binding text does not parse, translated.
pub(crate) fn binding_error(env: &Env, e: &BindingError) -> String {
    match e {
        BindingError::TooLong => env.ui("the shortcut is too long", &[]),
        BindingError::Scope => env.ui("the scope name is not valid", &[]),
        BindingError::Trigger(e) => match e {
            ParseError::Empty => env.ui("empty shortcut", &[]),
            ParseError::TooLong => env.ui("the shortcut is too long", &[]),
            ParseError::MissingKey => env.ui("the shortcut has no key", &[]),
            ParseError::UnknownKey(k) => env.ui("unknown key {key}", &[("key", k)]),
            ParseError::UnknownModifier(m) => env.ui("unknown modifier {key}", &[("key", m)]),
            ParseError::DuplicateModifier(m) => env.ui("modifier {key} given twice", &[("key", m)]),
            ParseError::Sequence => env.ui("key sequences are not supported", &[]),
        },
    }
}

fn profile_error(env: &Env, e: &ProfileError) -> String {
    match e {
        ProfileError::BadName => env.ui("The name is empty, too long or has control characters", &[]),
        ProfileError::NameTaken => env.ui("A set with this name exists", &[]),
        ProfileError::NotFound => env.ui("The set was not found", &[]),
        ProfileError::BuiltIn => env.ui("Built-in sets cannot be changed", &[]),
        ProfileError::TooMany => env.ui("There are too many sets", &[]),
        ProfileError::BadFile => env.ui("This is not a shortcut set file", &[]),
        ProfileError::WrongFormat => env.ui("This is not a craft-keymap file", &[]),
        ProfileError::NewerVersion(v) => env.ui("This file is from a newer version (format {n})", &[("n", &v.to_string())]),
        ProfileError::TooLarge => env.ui("The file is too large", &[]),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::chord::Platform;
    use crate::keymap::ConflictKind;
    use crate::ui::rows::Line;
    use crate::ui::rows::tests::fixture;
    use crate::ui::text::English;

    struct Fx {
        km: Keymap,
        p: Profiles,
        b: Vec<(&'static str, Overrides)>,
        o: Options,
        sets: bool,
    }

    impl Fx {
        fn new(o: Options) -> Fx {
            Fx { km: fixture(Platform::Mac), p: Profiles::default(), b: vec![("Fixture Defaults", Overrides::new())], o, sets: true }
        }

        fn env(&mut self) -> Env<'_> {
            let sets = if self.sets { Some((&mut self.p, &self.b[..])) } else { None };
            Env { keymap: &mut self.km, sets, options: &self.o, text: &English }
        }

        fn keys(&self, id: &str) -> Vec<String> {
            self.km.bindings(&id.into()).iter().map(ToString::to_string).collect()
        }
    }

    fn c(s: &str) -> Chord {
        Chord::parse(s).unwrap_or(Chord::from(Key::F35))
    }

    fn b(s: &str) -> Binding {
        Binding::parse(s).unwrap_or(Binding::new(Chord::from(Key::F35)))
    }

    fn open(fx: &mut Fx) -> DialogState {
        let mut st = DialogState::new();
        st.begin(&mut fx.env());
        st
    }

    /// Records `chords` into `target` at time 0.
    fn record(st: &mut DialogState, fx: &mut Fx, target: Target, chords: &[&str]) {
        let mut env = fx.env();
        st.record(&env, target);
        let chords: Vec<Chord> = chords.iter().map(|s| c(s)).collect();
        st.feed(&mut env, &chords, 0.0);
    }

    fn add(command: &str) -> Target {
        Target::Cell { command: command.into(), old: None }
    }

    #[test]
    fn recording_takes_the_frames_keys_before_the_app() {
        let mut fx = Fx::new(Options::default());
        let mut st = open(&mut fx);
        st.record(&fx.env(), Target::Cell { command: "file.save".into(), old: Some(b("Cmd+S")) });
        let ctx = egui::Context::default();
        let press = egui::Event::Key { key: egui::Key::E, physical_key: None, pressed: true, repeat: false, modifiers: egui::Modifiers::MAC_CMD };
        let input = egui::RawInput { events: vec![press, egui::Event::Text("e".into())], ..egui::RawInput::default() };
        let mut left = usize::MAX;
        let mut full = ctx.run_ui(input, |ui| {
            st.input(&mut fx.env(), ui.ctx());
            left = ui.ctx().input(|i| i.events.len());
        });
        full.textures_delta.clear();
        assert_eq!(left, 0);
        assert!(!st.is_recording());
        assert_eq!(st.conflict.as_ref().map(|c| c.text.as_str()), Some("⚠ ⌘E is used by Export…"));
        assert_eq!(fx.keys("file.save"), ["Cmd+S"], "nothing changes before the answer");
    }

    #[test]
    fn recording_takes_tab_and_shift_escape_and_escape_cancels_only_the_recording() {
        let mut fx = Fx::new(Options::default());
        let mut st = open(&mut fx);
        record(&mut st, &mut fx, add("object.front"), &["Tab"]);
        record(&mut st, &mut fx, add("object.front"), &["Shift+Escape"]);
        assert_eq!(fx.keys("object.front"), ["Tab", "Shift+Escape"]);

        record(&mut st, &mut fx, add("object.front"), &["Escape"]);
        assert!(!st.is_recording() && st.is_open());
        assert_eq!(fx.keys("object.front").len(), 2);
        assert!(matches!(st.finish(), Some(Outcome::Changed)));
        st.escape(&mut fx.env());
        assert!(matches!(st.finish(), Some(Outcome::Cancel)));
        assert!(fx.keys("object.front").is_empty(), "Cancel restores the opening keymap");
        assert!(st.is_open() && matches!(st.finish(), Some(Outcome::Changed)), "Changed follows, for the app's menus");
        assert!(!st.is_open());
    }

    #[test]
    fn double_press_records_k_k_within_the_window() {
        let mut fx = Fx::new(Options { double_press: true, ..Options::default() });
        let mut st = open(&mut fx);
        let mut env = fx.env();
        st.record(&env, add("object.front"));
        st.feed(&mut env, &[c("K")], 0.0);
        assert!(st.is_recording(), "a single waits for a second press");
        st.feed(&mut env, &[c("K")], 0.3);
        assert!(!st.is_recording());
        assert_eq!(fx.keys("object.front"), ["K K"]);

        let mut env = fx.env();
        st.record(&env, add("type.bold"));
        st.feed(&mut env, &[c("K")], 1.0);
        st.feed(&mut env, &[c("K")], 1.9);
        assert_eq!(fx.keys("type.bold"), ["Text: Cmd+B", "K"]);
    }

    #[test]
    fn conflict_take_it_steals_and_undo_restores() {
        let mut fx = Fx::new(Options::default());
        let mut st = open(&mut fx);
        record(&mut st, &mut fx, add("file.export"), &["Cmd+S"]);
        assert_eq!(st.conflict.as_ref().map(|c| c.text.as_str()), Some("⚠ ⌘S is used by Save"));
        st.take_it(&mut fx.env());
        assert_eq!((fx.keys("file.save"), fx.keys("file.export")), (vec![], vec!["Cmd+E".to_string(), "Cmd+S".to_string()]));
        assert!(st.message.as_ref().is_some_and(|m| m.undo && m.goto == Some("file.save".into())));
        st.undo(&mut fx.env());
        assert_eq!((fx.keys("file.save"), fx.keys("file.export")), (vec!["Cmd+S".to_string()], vec!["Cmd+E".to_string()]));
        st.redo(&mut fx.env());
        assert!(fx.keys("file.save").is_empty());
    }

    #[test]
    fn conflict_keep_both_lists_the_clash_and_cancel_changes_nothing() {
        let mut fx = Fx::new(Options::default());
        let mut st = open(&mut fx);
        record(&mut st, &mut fx, add("file.export"), &["Cmd+S"]);
        st.dismiss_conflict();
        assert_eq!(fx.keys("file.export"), ["Cmd+E"]);
        record(&mut st, &mut fx, add("file.export"), &["Cmd+S"]);
        st.keep_both(&mut fx.env());
        let clashes = fx.km.conflicts().into_iter().filter(|c| c.kind == ConflictKind::Clash).count();
        assert_eq!(clashes, 1);
        st.set_filter(Filter::Conflicts);
        let env = fx.env();
        let rows: Vec<String> = st.lines(&env).lines.iter().filter_map(|l| if let Line::Row(r) = l { Some(r.key.clone()) } else { None }).collect();
        assert_eq!(rows, ["file.save", "file.export"]);
    }

    #[test]
    fn go_to_conflict_selects_and_reveals_the_holder() {
        let mut fx = Fx::new(Options::default());
        let mut st = open(&mut fx);
        st.set_area(AreaId::Tools);
        st.set_query("marquee".into());
        record(&mut st, &mut fx, add("tool.marquee"), &["Cmd+S"]);
        st.take_it_and_go(&mut fx.env());
        assert_eq!((&st.area, st.query.as_str(), st.selected.clone(), st.reveal), (&AreaId::All, "", Some("file.save".into()), true));
    }

    #[test]
    fn steal_and_keep_modes_apply_and_say_so() {
        let mut fx = Fx::new(Options { conflicts: ConflictMode::Steal, ..Options::default() });
        let mut st = open(&mut fx);
        record(&mut st, &mut fx, add("file.export"), &["Cmd+S"]);
        assert_eq!(st.message.as_ref().map(|m| m.text.as_str()), Some("⌘S was removed from Save"));
        let mut fx = Fx::new(Options { conflicts: ConflictMode::Keep, ..Options::default() });
        let mut st = open(&mut fx);
        record(&mut st, &mut fx, add("file.export"), &["Cmd+S"]);
        assert_eq!(st.message.as_ref().map(|m| (m.text.as_str(), m.undo)), Some(("Also used by Save", false)));
        assert_eq!(fx.keys("file.save"), ["Cmd+S"]);
    }

    #[test]
    fn reserved_chord_and_app_rule_refuse() {
        let mut fx = Fx::new(Options {
            accept: Some(Box::new(|bindable, binding| match bindable.kind {
                crate::Kind::Tool { .. } if !binding.trigger.chord.mods.is_none() => Err("Tools take single keys".into()),
                _ => Ok(()),
            })),
            ..Options::default()
        });
        fx.km = fx.km.clone().with_reserved(vec![c("Cmd+Tab")]);
        let mut st = open(&mut fx);
        record(&mut st, &mut fx, add("object.front"), &["Cmd+Tab"]);
        assert_eq!(st.message.as_ref().map(|m| (m.text.as_str(), m.refused)), Some(("⌘⇥ is reserved by the system", true)));
        record(&mut st, &mut fx, add("tool.marquee"), &["Cmd+J"]);
        assert_eq!(st.message.as_ref().map(|m| m.text.as_str()), Some("Tools take single keys"));
        assert!(fx.km.overrides().is_empty());
        assert!(st.finish().is_none());
    }

    #[test]
    fn backspace_clears_the_binding_being_replaced() {
        let mut fx = Fx::new(Options { backspace_clears: true, ..Options::default() });
        let mut st = open(&mut fx);
        record(&mut st, &mut fx, Target::Cell { command: "file.save".into(), old: Some(b("Cmd+S")) }, &["Backspace"]);
        assert!(fx.keys("file.save").is_empty());
    }

    #[test]
    fn field_waits_for_assign_and_lists_holders() {
        let mut fx = Fx::new(Options::default());
        let mut st = open(&mut fx);
        record(&mut st, &mut fx, Target::Field { command: "file.save".into() }, &["Cmd+B"]);
        let f = st.field.clone();
        assert_eq!(f.as_ref().map(|f| (f.holders.len(), f.shadows.len())), Some((0, 1)), "Bold's Text: Cmd+B only shadows");
        st.set_context(&fx.env(), Scope::new("Text"));
        assert_eq!(st.field.as_ref().map(|f| f.holders.len()), Some(1), "in Text it clashes");
        st.assign_field(&mut fx.env());
        assert!(st.conflict.is_some());
        assert!(fx.km.overrides().is_empty());

        st.set_context(&fx.env(), None);
        assert_eq!(st.type_binding(&mut fx.env(), &"file.save".into(), "F7 F7"), Ok(()));
        st.assign_field(&mut fx.env());
        assert_eq!(fx.keys("file.save"), ["Cmd+S", "F7 F7"]);
        assert_eq!(st.type_binding(&mut fx.env(), &"file.save".into(), "Cmd+Nope"), Err("unknown key Nope".into()));
    }

    #[test]
    fn new_set_refuses_a_taken_name_and_keeps_the_set() {
        let mut fx = Fx::new(Options::default());
        let mut st = open(&mut fx);
        st.new_set(&mut fx.env(), "Mine", None);
        record(&mut st, &mut fx, add("object.front"), &["F2"]);
        st.save_set(&mut fx.env());
        let kept = fx.p.sets()["Mine"].clone();
        assert!(!kept.is_empty());
        st.switch_set(&mut fx.env(), "Fixture Defaults");
        for name in ["Mine", " mine ", "fixture defaults"] {
            for based_on in [None, Some("Fixture Defaults")] {
                st.new_set(&mut fx.env(), name, based_on);
                assert!(st.message.as_ref().is_some_and(|m| m.refused), "{name:?} from {based_on:?}");
                assert_eq!((fx.p.sets().len(), fx.p.active()), (1, "Fixture Defaults"));
                assert_eq!(fx.p.sets()["Mine"], kept);
            }
        }
        st.new_set(&mut fx.env(), "Other", None);
        st.rename_set(&mut fx.env(), "MINE");
        assert!(st.message.as_ref().is_some_and(|m| m.refused));
        assert_eq!(fx.p.sets()["Mine"], kept);
    }

    #[test]
    fn cancel_restores_keymap_and_sets() {
        let mut fx = Fx::new(Options::default());
        fx.p.save_as("Mine", Overrides::new(), &fx.b).ok();
        let (km0, p0) = (fx.km.overrides().clone(), fx.p.clone());
        let mut st = open(&mut fx);
        record(&mut st, &mut fx, add("object.front"), &["F2"]);
        st.new_set(&mut fx.env(), "Other", None);
        assert_eq!(fx.p.active(), "Other");
        st.ask_delete_set(&fx.env());
        assert_eq!(st.prompt, Some(Prompt::DeleteSet("Other".into())));
        st.answer(&mut fx.env(), Answer::Yes);
        assert!(!fx.p.sets().contains_key("Other"));
        st.cancel(&mut fx.env());
        assert_eq!((fx.km.overrides(), &fx.p), (&km0, &p0));
    }

    #[test]
    fn a_stale_delete_prompt_neither_reloads_nor_drops_edits() {
        let mut fx = Fx::new(Options::default());
        fx.p.save_as("Mine", Overrides::new(), &fx.b).ok();
        fx.p.save_as("Other", Overrides::new(), &fx.b).ok();
        let mut st = open(&mut fx);
        st.ask_delete_set(&fx.env());
        assert!(st.modal());
        st.switch_set(&mut fx.env(), "Mine");
        assert_eq!((st.prompt.clone(), fx.p.active()), (None, "Mine"), "switching closes the prompt");

        record(&mut st, &mut fx, add("object.front"), &["F2"]);
        st.prompt = Some(Prompt::DeleteSet("Other".into()));
        st.answer(&mut fx.env(), Answer::Yes);
        assert!(!fx.p.sets().contains_key("Other"));
        assert_eq!((fx.p.active(), fx.keys("object.front")), ("Mine", vec!["F2".to_string()]), "the active set keeps its edits");
        assert!(st.can_undo());
    }

    #[test]
    fn deleting_the_last_set_without_builtins_loads_no_overrides() {
        let mut fx = Fx::new(Options::default());
        fx.b.clear();
        let mut mine = Overrides::new();
        mine.insert("object.front", vec!["F2".into()]);
        fx.p.save_as("Mine", mine.clone(), &fx.b).ok();
        fx.km.set_overrides(mine);
        let mut st = open(&mut fx);
        st.ask_delete_set(&fx.env());
        st.answer(&mut fx.env(), Answer::Yes);
        assert!(st.message.is_none(), "{:?}", st.message);
        assert!(fx.p.sets().is_empty() && fx.km.overrides().is_empty() && !st.can_undo());
        st.ok(&mut fx.env());
        assert!(fx.p.sets().is_empty(), "OK stores no copy of the deleted set");
    }

    #[test]
    fn ok_stores_edits_of_a_builtin_as_a_copy() {
        let mut fx = Fx::new(Options::default());
        let mut st = open(&mut fx);
        record(&mut st, &mut fx, add("object.front"), &["F2"]);
        assert!(DialogState::is_modified(&fx.env()));
        st.ok(&mut fx.env());
        assert!(matches!(st.finish(), Some(Outcome::Ok)));
        assert_eq!(fx.p.active(), "Custom");
        assert_eq!(fx.p.get("Custom", &fx.b).and_then(|o| o.get("object.front")), Some(&["F2".to_string()][..]));
    }

    #[test]
    fn switching_a_modified_set_asks_to_save() {
        let mut fx = Fx::new(Options::default());
        fx.p.save_as("Mine", Overrides::new(), &fx.b).ok();
        let mut st = open(&mut fx);
        record(&mut st, &mut fx, add("object.front"), &["F2"]);
        st.switch_set(&mut fx.env(), "Fixture Defaults");
        assert!(matches!(&st.prompt, Some(Prompt::SaveChanges { set, .. }) if set == "Mine"));
        st.answer(&mut fx.env(), Answer::No);
        assert_eq!(fx.p.active(), "Fixture Defaults");
        assert!(fx.km.overrides().is_empty() && !st.can_undo(), "the set loads with a fresh undo history");
        assert!(fx.p.get("Mine", &fx.b).is_some_and(Overrides::is_empty), "Don't Save keeps the stored set");
    }

    #[test]
    fn dont_save_before_import_drops_the_edits() {
        use crate::import::{ContextRule, ForeignEntry, ForeignKey};
        let mut fx = Fx::new(Options::default());
        fx.p.save_as("Mine", Overrides::new(), &fx.b).ok();
        let mut st = open(&mut fx);
        record(&mut st, &mut fx, add("object.front"), &["F2"]);
        st.request_import(&mut fx.env());
        st.answer(&mut fx.env(), Answer::No);
        assert!(matches!(st.finish(), Some(Outcome::Import { .. })));
        assert!(fx.km.overrides().is_empty() && !st.can_undo());
        let read = |text: &str| ForeignKey::read(Some("Default"), text, crate::Flavor::Windows);
        let set = ForeignSet::new(vec![ForeignEntry::new(Some("File"), ["Save"]).with_key(read("Ctrl+Alt+S"))]).with_name("Theirs");
        let options = ImportOptions { contexts: vec![("Default".into(), ContextRule::Home)], ..ImportOptions::default() };
        st.review_import(&mut fx.env(), set, options);
        st.import_reviewed(&mut fx.env());
        assert_eq!(fx.p.active(), "Theirs");
        assert_eq!(fx.p.get("Theirs", &fx.b).and_then(|o| o.get("object.front")), None);
        assert!(fx.p.get("Mine", &fx.b).is_some_and(Overrides::is_empty));
    }

    #[test]
    fn save_prompt_before_import_reports_changed_after_import() {
        for answer in [Answer::No, Answer::Yes] {
            let mut fx = Fx::new(Options::default());
            fx.p.save_as("Mine", Overrides::new(), &fx.b).ok();
            let mut st = open(&mut fx);
            record(&mut st, &mut fx, add("object.front"), &["F2"]);
            assert!(matches!(st.finish(), Some(Outcome::Changed)));
            st.request_import(&mut fx.env());
            st.answer(&mut fx.env(), answer);
            assert!(matches!(st.finish(), Some(Outcome::Import { .. })), "{answer:?}");
            assert!(matches!(st.finish(), Some(Outcome::Changed)), "{answer:?}");
            assert!(st.finish().is_none());
        }
    }

    #[test]
    fn set_export_import_round_trip() {
        let mut fx = Fx::new(Options::default());
        let mut st = open(&mut fx);
        st.new_set(&mut fx.env(), "Name", None);
        record(&mut st, &mut fx, add("object.front"), &["F2"]);
        st.export_set(&mut fx.env());
        let (name, json) = match st.finish() {
            Some(Outcome::Export { name, json }) => (name, json),
            _ => (String::new(), String::new()),
        };
        assert_eq!(name, "Name");
        let edited = fx.km.overrides().clone();

        let mut fresh = Fx::new(Options::default());
        fresh.p = fx.p.clone();
        let mut st = open(&mut fresh);
        assert_eq!(st.import_json(&mut fresh.env(), &json), Ok("Name 2".into()));
        assert_eq!(fresh.km.overrides(), &edited);
        assert!(st.import_json(&mut fresh.env(), "{}").is_err());
        assert!(st.message.as_ref().is_some_and(|m| m.refused));

        let mut no_sets = Fx::new(Options::default());
        no_sets.sets = false;
        let mut st = open(&mut no_sets);
        assert!(st.import_json(&mut no_sets.env(), &json).is_ok());
        assert_eq!(no_sets.km.overrides(), &edited);
        assert!(st.can_undo());
    }

    #[test]
    fn import_request_and_reset_all() {
        let mut fx = Fx::new(Options::default());
        let mut st = open(&mut fx);
        st.request_import(&mut fx.env());
        assert!(matches!(st.finish(), Some(Outcome::Import { formats }) if formats == [JSON_FORMAT]));
        record(&mut st, &mut fx, add("object.front"), &["F2"]);
        st.ask_reset_all();
        st.answer(&mut fx.env(), Answer::Yes);
        assert!(fx.km.overrides().is_empty());
        st.undo(&mut fx.env());
        assert_eq!(fx.keys("object.front"), ["F2"]);
    }

    #[test]
    fn foreign_import_review_makes_a_new_set() {
        use crate::import::{ContextRule, ForeignEntry, ForeignKey};
        let mut fx = Fx::new(Options::default());
        let mut st = open(&mut fx);
        let read = |text: &str| ForeignKey::read(Some("Default"), text, crate::Flavor::Windows);
        let set = ForeignSet::new(vec![
            ForeignEntry::new(Some("File"), ["Save"]).with_key(read("Ctrl+Alt+S")).at_line(3),
            ForeignEntry::new(Some("File"), ["Frobnicate"]).with_key(read("Ctrl+F")).at_line(4),
        ])
        .with_name("Theirs");
        let options = ImportOptions { contexts: vec![("Default".into(), ContextRule::Home)], ..ImportOptions::default() };
        st.review_import(&mut fx.env(), set, options);
        let r = st.review.as_ref();
        assert_eq!(r.map(|r| (r.name.as_str(), r.plan.commands.len(), r.plan.report.unmatched.len())), Some(("Theirs", 1, 1)));
        assert!(fx.km.overrides().is_empty(), "review changes nothing");
        st.import_reviewed(&mut fx.env());
        assert_eq!(fx.p.active(), "Theirs");
        assert_eq!(fx.keys("file.save"), ["Cmd+Alt+S"]);
        assert!(st.review.as_ref().is_some_and(|r| r.done.is_some()));
        st.escape(&mut fx.env());
        assert!(st.review.is_none() && st.is_open());
    }
}
