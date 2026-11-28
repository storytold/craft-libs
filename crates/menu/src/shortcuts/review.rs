//! The foreign import review, in place of the table and the detail pane: the start, a search, what the set will change,
//! what did not match and why with a command to pick for each, the key codes the file leaves undecided, and after the
//! import the report of what was skipped, taken or failed.

use std::cell::OnceCell;
use std::cmp::Reverse;

use craft_i18n::matcher::Matcher;
use craft_keymap::chord::foreign::{ForeignError, ReadNote};
use craft_keymap::import::{FailedChange, NoChangeReason, Planned, SkipReason, Start, Suggestion, UnmatchedReason};
use craft_keymap::{Binding, CommandDef, CommandRef, ImportPlan, ImportReport};
use craft_theme::variant::Tone;
use craft_ui::buttons::Button;
use craft_ui::choice::{Checkbox, Radio, RadioGroup};
use craft_ui::contract::{Animate as _, Bind, BodyCx, Entry, FieldLayout, Lazy, Props as _, Show as _, ShowContainer as _, SideLabel, clicked};
use craft_ui::display::{Shade, Text};
use craft_ui::layout_components::{Row, Stack};
use craft_ui::notices::Notice;
use craft_ui::pickers::Select;
use craft_ui::runtime::commands::{ClipItem, Commands as _};
use craft_ui::runtime::env::Env as UiEnv;
use craft_ui::text::TextField;
use egui::Ui;

use super::recorder::ChordRecorder;
use super::state::{DialogState, Env, Target};
use super::view::recording_text;
use super::words::{Say, ShortcutWords, around_chord};

/// The separator between the parts of an entry's line.
const PART_SEP: &str = " · ";
/// The separator before an entry's note.
const NOTE_SEP: &str = " — ";
/// Most entry names listed beside a key code.
const CODE_ENTRIES: usize = 3;

/// One list row: the item, its line, its keys and a note.
struct Line {
    item: String,
    line: Option<usize>,
    keys: String,
    note: String,
    /// The note is what the item becomes (Will change), shown in the normal shade, not muted.
    to: bool,
    /// Chord text inside the note, kept out of bidi reordering.
    note_chord: String,
}

impl Line {
    fn new(item: String, line: Option<usize>) -> Self {
        Line { item, line, keys: String::new(), note: String::new(), to: false, note_chord: String::new() }
    }

    fn keys(mut self, keys: impl Into<String>) -> Self {
        self.keys = keys.into();
        self
    }

    fn note(mut self, note: impl Into<String>) -> Self {
        self.note = note.into();
        self
    }

    /// "item · line n · keys — note" in reading order.
    fn text(&self, say: &Say) -> String {
        let mut parts = vec![self.item.clone()];
        if let Some(n) = self.line {
            parts.extend([PART_SEP.to_owned(), say.words().line_n.fill(&[("n", n.to_string().as_str())])]);
        }
        if !self.keys.is_empty() {
            parts.extend([PART_SEP.to_owned(), self.keys.clone()]);
        }
        if !self.note.is_empty() {
            parts.extend([NOTE_SEP.to_owned(), around_chord(say.rtl(), &self.note, &self.note_chord)]);
        }
        if say.rtl() {
            parts.reverse();
        }
        parts.concat()
    }
}

/// What a control of the review asks for; `show` is the one place that turns it into a state change.
enum Act {
    Start(Start),
    Include(usize, bool),
    Target(usize, Option<CommandRef>),
    Record(String),
    Code(String, Option<String>),
    Import,
    Close,
    Copy(String),
}

/// An undecided key or modifier code, and what the plan made of it.
struct CodeRow {
    /// `key:N` or `mods:N`.
    code: String,
    /// `None` while the code is open; `Some(None)` once answered No equivalent.
    answer: Option<Option<String>>,
    entries: Vec<String>,
}

