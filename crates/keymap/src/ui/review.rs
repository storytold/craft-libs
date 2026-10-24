//! The foreign import review (§2.7 of the dialog plan). It takes the place of the table and
//! detail pane: what the set will change, what did not match and why, suggestions to pick from,
//! and after the import the report of what was skipped, taken or failed.

use egui::text::{LayoutJob, TextFormat};
use egui::{ComboBox, Label, Rect, RichText, ScrollArea, TextEdit, TextStyle, Ui};

use super::{Draw, FIELD_W, fill};
use crate::chord::foreign::{ForeignError, ReadNote};
use crate::command::CommandRef;
use crate::import::{FailedChange, ImportPlan, ImportReport, NoChangeReason, SkipReason, UnmatchedReason};

/// One list row: the item, its line, its keys and a note.
struct Entry {
    item: String,
    line: Option<usize>,
    keys: String,
    note: String,
    /// The note is what the item becomes (Will change), drawn in the text color, not dimmed.
    to: bool,
    /// Chord text inside the note, kept out of bidi reordering.
    note_chord: String,
}

enum Act {
    Pick(usize, usize),
    Import,
    Close,
    Copy(String),
}

impl Draw<'_, '_> {
    /// Draws the review in `rect` while one is open; returns whether it drew.
    pub(super) fn review(&mut self, ui: &mut Ui, rect: Rect) -> bool {
        let Some(r) = self.state.review.as_ref() else { return false };
        let (plan, name, done) = (r.plan.clone(), r.name.clone(), r.done.clone());
        let mut act = None;
        self.child(ui, rect, |d, ui| {
            act = match &done {
                Some((set, report)) => d.after_report(ui, set, report),
                None => d.plan_view(ui, &plan, name),
            };
        });
        match act {
            Some(Act::Pick(s, c)) => self.state.pick_suggestion(self.env, s, c),
            Some(Act::Import) => self.state.import_reviewed(self.env),
            Some(Act::Close) => self.state.close_review(),
            Some(Act::Copy(text)) => ui.ctx().copy_text(text),
            None => {}
        }
        true
    }

    fn line_text(&self, line: Option<usize>) -> Option<String> {
        line.map(|n| fill(self.ui("line {n}"), &[("n", &n.to_string())]))
    }

    /// "item · line n · keys — note" on one wrapping line, the item in the strong color.
    fn entry(&self, ui: &mut Ui, e: &Entry) {
        let font = TextStyle::Body.resolve(ui.style());
        let strong = ui.visuals().strong_text_color();
        let dim = self.style.dim;
        let text = ui.visuals().text_color();
        let mut parts = vec![(self.vis(&e.item), strong)];
        if let Some(line) = self.line_text(e.line) {
            parts.extend([(" · ".to_string(), dim), (self.vis(&line), dim)]);
        }
        if !e.keys.is_empty() {
            parts.extend([(" · ".to_string(), dim), (e.keys.clone(), text)]);
        }
        if !e.note.is_empty() {
            let color = if e.to { text } else { dim };
            parts.extend([(" — ".to_string(), color), (self.vis_chord(&e.note, &e.note_chord), color)]);
        }
        let mut job = LayoutJob::default();
        for (s, color) in self.ordered(parts) {
            job.append(&s, 0.0, TextFormat::simple(font.clone(), color));
        }
        ui.add(Label::new(job).wrap());
    }

    fn section(&self, ui: &mut Ui, title: &str, entries: &[Entry]) {
        if entries.is_empty() {
            return;
        }
        ui.add_space(4.0);
        ui.label(RichText::new(self.vis(&fill(self.ui("{title} ({n})"), &[("title", title), ("n", &entries.len().to_string())]))).strong());
        for e in entries {
            self.entry(ui, e);
        }
    }

    fn unmatched_reason(&self, r: UnmatchedReason) -> String {
        match r {
            UnmatchedReason::NoMatch => self.ui("no command has this name"),
            UnmatchedReason::Ambiguous => self.ui("more than one command fits"),
            UnmatchedReason::Context => self.ui("in a context that is not imported"),
            UnmatchedReason::Tool => self.ui("only a command of the other kind fits (tool or menu command)"),
        }
    }

    fn foreign_error(&self, e: &ForeignError) -> String {
        match e {
            ForeignError::Empty => self.ui("empty shortcut"),
            ForeignError::TooLong => self.ui("the shortcut is too long"),
            ForeignError::UnknownKey(k) => fill(self.ui("unknown key {key}"), &[("key", k)]),
            ForeignError::UnsupportedModifier(m) => fill(self.ui("unsupported modifier {key}"), &[("key", m)]),
        }
    }

    fn read_note(&self, n: ReadNote) -> String {
        match n {
            ReadNote::Numpad => self.ui("numeric keypad key read as main key"),
        }
    }

