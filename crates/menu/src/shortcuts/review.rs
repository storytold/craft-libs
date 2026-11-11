//! The foreign import review, in place of the table and the detail pane: what the set will change, what did not
//! match and why, suggestions to pick from, and after the import the report of what was skipped, taken or failed.

use craft_keymap::chord::foreign::{ForeignError, ReadNote};
use craft_keymap::import::{FailedChange, NoChangeReason, Planned, SkipReason, UnmatchedReason};
use craft_keymap::{Binding, CommandRef, ImportPlan, ImportReport};
use craft_theme::variant::Tone;
use craft_ui::buttons::Button;
use craft_ui::contract::{Bind, BodyCx, Entry, Props as _, Show as _, ShowContainer as _, clicked};
use craft_ui::display::{Shade, Text};
use craft_ui::layout_components::{Row, Stack};
use craft_ui::notices::Notice;
use craft_ui::pickers::Select;
use craft_ui::runtime::commands::{ClipItem, Commands as _};
use craft_ui::runtime::env::Env as UiEnv;
use craft_ui::text::TextField;
use egui::Ui;

use super::state::{DialogState, Env};
use super::words::{Say, around_chord};

/// The separator between the parts of an entry's line.
const PART_SEP: &str = " · ";
/// The separator before an entry's note.
const NOTE_SEP: &str = " — ";

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

enum Act {
    Pick(usize, usize),
    Import,
    Close,
    Copy(String),
}

/// Shows the review while one is open.
pub(crate) fn show(ui: &mut Ui, cx: &mut BodyCx<'_>, env: &mut Env<'_>, state: &mut DialogState) {
    let Some(review) = state.review.as_ref() else { return };
    let (plan, name, done) = (review.plan.clone(), review.name.clone(), review.done.clone());
    let act = match &done {
        Some((set, report)) => after_report(ui, env, set, report),
        None => plan_view(ui, env, state, &plan, name),
    };
    match act {
        Some(Act::Pick(s, c)) => state.pick_suggestion(env, s, c),
        Some(Act::Import) => state.import_reviewed(env),
        Some(Act::Close) => state.close_review(),
        Some(Act::Copy(text)) => cx.clipboard().put(&[ClipItem::Text(text.into())]),
        None => {}
    }
}

fn section(ui: &mut Ui, say: &Say, title: &str, lines: &[Line]) {
    if lines.is_empty() {
        return;
    }
    let heading = say.words().title_n.fill(&[("title", title), ("n", lines.len().to_string().as_str())]);
    Text::new(heading).role(craft_ui::style::TypeRole::Heading).show(ui);
    for (at, line) in lines.iter().enumerate() {
        let shade = if line.to { Shade::Normal } else { Shade::Muted };
        Text::new(line.text(say)).shade(shade).id_salt((title, at)).show(ui);
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

/// The review of a plan: the counts, the set's name, Import and Cancel Import, then the lists.
fn plan_view(ui: &mut Ui, env: &Env<'_>, state: &mut DialogState, plan: &ImportPlan, mut name: String) -> Option<Act> {
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
    if let Some(review) = state.review.as_mut() {
        review.name = name;
    }

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
        .collect();
    section(ui, say, w.will_change.as_str(), &changes);

    if !report.suggestions.is_empty() {
        Text::new(w.suggestions_n.fill(&[("n", number(report.suggestions.len()).as_str())])).role(craft_ui::style::TypeRole::Heading).show(ui);
        for (si, s) in report.suggestions.iter().enumerate() {
            Row::new().show(ui, |ui, _| {
                Text::new(Line::new(s.item.clone(), s.line).text(say)).shade(Shade::Muted).id_salt(("shortcuts.import.item", si)).show(ui);
                let entries: Vec<Entry<usize>> = s.candidates.iter().enumerate().map(|(ci, c)| Entry::new(ci, say.label_of(env.keymap, c))).collect();
                let mut chosen = usize::MAX;
                Select::new(w.command_for_item.fill(&[("item", s.item.as_str())]), Bind::bound(&mut chosen), &entries)
                    .placeholder(w.choose_a_command_more.clone())
                    .id_salt(("shortcuts.import.suggestion", si))
                    .show(ui);
                if chosen != usize::MAX {
                    act = Some(Act::Pick(si, chosen));
                }
            });
        }
    }

    let unmatched: Vec<Line> =
        report.unmatched.iter().map(|u| Line::new(u.item.clone(), u.line).keys(u.keys.clone()).note(unmatched_reason(say, u.reason))).collect();
    section(ui, say, w.not_matched.as_str(), &unmatched);
    let unreadable: Vec<Line> =
        report.unreadable.iter().map(|u| Line::new(u.item.clone(), u.line).keys(u.keys.clone()).note(foreign_error(say, &u.reason))).collect();
    section(ui, say, w.unreadable.as_str(), &unreadable);
    let approximated: Vec<Line> =
        report.approximated.iter().map(|a| Line::new(a.item.clone(), a.line).keys(a.keys.clone()).note(read_note(say, a.note))).collect();
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
        .collect();
    section(ui, say, w.matched_without_change.as_str(), &kept);
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