/// Shows the review while one is open.
pub(crate) fn show(ui: &mut Ui, cx: &mut BodyCx<'_>, env: &mut Env<'_>, state: &mut DialogState) {
    let Some(review) = state.review.as_ref() else { return };
    let (plan, name, done) = (review.plan.clone(), review.name.clone(), review.done.clone());
    let act = match &done {
        Some((set, report)) => after_report(ui, env, set, report),
        None => plan_view(ui, cx.now(), env, state, &plan, name),
    };
    match act {
        Some(Act::Start(start)) => state.set_start(env, start),
        Some(Act::Include(entry, on)) => state.include(env, entry, on),
        Some(Act::Target(entry, command)) => state.pick_target(env, entry, command),
        Some(Act::Record(code)) => state.record(env, Target::Code { code }),
        Some(Act::Code(code, answer)) => state.answer_code(env, code, answer),
        Some(Act::Import) => state.import_reviewed(env),
        Some(Act::Close) => state.close_review(),
        Some(Act::Copy(text)) => cx.clipboard().put(&[ClipItem::Text(text.into())]),
        None => {}
    }
}

fn heading(ui: &mut Ui, say: &Say, title: &str, n: usize) {
    let heading = say.words().title_n.fill(&[("title", title), ("n", n.to_string().as_str())]);
    Text::new(heading).role(craft_ui::style::TypeRole::Heading).show(ui);
}

fn section(ui: &mut Ui, say: &Say, title: &str, lines: &[Line]) {
    if lines.is_empty() {
        return;
    }
    heading(ui, say, title, lines.len());
    let estimate = UiEnv::of(ui).tokens.size.row.text;
    for (at, line) in lines.iter().enumerate() {
        // A line out of view does not run: a review can list thousands.
        Lazy::new((title, at), estimate).show(ui, |ui| {
            let shade = if line.to { Shade::Normal } else { Shade::Muted };
            Text::new(line.text(say)).shade(shade).id_salt((title, at)).show(ui);
        });
    }
}

fn unmatched_reason(say: &Say, r: UnmatchedReason) -> String {
    let w = say.words();
    match r {
        UnmatchedReason::NoMatch => w.no_command_has_this_name.as_str(),
        UnmatchedReason::Ambiguous => w.more_than_one_command_fits.as_str(),
        UnmatchedReason::Context => w.in_a_context_that_is_not.as_str(),
        UnmatchedReason::Tool => w.only_a_command_of_the_other.as_str(),
        _ => "",
    }
    .to_owned()
}

fn foreign_error(say: &Say, e: &ForeignError) -> String {
    let w = say.words();
    match e {
        ForeignError::Empty => w.empty_shortcut.as_str().to_owned(),
        ForeignError::TooLong => w.the_shortcut_is_too_long.as_str().to_owned(),
        ForeignError::UnknownKey(k) => w.unknown_key_key.fill(&[("key", k.as_str())]),
        ForeignError::UnsupportedModifier(m) => w.unsupported_modifier_key.fill(&[("key", m.as_str())]),
        ForeignError::Undecided { key: Some(n), .. } => w.undecided_key.fill(&[("n", n.to_string().as_str())]),
        ForeignError::Undecided { mods: Some(n), .. } => w.undecided_modifier.fill(&[("n", n.to_string().as_str())]),
        _ => String::new(),
    }
}

fn read_note(say: &Say, n: ReadNote) -> String {
    match n {
        ReadNote::Numpad => say.words().numeric_keypad_key_read_as_main.as_str().to_owned(),
        _ => String::new(),
    }
}

/// A binding as text, with its scope when that is not the command's home scope.
fn binding_text(say: &Say, env: &Env<'_>, b: &Binding, home: Option<&craft_keymap::Scope>) -> String {
    let chord = say.trigger(&b.trigger, env.keymap.platform());
    match &b.scope {
        Some(s) if Some(s) != home => say.words().chord_scope.fill(&[("chord", chord.as_str()), ("scope", say.scope_name(s).as_str())]),
        _ => chord,
    }
}

