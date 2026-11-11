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
use craft_theme::variant::Tone;
use egui::{Response, Ui};

#[cfg(feature = "testing")]
mod stories;

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
}

impl ReportNote {
    /// A note of weight `tone` with `text`.
    pub fn new(tone: Tone, text: impl Into<Label>) -> Self {
        ReportNote { tone, text: text.into() }
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
    /// Binds the open state to `open`.
    #[prop(name = "opened")]
    open: Option<Bind<'a, bool>>,
}

impl<'a> ReportDialog<'a> {
    /// A report titled `title` with one `summary` line and the `notes`.
    pub fn new(title: impl Into<Label>, summary: impl Into<Label>, notes: Vec<ReportNote>) -> Self {
        ReportDialog { title: title.into(), summary: summary.into(), notes, open: None }
    }

    /// The notes as the Copy button hands them back, one per line with the tone's word first.
    pub fn as_text(&self, env: &Env) -> String {
        let words = env.words::<ReportWords>();
        let view = env.view();
        self.notes
            .iter()
            .map(|note| {
                let word = match note.tone {
                    Tone::Info => &words.info,
                    Tone::Warn => &words.warning,
                    Tone::Bad => &words.error,
                    Tone::Good => &words.success,
                    _ => &words.note,
                };
                words.line.fill(&[("tone", view.resolve(word).as_str()), ("text", view.resolve(&note.text).as_str())])
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    /// Shows the report.
    pub fn show(self, ui: &mut Ui) -> ReportOutput {
        let env = Env::of(ui);
        let words = env.words::<ReportWords>();
        let common = env.words::<CommonWords>();
        let text = self.as_text(&env);
        let ReportDialog { title, summary, notes, open } = self;
        let accept = FooterButton::new(common.ok.clone(), DialogRole::Accept);
        let copy = FooterButton::new(words.copy.clone(), DialogRole::Other).stays_open(true);
        let mut dialog = Dialog::new(title).buttons([accept, copy]);
        if let Some(open) = open {
            dialog = dialog.opened(open);
        }
        let count = (!notes.is_empty()).then(|| {
            let n = notes.len() as u64;
            words.count.text(n, &env.format().integer(n))
        });
        let shown = dialog.show(ui, |ui, _| {
            Stack::new().show(ui, |ui, _| {
                Text::new(summary.clone()).show(ui);
                if let Some(count) = &count {
                    Text::new(count.clone()).role(TypeRole::Caption).shade(Shade::Muted).show(ui);
                }
                for (at, note) in notes.iter().enumerate() {
                    Notice::new(note.tone, note.text.clone()).layout(NoticeLayout::Inline).id_salt(at).show(ui);
                }
            });
        });
        let DialogOutput { response, closed, picked } = shown.output;
        ReportOutput { response, closed, copied: (picked == Some(COPY)).then_some(text) }
    }
}

/// The position of the Copy button among the footer buttons.
const COPY: usize = 1;
