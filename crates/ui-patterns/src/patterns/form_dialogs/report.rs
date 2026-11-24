//! `ReportDialog`: a summary line, the count of notes and the notes as notices, with a Copy button that hands the
//! notes back as text (an import's warnings, a batch's results).

use crate::contract::{Bind, NoValue, Outcome, Props as _, Show as _, ShowContainer as _};
use crate::dialog::{Dialog, DialogOutput};
use crate::display::{Shade, Text};
use crate::layout_components::Stack;
use crate::notices::{Notice, NoticeLayout};
use crate::runtime::env::{Env, Label};
use crate::runtime::overlay::CloseReason;
use crate::runtime::words::CommonWords;
use crate::sections::{DialogRole, FooterButton};
use crate::style::TypeRole;
use crate::text::SelectableText;
use craft_theme::variant::Tone;
use egui::{Response, Ui};
use serde_json::{Value, json};

#[cfg(feature = "suite")]
pub(crate) mod stories;
#[cfg(feature = "suite")]
pub(crate) mod trials;

craft_ui::words! {
    /// The words of the report dialog beside the common OK.
    pub struct ReportWords {
        count: count "reportDialog.notes" = "{n} note" | "{n} notes";
        copy: "reportDialog.copy" = "Copy";
        title: "reportDialog.title" = "Report";
        info: "reportDialog.info" = "Info";
        warning: "reportDialog.warning" = "Warning";
        error: "reportDialog.error" = "Error";
        success: "reportDialog.success" = "Success";
        note: "reportDialog.note" = "Note";
        line: template "reportDialog.line" = "{tone}: {text}";
        line_detail: template "reportDialog.lineDetail" = "{tone}: {text} ({detail})";
    }
}

/// One line of the report.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub struct ReportNote {
    /// The note's weight: the notice's icon and colours, and the word that starts its line when the notes are copied.
    pub tone: Tone,
    /// The note's text.
    pub text: Label,
    /// A muted second part after the text (a place such as "line 3", a count).
    pub detail: Option<Label>,
    /// The text is shown in the mono type role.
    pub code: bool,
}

impl ReportNote {
    /// A note of weight `tone` with `text`.
    pub fn new(tone: Tone, text: impl Into<Label>) -> Self {
        ReportNote { tone, text: text.into(), detail: None, code: false }
    }

    /// A muted second part after the text.
    #[must_use]
    pub fn detail(self, detail: impl Into<Label>) -> Self {
        ReportNote { detail: Some(detail.into()), ..self }
    }

    /// Shows the text in the mono type role.
    #[must_use]
    pub fn code(self) -> Self {
        ReportNote { code: true, ..self }
    }
}

/// A heading over its notes.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub struct ReportGroup {
    /// The group's heading.
    pub heading: Label,
    /// The notes under it.
    pub notes: Vec<ReportNote>,
}

impl ReportGroup {
    /// A group under `heading` with `notes`.
    pub fn new(heading: impl Into<Label>, notes: Vec<ReportNote>) -> Self {
        ReportGroup { heading: heading.into(), notes }
    }
}

/// What a report dialog reports.
#[derive(Debug)]
#[non_exhaustive]
pub struct ReportOutput {
    /// The dialog's response.
    pub response: Response,
    /// How the dialog closed in this pass; `None` while it stays open.
    pub closed: Option<CloseReason>,
    /// The notes as text, the pass the Copy button was pressed: the app puts it on the clipboard.
    pub copied: Option<String>,
}

impl Outcome for ReportOutput {
    type Value = NoValue;

    fn response(&self) -> &Response {
        &self.response
    }
}

/// A report: the title, one summary line, the count of notes, the notes with their tone icons and, in the footer,
/// Copy and OK. Enter and Escape both close it; the body scrolls when the notes outgrow the window.
#[must_use = "show the dialog with `show`"]
#[derive(crate::contract::Pattern, crate::contract::Props)]
pub struct ReportDialog<'a> {
    title: Label,
    summary: Label,
    notes: Vec<ReportNote>,
    /// Plain `lines` shown as the notes, in place of ones built with [`ReportNote`]: each line is selectable text,
    /// and Copy hands them back as they are.
    #[prop]
    lines: Vec<Label>,
    /// Groups of notes shown after the flat notes: each heading, then its notes. The note count counts every note.
    #[prop]
    groups: Vec<ReportGroup>,
    /// Binds the open state to `open`.
    #[prop(name = "opened")]
    open: Option<Bind<'a, bool>>,
}