/// What the Start radios ask for: the keymap's bindings as they are, or every command's defaults.
fn start_choice(ui: &mut Ui, say: &Say, start: Start) -> Option<Act> {
    let w = say.words();
    let after = SideLabel::new().layout(FieldLayout::After);
    let group = RadioGroup::new(w.start.clone(), Bind::given(start == Start::Defaults)).show(ui, |ui, cx| {
        cx.labelled(ui, "current", w.start_current.clone(), after.clone(), Radio::new(false));
        cx.labelled(ui, "defaults", w.start_defaults.clone(), after.clone(), Radio::new(true));
    });
    group.output.value().copied().map(|defaults| Act::Start(if defaults { Start::Defaults } else { Start::Current }))
}

/// The command a target select value stands for: a catalog row, then No equivalent, then the line's own candidates.
fn target_of(catalog: &[CommandDef], candidates: &[CommandRef], value: usize) -> Option<Option<CommandRef>> {
    match value.cmp(&catalog.len()) {
        std::cmp::Ordering::Less => catalog.get(value).map(|c| Some(c.command.clone())),
        std::cmp::Ordering::Equal => Some(None),
        std::cmp::Ordering::Greater => candidates.get(value - catalog.len() - 1).cloned().map(Some),
    }
}

/// The 16 modifier combinations as (label, stored text), by number of modifiers then Cmd, Ctrl, Alt, Shift. Bit 3 is
/// Cmd, bit 2 Ctrl, bit 1 Alt, bit 0 Shift; the stored text joins the names with `+` (`""` for none).
fn modifier_choices(w: &ShortcutWords) -> Vec<(String, String)> {
    let names = [("Cmd", w.mod_cmd.as_str()), ("Ctrl", w.mod_ctrl.as_str()), ("Alt", w.mod_alt.as_str()), ("Shift", w.mod_shift.as_str())];
    let mut order: Vec<u8> = (0..16).collect();
    order.sort_by_key(|n| (n.count_ones(), Reverse(*n)));
    order
        .into_iter()
        .map(|n| {
            let held: Vec<_> = names.iter().enumerate().filter(|(i, _)| n & (8 >> i) != 0).map(|(_, name)| name).collect();
            if held.is_empty() {
                return (w.mod_none.as_str().to_owned(), String::new());
            }
            let label = held.iter().map(|(_, label)| *label).collect::<Vec<_>>().join("+");
            let stored = held.iter().map(|(stored, _)| *stored).collect::<Vec<_>>().join("+");
            (label, stored)
        })
        .collect()
}

/// The codes the report decoded or was answered, then the ones still open, each once with the entries that use it.
fn code_rows(report: &ImportReport) -> Vec<CodeRow> {
    let mut rows: Vec<CodeRow> =
        report.codes.iter().map(|d| CodeRow { code: d.code.clone(), answer: Some(d.text.clone()), entries: d.entries.clone() }).collect();
    for u in &report.unreadable {
        let ForeignError::Undecided { key, mods } = &u.reason else { continue };
        for code in key.map(|n| format!("key:{n}")).into_iter().chain(mods.map(|n| format!("mods:{n}"))) {
            match rows.iter_mut().find(|r| r.code == code) {
                Some(row) => {
                    if row.entries.len() < CODE_ENTRIES && !row.entries.contains(&u.item) {
                        row.entries.push(u.item.clone());
                    }
                }
                None => rows.push(CodeRow { code, answer: None, entries: vec![u.item.clone()] }),
            }
        }
    }
    rows
}

/// What a code row shows: the text it stands for, No equivalent, or the prompt while it is open.
fn code_text(w: &ShortcutWords, row: &CodeRow) -> String {
    match &row.answer {
        Some(Some(text)) => text.clone(),
        Some(None) => w.no_equivalent.as_str().to_owned(),
        None => w.record_a_key.as_str().to_owned(),
    }
}