    fn plan_view(&mut self, ui: &mut Ui, plan: &ImportPlan, mut name: String) -> Option<Act> {
        let mut act = None;
        let report = &plan.report;
        let set = report.set.clone().unwrap_or_else(|| self.ui("Imported"));
        ui.label(RichText::new(self.vis(&fill(self.ui("Import “{set}”"), &[("set", &set)]))).strong());

        let t = self.env.text;
        let mut counts = vec![t.plural(plan.commands.len() as u64, "{n} command will change", "{n} commands will change")];
        let more = [
            (report.unmatched.len(), t.plural(report.unmatched.len() as u64, "{n} item not matched", "{n} items not matched")),
            (
                report.unmatched_without_shortcut,
                t.plural(report.unmatched_without_shortcut as u64, "{n} item without a shortcut", "{n} items without a shortcut"),
            ),
            (report.unreadable.len(), t.plural(report.unreadable.len() as u64, "{n} item could not be read", "{n} items could not be read")),
            (report.approximated.len(), t.plural(report.approximated.len() as u64, "{n} key read approximately", "{n} keys read approximately")),
            (report.truncated, t.plural(report.truncated as u64, "{n} entry cut at a limit", "{n} entries cut at a limit")),
        ];
        counts.extend(more.into_iter().filter(|(n, _)| *n > 0).map(|(_, s)| s));
        ui.label(RichText::new(self.vis(&counts.join(" · "))).color(self.style.dim));

        ui.horizontal_wrapped(|ui| {
            let name_label = self.ui("Set name:");
            let l = ui.label(self.vis(&name_label));
            let edit = ui.add(TextEdit::singleline(&mut name).id(self.base.with("import_name")).desired_width(FIELD_W)).labelled_by(l.id);
            self.text_field(&edit);
            self.tag(|| "shortcuts.import.name".into(), edit.rect, &name_label);
            let import = if self.env.sets.is_some() { self.ui("Import as New Set") } else { self.ui("Import") };
            let valid = self.env.sets.is_none() || crate::profiles::set_name(&name).is_some();
            if self.button(ui, valid, &import, "shortcuts.import.ok") {
                act = Some(Act::Import);
            }
            if self.button(ui, true, &self.ui("Cancel Import"), "shortcuts.import.cancel") {
                act = Some(Act::Close);
            }
        });
        if let Some(r) = self.state.review.as_mut() {
            r.name = name;
        }

        ScrollArea::vertical().id_salt(self.base.with("review")).auto_shrink([false, false]).show(ui, |ui| {
            let chord_list = |d: &Self, c: &crate::import::Planned| -> String {
                let home = d.env.keymap.bindable(&c.command).map(|b| b.scope.clone());
                if c.bindings.is_empty() {
                    return d.ui("None");
                }
                c.bindings.iter().map(|b| d.binding_text(&b.binding, home.as_ref())).collect::<Vec<_>>().join(", ")
            };
            let changes: Vec<Entry> = plan
                .commands
                .iter()
                .map(|c| {
                    let chords = chord_list(self, c);
                    Entry {
                        item: c.item.clone(),
                        line: c.line,
                        keys: String::new(),
                        note: fill(self.ui("{label}: {chord}"), &[("label", &self.label_of(&c.command)), ("chord", &chords)]),
                        to: true,
                        note_chord: chords,
                    }
                })
                .collect();
            self.section(ui, &self.ui("Will change"), &changes);

            if !report.suggestions.is_empty() {
                ui.add_space(4.0);
                let title = fill(self.ui("Suggestions ({n})"), &[("n", &report.suggestions.len().to_string())]);
                ui.label(RichText::new(self.vis(&title)).strong());
                for (si, s) in report.suggestions.iter().enumerate() {
                    ui.horizontal_wrapped(|ui| {
                        self.entry(
                            ui,
                            &Entry {
                                item: s.item.clone(),
                                line: s.line,
                                keys: String::new(),
                                note: String::new(),
                                to: false,
                                note_chord: String::new(),
                            },
                        );
                        let label = fill(self.ui("Command for {item}"), &[("item", &s.item)]);
                        let choose = self.ui("Choose a command…");
                        let combo = ComboBox::from_id_salt(self.base.with(("suggestion", si))).selected_text(self.vis(&choose)).show_ui(ui, |ui| {
                            for (ci, c) in s.candidates.iter().enumerate() {
                                if ui.selectable_label(false, self.vis(&self.label_of(c))).clicked() {
                                    act = Some(Act::Pick(si, ci));
                                }
                            }
                        });
                        let resp = combo.response.on_hover_text(self.vis(&label));
                        resp.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::ComboBox, true, &label));
                        self.tag(|| format!("shortcuts.import.suggestion.{si}"), resp.rect, &label);
                    });
                }
            }

            let unmatched: Vec<Entry> = report
                .unmatched
                .iter()
                .map(|u| Entry {
                    item: u.item.clone(),
                    line: u.line,
                    keys: u.keys.clone(),
                    note: self.unmatched_reason(u.reason),
                    to: false,
                    note_chord: String::new(),
                })
                .collect();
            self.section(ui, &self.ui("Not matched"), &unmatched);
            let unreadable: Vec<Entry> = report
                .unreadable
                .iter()
                .map(|u| Entry {
                    item: u.item.clone(),
                    line: u.line,
                    keys: u.keys.clone(),
                    note: self.foreign_error(&u.reason),
                    to: false,
                    note_chord: String::new(),
                })
                .collect();
            self.section(ui, &self.ui("Unreadable"), &unreadable);
            let approximated: Vec<Entry> = report
                .approximated
                .iter()
                .map(|a| Entry {
                    item: a.item.clone(),
                    line: a.line,
                    keys: a.keys.clone(),
                    note: self.read_note(a.note),
                    to: false,
                    note_chord: String::new(),
                })
                .collect();
            self.section(ui, &self.ui("Approximated"), &approximated);
            let kept: Vec<Entry> = report
                .matched_without_change
                .iter()
                .map(|m| Entry {
                    item: m.item.clone(),
                    line: m.line,
                    keys: String::new(),
                    note: match m.reason {
                        NoChangeReason::PanelMenu => {
                            fill(self.ui("{label} keeps its shortcuts (matched by a panel menu name)"), &[("label", &self.label_of(&m.command))])
                        }
                    },
                    to: false,
                    note_chord: String::new(),
                })
                .collect();
            self.section(ui, &self.ui("Matched without change"), &kept);
        });
        act
    }

    fn skip_note(&self, reason: SkipReason, kept_by: Option<&CommandRef>) -> String {
        match (reason, kept_by) {
            (SkipReason::Taken, Some(c)) => fill(self.ui("taken by {label}"), &[("label", &self.label_of(c))]),
            (SkipReason::Taken, None) => self.ui("taken by a later entry"),
            (SkipReason::Reserved, _) => self.ui("reserved by the system"),
            (SkipReason::UnknownScope, _) => self.ui("the scope is unknown"),
        }
    }

    fn failed_note(&self, change: FailedChange) -> String {
        match change {
            FailedChange::Bind => self.ui("the set's shortcuts could not be added"),
            FailedChange::Clear => self.ui("the shortcuts could not be removed"),
        }
    }

    /// The report's lists: skipped keys, shortcuts taken from other commands, failed changes.
    fn report_lists(&self, report: &ImportReport) -> [(String, Vec<Entry>); 3] {
        let skipped = report
            .skipped
            .iter()
            .map(|s| Entry {
                item: s.item.clone(),
                line: s.line,
                keys: s.shortcut.clone(),
                note: self.skip_note(s.reason, s.kept_by.as_ref()),
                to: false,
                note_chord: String::new(),
            })
            .collect();
        let cleared = report
            .conflicts_cleared
            .iter()
            .map(|c| Entry {
                item: self.label_of(&c.command),
                line: None,
                keys: c.shortcut.clone(),
                note: String::new(),
                to: false,
                note_chord: String::new(),
            })
            .collect();
        let failed = report
            .failed
            .iter()
            .map(|f| Entry {
                item: self.label_of(&f.command),
                line: None,
                keys: f.shortcuts.join(", "),
                note: self.failed_note(f.change),
                to: false,
                note_chord: String::new(),
            })
            .collect();
        [(self.ui("Skipped"), skipped), (self.ui("Taken from other commands"), cleared), (self.ui("Not changed"), failed)]
    }

    fn after_report(&mut self, ui: &mut Ui, set: &str, report: &ImportReport) -> Option<Act> {
        let mut act = None;
        let heading = fill(self.ui("Imported as “{set}”"), &[("set", set)]);
        ui.label(RichText::new(self.vis(&heading)).strong());
        let changed = self.env.text.plural(report.imported as u64, "{n} command changed", "{n} commands changed");
        ui.label(RichText::new(self.vis(&changed)).color(self.style.dim));
        let lists = self.report_lists(report);
        ui.horizontal_wrapped(|ui| {
            if self.button(ui, true, &self.ui("Copy Report"), "shortcuts.import.copy") {
                act = Some(Act::Copy(self.report_text(&heading, &changed, &lists)));
            }
            if self.button(ui, true, &self.ui("Done"), "shortcuts.import.done") {
                act = Some(Act::Close);
            }
        });
        ScrollArea::vertical().id_salt(self.base.with("report")).auto_shrink([false, false]).show(ui, |ui| {
            for (title, entries) in &lists {
                self.section(ui, title, entries);
            }
        });
        act
    }

    /// The report as plain text for Copy Report (logical order).
    fn report_text(&self, heading: &str, changed: &str, lists: &[(String, Vec<Entry>)]) -> String {
        let mut out = format!("{heading}\n{changed}\n");
        for (title, entries) in lists.iter().filter(|(_, e)| !e.is_empty()) {
            out.push_str(&format!("\n{title}\n"));
            for e in entries {
                let mut line = format!("  {}", e.item);
                for part in [self.line_text(e.line).unwrap_or_default(), e.keys.clone()] {
                    if !part.is_empty() {
                        line.push_str(" · ");
                        line.push_str(&part);
                    }
                }
                if !e.note.is_empty() {
                    line.push_str(" — ");
                    line.push_str(&e.note);
                }
                out.push_str(&line);
                out.push('\n');
            }
        }
        out
    }
}