impl<'a> ReportDialog<'a> {
    /// A report titled `title` with one `summary` line and the `notes`.
    pub fn new(title: impl Into<Label>, summary: impl Into<Label>, notes: Vec<ReportNote>) -> Self {
        ReportDialog { title: title.into(), summary: summary.into(), notes, lines: Vec::new(), groups: Vec::new(), open: None }
    }

    /// The notes as the Copy button hands them back, one per line with the tone's word first (a note with a detail
    /// ends with it in brackets); plain [`lines`](ReportDialog::lines) one per line as they are. Each group follows
    /// with its heading line, then its notes.
    pub fn as_text(&self, env: &Env) -> String {
        let view = env.view();
        let words = env.words::<ReportWords>();
        let text = |label: &Label| view.resolve(label).as_str().to_owned();
        let line = |note: &ReportNote| {
            let word = match note.tone {
                Tone::Info => &words.info,
                Tone::Warn => &words.warning,
                Tone::Bad => &words.error,
                Tone::Good => &words.success,
                _ => &words.note,
            };
            let (tone, body) = (text(word), text(&note.text));
            match &note.detail {
                Some(detail) => words.line_detail.fill(&[("tone", tone.as_str()), ("text", body.as_str()), ("detail", text(detail).as_str())]),
                None => words.line.fill(&[("tone", tone.as_str()), ("text", body.as_str())]),
            }
        };
        let mut out: Vec<String> =
            if self.lines.is_empty() { self.notes.iter().map(&line).collect() } else { self.lines.iter().map(&text).collect() };
        for group in &self.groups {
            out.push(text(&group.heading));
            out.extend(group.notes.iter().map(&line));
        }
        out.join("\n")
    }

    /// Shows the report.
    pub fn show(self, ui: &mut Ui) -> ReportOutput {
        let env = Env::of(ui);
        let words = env.words::<ReportWords>();
        let common = env.words::<CommonWords>();
        let text = self.as_text(&env);
        let ReportDialog { title, summary, notes, lines, groups, open } = self;
        let accept = FooterButton::new(common.ok.clone(), DialogRole::Accept);
        let copy = FooterButton::new(words.copy.clone(), DialogRole::Other).stays_open(true);
        let mut dialog = Dialog::new(title).buttons([accept, copy]);
        if let Some(open) = open {
            dialog = dialog.opened(open);
        }
        let grouped: usize = groups.iter().map(|group| group.notes.len()).sum();
        let shown_count = (if lines.is_empty() { notes.len() } else { lines.len() }) + grouped;
        let count = (shown_count > 0).then(|| {
            let n = shown_count as u64;
            words.count.text(n, &env.format().integer(n))
        });
        let view = env.view();
        let lines: Vec<String> = lines.iter().map(|line| view.resolve(line).as_str().to_owned()).collect();
        let shown = dialog.show(ui, |ui, _| {
            Stack::new().show(ui, |ui, _| {
                Text::new(summary.clone()).show(ui);
                if let Some(count) = &count {
                    Text::new(count.clone()).role(TypeRole::Caption).shade(Shade::Muted).show(ui);
                }
                let mut at = 0;
                if lines.is_empty() {
                    for note in &notes {
                        show_note(ui, note, at);
                        at += 1;
                    }
                }
                for group in &groups {
                    Text::new(group.heading.clone()).role(TypeRole::Heading).show(ui);
                    for note in &group.notes {
                        show_note(ui, note, at);
                        at += 1;
                    }
                }
                for (at, line) in lines.iter().enumerate() {
                    SelectableText::new(words.note.clone(), line).multiline().id_salt(("line", at)).show(ui);
                }
            });
        });
        let DialogOutput { response, closed, picked } = shown.output;
        ReportOutput { response, closed, copied: (picked == Some(COPY)).then_some(text) }
    }
}