/// The Keys section: for each code, a recorder (key codes) or the 16 modifier combinations (modifier codes), and No
/// equivalent.
fn keys_section(ui: &mut Ui, now: f64, env: &mut Env<'_>, state: &mut DialogState, rows: &[CodeRow]) -> Option<Act> {
    if rows.is_empty() {
        return None;
    }
    let say = env.text;
    let w = say.words();
    heading(ui, say, w.keys_section.as_str(), rows.len());
    let choices = modifier_choices(w);
    let entries: Vec<Entry<usize>> = choices.iter().enumerate().map(|(i, (label, _))| Entry::new(i, label.clone())).collect();
    let mut act = None;
    for (at, row) in rows.iter().enumerate() {
        Text::new(format!("{}{PART_SEP}{}", row.code, row.entries.join(", "))).shade(Shade::Muted).id_salt(("shortcuts.import.code", at)).show(ui);
        Row::new().id_salt(("shortcuts.import.codeRow", at)).show(ui, |ui, _| {
            if row.code.starts_with("mods:") {
                let mut chosen = usize::MAX;
                Select::new(row.code.clone(), Bind::bound(&mut chosen), &entries).placeholder(code_text(w, row)).id_salt("modifiers").show(ui);
                if let Some((_, stored)) = choices.get(chosen) {
                    act = Some(Act::Code(row.code.clone(), Some(stored.clone())));
                }
            } else {
                let recording = matches!(state.recording.as_ref().map(|r| &r.target), Some(Target::Code { code }) if *code == row.code);
                let shown = if recording { recording_text(env, state) } else { code_text(w, row) };
                let name = format!("{}: {}", w.record_a_key.as_str(), row.code);
                let out = ChordRecorder::new(name, recording, shown).id_salt("record").show(ui);
                if out.started && !recording {
                    act = Some(Act::Record(row.code.clone()));
                }
                if recording {
                    if let Some(recording) = state.recording.as_mut() {
                        recording.started = true;
                    }
                    if !out.chords.is_empty() {
                        state.feed(env, &out.chords, now);
                    } else if out.response.lost_focus() {
                        state.stop_recording();
                    }
                }
            }
            if clicked(&Button::new(w.no_equivalent.clone()).id_salt("noEquivalent").show(ui)) {
                act = Some(Act::Code(row.code.clone(), None));
            }
        });
    }
    act
}

/// The review of a plan: the counts, the set's name, Import and Cancel Import, the start and the search, then the lists
/// with their choices and the Keys section.
fn plan_view(ui: &mut Ui, now: f64, env: &mut Env<'_>, state: &mut DialogState, plan: &ImportPlan, mut name: String) -> Option<Act> {
    let say = env.text;
    let w = say.words();
    let ui_env = UiEnv::of(ui);
    let number = |n: usize| ui_env.format().integer(n as u64);
    let mut act = None;
    let report = &plan.report;
    let set = report.set.clone().unwrap_or_else(|| w.imported.as_str().to_owned());
    Text::new(w.import_set.fill(&[("set", set.as_str())])).role(craft_ui::style::TypeRole::Heading).show(ui);

    let mut counts = vec![w.count_will_change.text(plan.commands.len() as u64, &number(plan.commands.len()))];
    for (n, word) in [
        (report.unmatched.len(), &w.count_not_matched),
        (report.unmatched_without_shortcut, &w.count_without_shortcut),
        (report.unreadable.len(), &w.count_unreadable),
        (report.approximated.len(), &w.count_approximate),
        (report.truncated, &w.count_cut),
    ] {
        if n > 0 {
            counts.push(word.text(n as u64, &number(n)));
        }
    }
    Text::new(counts.join(PART_SEP)).shade(Shade::Muted).show(ui);

    Row::new().show(ui, |ui, _| {
        TextField::new(w.set_name.clone(), Bind::bound(&mut name)).id_salt("shortcuts.import.name").show(ui);
        let import = if env.sets.is_some() { w.import_as_new_set.clone() } else { w.import.clone() };
        let valid = env.sets.is_none() || craft_keymap::profiles::set_name(&name).is_some();
        if clicked(&Button::new(import).id_salt("shortcuts.import.ok").enabled(valid).show(ui)) {
            act = Some(Act::Import);
        }
        if clicked(&Button::new(w.cancel_import.clone()).id_salt("shortcuts.import.cancel").show(ui)) {
            act = Some(Act::Close);
        }
    });
    let (start, mut search) = state.review.as_ref().map(|r| (r.options.start, r.search.clone())).unwrap_or_default();
    if let Some(start_act) = start_choice(ui, say, start) {
        act = Some(start_act);
    }
    TextField::search(w.search_entries.clone(), Bind::bound(&mut search)).id_salt("shortcuts.import.search").show(ui);
    if let Some(review) = state.review.as_mut() {
        review.name = name;
        review.search = search.clone();
    }

    let matcher = Matcher::new(&say.folder(), search.trim());
    let shown = |line: &Line| search.trim().is_empty() || matcher.score(&format!("{} {} {}", line.item, line.keys, line.note)).is_some();

    let chord_list = |c: &Planned| -> String {
        let home = env.keymap.bindable(&c.command).map(|b| b.scope.clone());
        if c.bindings.is_empty() {
            return w.none.as_str().to_owned();
        }
        c.bindings.iter().map(|b| binding_text(say, env, &b.binding, home.as_ref())).collect::<Vec<_>>().join(", ")
    };
    let changes: Vec<Line> = plan
        .commands
        .iter()
        .map(|c| {
            let chords = chord_list(c);
            let mut line = Line::new(c.item.clone(), c.line)
                .note(w.label_chord.fill(&[("label", say.label_of(env.keymap, &c.command).as_str()), ("chord", chords.as_str())]));
            line.to = true;
            line.note_chord = chords;
            line
        })
        .filter(&shown)
        .collect();
    let unmatched: Vec<Line> = report
        .unmatched
        .iter()
        .map(|u| Line::new(u.item.clone(), u.line).keys(u.keys.clone()).note(unmatched_reason(say, u.reason)))
        .filter(&shown)
        .collect();
    let mut unreadable: Vec<Line> =
        report.unreadable.iter().map(|u| Line::new(u.item.clone(), u.line).keys(u.keys.clone()).note(foreign_error(say, &u.reason))).collect();
    // PLAN: words.rs has no "Not read" title; the file's unread lines share the unreadable list until shortcuts-w4 adds one.
    unreadable.extend(report.not_read.iter().map(|n| Line::new(n.text.clone(), n.line)));
    unreadable.retain(&shown);
    let [(skipped_title, skipped), ..] = report_lists(say, env, report);
    let mut skipped: Vec<Line> = skipped.into_iter().filter(&shown).collect();

    let Some(review) = state.review.as_ref() else { return act };
    // An entry left out is planned as if the set lacked it: it is listed with the skipped keys, its include box off.
    skipped.extend(
        review.set.entries.iter().zip(&review.included).filter(|(_, on)| !**on).map(|(entry, _)| Line::new(entry.item(), entry.line)).filter(&shown),
    );
    let entry_of = |item: &str, line: Option<usize>| review.entry_at(item, line).and_then(|i| review.included.get(i).map(|on| (i, *on)));
    let catalog = env.keymap.catalog();
    let shared: OnceCell<Vec<Entry<usize>>> = OnceCell::new();
    let shared_entries = || {
        shared.get_or_init(|| {
            let none = Entry::new(catalog.len(), w.no_equivalent.as_str().to_owned());
            catalog.iter().enumerate().map(|(i, c)| Entry::new(i, say.label_of(env.keymap, &c.command))).chain(std::iter::once(none)).collect()
        })
    };
    let suggestions: &[Suggestion] = &report.suggestions;

    let choice_lines = |ui: &mut Ui, tag: &'static str, title: &str, lines: &[Line], pick: bool, act: &mut Option<Act>| {
        if lines.is_empty() {
            return;
        }
        heading(ui, say, title, lines.len());
        for (at, line) in lines.iter().enumerate() {
            let marks = entry_of(&line.item, line.line);
            // A line out of view does not run: a review can list thousands.
            Lazy::new((tag, at), ui_env.tokens.size.control.md).show(ui, |ui| {
                Row::new().id_salt((tag, at)).show(ui, |ui, _| {
                    if let Some((entry, on)) = marks {
                        let label = format!("{}: {}", w.include_entry.as_str(), line.item);
                        let check = Checkbox::new(Bind::given(on)).label(label).show(ui);
                        if let Some(on) = check.value().copied() {
                            *act = Some(Act::Include(entry, on));
                        }
                    }
                    let shade = if line.to { Shade::Normal } else { Shade::Muted };
                    Text::new(line.text(say)).shade(shade).id_salt("text").show(ui);
                    let Some((entry, _)) = marks.filter(|_| pick) else { return };
                    let candidates: &[CommandRef] =
                        suggestions.iter().find(|s| s.item == line.item && s.line == line.line).map(|s| s.candidates.as_slice()).unwrap_or_default();
                    let own: Vec<Entry<usize>>;
                    let entries: &[Entry<usize>] = if candidates.is_empty() {
                        shared_entries()
                    } else {
                        let first = catalog.len() + 1;
                        own = candidates
                            .iter()
                            .enumerate()
                            .map(|(ci, c)| Entry::new(first + ci, say.label_of(env.keymap, c)))
                            .chain(shared_entries().iter().cloned())
                            .collect();
                        &own
                    };
                    let mut chosen = usize::MAX;
                    Select::new(w.command_for_item.fill(&[("item", line.item.as_str())]), Bind::bound(&mut chosen), entries)
                        .placeholder(w.choose_a_command_more.clone())
                        .searchable()
                        .id_salt("target")
                        .show(ui);
                    if let Some(target) = target_of(catalog, candidates, chosen) {
                        *act = Some(Act::Target(entry, target));
                    }
                });
            });
        }
    };

    choice_lines(ui, "shortcuts.import.changes", w.will_change.as_str(), &changes, false, &mut act);
    if !report.suggestions.is_empty() {
        Text::new(w.suggestions_n.fill(&[("n", number(report.suggestions.len()).as_str())])).role(craft_ui::style::TypeRole::Heading).show(ui);
    }
    choice_lines(ui, "shortcuts.import.unmatched", w.not_matched.as_str(), &unmatched, true, &mut act);
    choice_lines(ui, "shortcuts.import.unreadable", w.unreadable.as_str(), &unreadable, false, &mut act);
    choice_lines(ui, "shortcuts.import.skipped", &skipped_title, &skipped, false, &mut act);

    let approximated: Vec<Line> = report
        .approximated
        .iter()
        .map(|a| Line::new(a.item.clone(), a.line).keys(a.keys.clone()).note(read_note(say, a.note)))
        .filter(&shown)
        .collect();
    section(ui, say, w.approximated.as_str(), &approximated);
    let kept: Vec<Line> = report
        .matched_without_change
        .iter()
        .map(|m| {
            let note = match m.reason {
                NoChangeReason::PanelMenu => w.label_keeps_its_shortcuts_matched_by.fill(&[("label", say.label_of(env.keymap, &m.command).as_str())]),
                _ => String::new(),
            };
            Line::new(m.item.clone(), m.line).note(note)
        })
        .filter(&shown)
        .collect();
    section(ui, say, w.matched_without_change.as_str(), &kept);

    if let Some(keys_act) = keys_section(ui, now, env, state, &code_rows(report)) {
        act = Some(keys_act);
    }
    act
}