/// Shows `note`: a notice of its tone, with a detail as a line under the text. `at` tells the notes of one dialog apart.
fn show_note(ui: &mut Ui, note: &ReportNote, at: usize) {
    if note.code {
        // PLAN: Notice has no type-role slot, so a code note shows its text in the mono role without the tone icon.
        Text::new(note.text.clone()).role(TypeRole::Mono).show(ui);
        if let Some(detail) = &note.detail {
            Text::new(detail.clone()).role(TypeRole::Caption).shade(Shade::Muted).show(ui);
        }
        return;
    }
    let mut notice = Notice::new(note.tone, note.text.clone()).layout(NoticeLayout::Inline).id_salt(at);
    if let Some(detail) = &note.detail {
        notice = notice.lines([detail.clone()]);
    }
    notice.show(ui);
}

/// A [`ReportDialog`] the [`DialogHost`](crate::dialog::DialogHost) owns: a title, a summary, plain lines or notes, and
/// groups of notes. The text of the Copy button waits in [`take_copied`](HostedReport::take_copied) until the app puts it on the
/// clipboard.
pub struct HostedReport {
    id: crate::dialog::DialogId,
    title: Label,
    summary: Label,
    lines: Vec<String>,
    groups: Vec<ReportGroup>,
    open: bool,
    copied: Option<String>,
    /// The notes shown as notices; plain `lines` take their place when there are any.
    notes: Vec<ReportNote>,
}

impl HostedReport {
    /// A report titled `title` with one `summary` line and the `lines`.
    pub fn new(title: impl Into<Label>, summary: impl Into<Label>, lines: Vec<String>) -> Self {
        HostedReport {
            id: crate::dialog::dialog_id("report"),
            title: title.into(),
            summary: summary.into(),
            lines,
            groups: Vec::new(),
            open: true,
            copied: None,
            notes: Vec::new(),
        }
    }

    /// Shows `notes` with the report; `json_fields` lists them.
    #[must_use]
    pub fn notes(mut self, notes: Vec<ReportNote>) -> Self {
        self.notes = notes;
        self
    }

    /// Shows and copies `groups` after the lines; `json_fields` lists their headings and note texts.
    #[must_use]
    pub fn groups(mut self, groups: Vec<ReportGroup>) -> Self {
        self.groups = groups;
        self
    }

    /// The text the Copy button copied since the last call, once. The app puts it on the clipboard.
    pub fn take_copied(&mut self) -> Option<String> {
        self.copied.take()
    }
}

impl crate::dialog::StackDialog for HostedReport {
    type Result = ();

    fn id(&self) -> crate::dialog::DialogId {
        self.id
    }

    fn show(&mut self, ui: &mut Ui, _: &crate::dialog::DialogStack) -> Option<crate::dialog::DialogResult<()>> {
        let lines = self.lines.iter().map(|line| Label::from(line.clone())).collect();
        let shown = ReportDialog {
            title: self.title.clone(),
            summary: self.summary.clone(),
            notes: self.notes.clone(),
            lines,
            groups: self.groups.clone(),
            open: Some(Bind::bound(&mut self.open)),
        }
        .show(ui);
        if let Some(text) = shown.copied {
            self.copied = Some(text);
        }
        shown.closed.map(|_| crate::dialog::DialogResult::Ok(()))
    }

    fn confirm(&mut self, _: &egui::Context) -> Option<Self::Result> {
        Some(())
    }

    fn json_fields(&self) -> Option<Value> {
        let notes: Vec<Value> = self.notes.iter().map(|note| json!({ "tone": note.tone.name(), "text": note.text.as_str() })).collect();
        let groups: Vec<Value> = self
            .groups
            .iter()
            .map(|group| json!({ "heading": group.heading.as_str(), "notes": group.notes.iter().map(|note| note.text.as_str()).collect::<Vec<_>>() }))
            .collect();
        Some(json!({ "lines": self.lines, "notes": notes, "groups": groups }))
    }
}

/// The position of the Copy button among the footer buttons.
const COPY: usize = 1;