fn skip_note(say: &Say, env: &Env<'_>, reason: SkipReason, kept_by: Option<&CommandRef>) -> String {
    let w = say.words();
    match (reason, kept_by) {
        (SkipReason::Taken, Some(c)) => w.taken_by_label.fill(&[("label", say.label_of(env.keymap, c).as_str())]),
        (SkipReason::Taken, None) => w.taken_by_a_later_entry.as_str().to_owned(),
        (SkipReason::Reserved, _) => w.reserved_by_the_system.as_str().to_owned(),
        (SkipReason::UnknownScope, _) => w.the_scope_is_unknown.as_str().to_owned(),
        _ => String::new(),
    }
}

fn failed_note(say: &Say, change: FailedChange) -> String {
    let w = say.words();
    match change {
        FailedChange::Bind => w.the_sets_shortcuts_could_not_be.as_str().to_owned(),
        FailedChange::Clear => w.the_shortcuts_could_not_be_removed.as_str().to_owned(),
        _ => String::new(),
    }
}

/// The report's lists: skipped keys, shortcuts taken from other commands, failed changes.
fn report_lists(say: &Say, env: &Env<'_>, report: &ImportReport) -> [(String, Vec<Line>); 3] {
    let w = say.words();
    let skipped = report
        .skipped
        .iter()
        .map(|s| Line::new(s.item.clone(), s.line).keys(s.shortcut.clone()).note(skip_note(say, env, s.reason, s.kept_by.as_ref())))
        .collect();
    let cleared = report.conflicts_cleared.iter().map(|c| Line::new(say.label_of(env.keymap, &c.command), None).keys(c.shortcut.clone())).collect();
    let failed = report
        .failed
        .iter()
        .map(|f| Line::new(say.label_of(env.keymap, &f.command), None).keys(f.shortcuts.join(", ")).note(failed_note(say, f.change)))
        .collect();
    [
        (w.skipped.as_str().to_owned(), skipped),
        (w.taken_from_other_commands.as_str().to_owned(), cleared),
        (w.not_changed.as_str().to_owned(), failed),
    ]
}

/// The report after the import: what changed, the lists, Copy Report and Done.
fn after_report(ui: &mut Ui, env: &Env<'_>, set: &str, report: &ImportReport) -> Option<Act> {
    let say = env.text;
    let w = say.words();
    let ui_env = UiEnv::of(ui);
    let mut act = None;
    let heading = w.imported_as_set.fill(&[("set", set)]);
    Text::new(heading.clone()).role(craft_ui::style::TypeRole::Heading).show(ui);
    let changed = w.count_changed.text(report.imported as u64, &ui_env.format().integer(report.imported as u64));
    Notice::new(Tone::Info, changed.clone()).show(ui);
    let lists = report_lists(say, env, report);
    Stack::new().show(ui, |ui, _| {
        Row::new().show(ui, |ui, _| {
            if clicked(&Button::new(w.copy_report.clone()).id_salt("shortcuts.import.copy").show(ui)) {
                act = Some(Act::Copy(report_text(say, &heading, &changed, &lists)));
            }
            if clicked(&Button::new(w.done.clone()).id_salt("shortcuts.import.done").show(ui)) {
                act = Some(Act::Close);
            }
        });
        for (title, lines) in &lists {
            section(ui, say, title, lines);
        }
    });
    act
}

/// The report as plain text for Copy Report, in logical order.
fn report_text(say: &Say, heading: &str, changed: &str, lists: &[(String, Vec<Line>)]) -> String {
    let w = say.words();
    let mut out = format!("{heading}\n{changed}\n");
    for (title, lines) in lists.iter().filter(|(_, lines)| !lines.is_empty()) {
        out.push_str(&format!("\n{title}\n"));
        for line in lines {
            let mut text = format!("  {}", line.item);
            let numbered = line.line.map(|n| w.line_n.fill(&[("n", n.to_string().as_str())]));
            for part in [numbered.unwrap_or_default(), line.keys.clone()] {
                if !part.is_empty() {
                    text.push_str(PART_SEP);
                    text.push_str(&part);
                }
            }
            if !line.note.is_empty() {
                text.push_str(NOTE_SEP);
                text.push_str(&line.note);
            }
            out.push_str(&text);
            out.push('\n');
        }
    }
    out
}
