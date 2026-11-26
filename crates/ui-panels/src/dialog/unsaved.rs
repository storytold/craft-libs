//! `UnsavedChangesDialog` (24.4): the Save, Don't Save and Cancel question for one document, a configuration of
//! `Prompt`.

use super::host::{DialogResult, FieldRefusal, StackDialog};
use super::prompt::{Prompt, Reply, apply_to_all, buttons};
use super::stack::{DialogId, DialogStack, dialog_id};
use super::words::DialogWords;
use crate::contract::{Bind, NoValue, Outcome};
use crate::runtime::env::{Env, Label};
use crate::runtime::overlay::CloseReason;
use crate::runtime::words::CommonWords;
use crate::sections::DialogRole;
use craft_behavior::IconId;
use craft_host::target::Os;
use craft_keymap::{Chord, Key, Mods};
use craft_quit::{CloseAnswer, CloseScope};
use craft_theme::variant::Tone;
use egui::{Context, Response, Ui};
use serde_json::{Value, json};

/// The answer for one document, with whether it applies to the rest.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub struct UnsavedAnswer {
    /// The button the user pressed.
    pub answer: CloseAnswer,
    /// The "Apply to All" box was ticked: the app calls `CloseGuard::answer_all`.
    pub apply_to_all: bool,
}

/// What the unsaved-changes question reports: its response, how it closed and the answer given.
#[derive(Debug)]
#[non_exhaustive]
pub struct UnsavedChangesOutput {
    /// The dialog's response.
    pub response: Response,
    /// How the dialog closed in this pass; `None` while it stays open.
    pub closed: Option<CloseReason>,
    /// The answer given in this pass, Cancel included; `None` while the dialog stays open.
    pub answer: Option<UnsavedAnswer>,
}

impl Outcome for UnsavedChangesOutput {
    type Value = NoValue;

    fn response(&self) -> &Response {
        &self.response
    }
}

/// The question for one modified document: a prompt titled "Unsaved changes" with the document's name in the
/// message, Save (the accepting answer), Don't Save and Cancel in the platform's order and words, and, while the
/// guard has more documents, an "Apply to All" check box. Windows and Linux ask Yes, No and Cancel. Reverting asks
/// Revert and Cancel; Revert answers `CloseAnswer::DontSave`.
///
/// The node has the alert-dialog role (`Role::ALERT_DIALOG`), described by the consequence message and any save
/// failure reason.
///
/// The result is `Ok(UnsavedAnswer)` for every answer, Cancel included (the guard needs it), and
/// `DialogResult::Cancel` only when the host closed the dialog from outside.
#[derive(crate::contract::Pattern)]
pub struct UnsavedChangesDialog {
    id: DialogId,
    scope: CloseScope,
    name: String,
    remaining: usize,
    apply_to_all: bool,
    open: bool,
    /// What a programmatic OK answers; the accepting answer's when `None`.
    chosen: Option<CloseAnswer>,
    /// Why the last save failed, shown under the message.
    reason: Option<String>,
    /// The buttons are worded and ordered as a document app's save question: Don't save, Cancel, Save.
    document_wording: bool,
    title: Option<Label>,
    message: Option<Label>,
    note: Option<Label>,
    icon: Option<IconId>,
    /// The app's word for the Don't Save button, replacing the catalog's.
    dont_save: Option<Label>,
}

impl UnsavedChangesDialog {
    /// The question for the document `document_name`, closing under `scope`; the guard asks about one document.
    pub fn new(scope: CloseScope, document_name: impl Into<String>) -> Self {
        Self {
            id: dialog_id("unsaved-changes"),
            scope,
            name: document_name.into(),
            remaining: 1,
            apply_to_all: false,
            open: true,
            chosen: None,
            reason: None,
            document_wording: false,
            title: None,
            message: None,
            note: None,
            icon: None,
            dont_save: None,
        }
    }

    /// Shows why the last save failed, in the bad tone under the message.
    #[must_use]
    pub fn reason(mut self, reason: impl Into<String>) -> Self {
        self.reason = Some(reason.into());
        self
    }

    /// The answer a host-side `confirm` gives, as pressing that button would; the accepting answer's until set. An
    /// answer the question has no button for (Save while reverting) gives none.
    pub fn set_answer(&mut self, answer: CloseAnswer) {
        self.chosen = Some(answer);
    }

    /// Words and orders the buttons as a document app's save question on every platform: Don't save, Cancel and Save
    /// last, the accepting answer at the end.
    #[must_use]
    pub fn document_wording(mut self, on: bool) -> Self {
        self.document_wording = on;
        self
    }

    /// Replaces the title ("Unsaved changes").
    #[must_use]
    pub fn title(mut self, title: impl Into<Label>) -> Self {
        self.title = Some(title.into());
        self
    }

    /// Replaces the question, in the app's words (the document's name is the app's to put in).
    #[must_use]
    pub fn message(mut self, message: impl Into<Label>) -> Self {
        self.message = Some(message.into());
        self
    }

    /// A muted note under the question.
    #[must_use]
    pub fn note(mut self, note: impl Into<Label>) -> Self {
        self.note = Some(note.into());
        self
    }

    /// An icon of the catalog beside the question.
    #[must_use]
    pub fn icon(mut self, icon: IconId) -> Self {
        self.icon = Some(icon);
        self
    }

    /// Replaces the Don't Save button's word, in the app's words.
    #[must_use]
    pub fn dont_save_label(mut self, label: impl Into<Label>) -> Self {
        self.dont_save = Some(label.into());
        self
    }

    /// What a programmatic OK answers.
    fn chosen(&self) -> CloseAnswer {
        let accepting = if self.scope == CloseScope::Revert { CloseAnswer::DontSave } else { CloseAnswer::Save };
        self.chosen.unwrap_or(accepting)
    }

    /// Whether the "Apply to All" box shows: the guard has more documents, and the question is not a revert.
    fn offers_apply_to_all(&self) -> bool {
        self.remaining > 1 && self.scope != CloseScope::Revert
    }

    /// How many documents the guard still asks about, this one included; above one the "Apply to All" box shows.
    #[must_use]
    pub fn remaining(mut self, remaining: usize) -> Self {
        self.remaining = remaining;
        self
    }

    /// The answers in the order the buttons are given: what each means and how it is asked. Cmd+D (Ctrl+D off macOS)
    /// also answers Don't Save, as the platform's own save sheets do; Revert has no such chord.
    fn replies(&self, words: &DialogWords, cancel: Label, os: Os) -> Vec<(CloseAnswer, Reply)> {
        let mac = os == Os::Mac || self.document_wording;
        self.offered()
            .iter()
            .map(|&answer| {
                let reply = match answer {
                    CloseAnswer::Save => Reply::new(if mac { &words.unsaved_save } else { &words.unsaved_yes }.clone(), DialogRole::Accept),
                    CloseAnswer::DontSave if self.scope == CloseScope::Revert => Reply::new(words.unsaved_revert.clone(), DialogRole::Accept),
                    CloseAnswer::DontSave => {
                        let word = if mac { &words.unsaved_dont_save } else { &words.unsaved_no };
                        let word = self.dont_save.as_ref().unwrap_or(word);
                        Reply::new(word.clone(), DialogRole::Alternate).chord(Chord::new(Mods::CMD, Key::D))
                    }
                    CloseAnswer::Cancel => Reply::new(cancel.clone(), DialogRole::Cancel),
                };
                (answer, reply)
            })
            .collect()
    }

    /// The answers the question has buttons for, in button order: reverting has no Save.
    fn offered(&self) -> &'static [CloseAnswer] {
        if self.scope == CloseScope::Revert {
            &[CloseAnswer::DontSave, CloseAnswer::Cancel]
        } else if self.document_wording {
            &[CloseAnswer::DontSave, CloseAnswer::Cancel, CloseAnswer::Save]
        } else {
            &[CloseAnswer::Save, CloseAnswer::DontSave, CloseAnswer::Cancel]
        }
    }

    fn answer(&self, answer: CloseAnswer) -> UnsavedAnswer {
        UnsavedAnswer { answer, apply_to_all: self.apply_to_all && self.remaining > 1 }
    }

    /// Shows the question; `hosted` holds the open state and the "Apply to All" box in the dialog's own fields, else
    /// in memory.
    fn present(&mut self, ui: &mut Ui, hosted: bool) -> UnsavedChangesOutput {
        let env = Env::of(ui);
        let (words, common) = (env.words::<DialogWords>(), env.words::<CommonWords>());
        let template = match self.scope {
            CloseScope::Close => &words.unsaved_closing,
            CloseScope::Quit => &words.unsaved_quitting,
            CloseScope::Revert => &words.unsaved_reverting,
        };
        let message = template.fill(&[("name", self.name.as_str())]);
        let replies = self.replies(&words, common.cancel.clone(), env.view().shell().os);
        let answers: Vec<CloseAnswer> = replies.iter().map(|(answer, _)| *answer).collect();
        let replies: Vec<Reply> = replies.into_iter().map(|(_, reply)| reply).collect();
        let title = self.title.clone().unwrap_or_else(|| words.unsaved_title.clone());
        let message = self.message.clone().unwrap_or_else(|| Label::from(message));
        let mut prompt = Prompt::new(title, message).alert().answers(buttons(&env, replies));
        if let Some(reason) = &self.reason {
            prompt = prompt.tone(Tone::Bad, reason.clone());
        }
        if let Some(icon) = self.icon.clone() {
            prompt = prompt.icon(icon);
        }
        if let Some(note) = self.note.clone() {
            prompt = prompt.note(note);
        }
        let mut ticked = hosted && self.apply_to_all;
        if self.offers_apply_to_all() {
            prompt = apply_to_all(prompt, words.unsaved_apply_to_all.clone(), self.remaining, hosted.then_some(&mut self.apply_to_all), &mut ticked);
        }
        if hosted {
            prompt = prompt.opened(Bind::bound(&mut self.open));
        }
        let shown = prompt.show(ui);
        let answer = shown.picked.map(|at| UnsavedAnswer { answer: answers[at], apply_to_all: ticked && self.remaining > 1 });
        UnsavedChangesOutput { response: shown.response, closed: shown.closed, answer }
    }

    /// Shows the question; `answer` is `Some` the pass a button, Enter, Escape or the close action answers.
    pub fn show(mut self, ui: &mut Ui) -> UnsavedChangesOutput {
        self.present(ui, false)
    }
}

impl StackDialog for UnsavedChangesDialog {
    type Result = UnsavedAnswer;

    fn id(&self) -> DialogId {
        self.id
    }

    fn show(&mut self, ui: &mut Ui, _: &DialogStack) -> Option<DialogResult<UnsavedAnswer>> {
        self.present(ui, true).answer.map(DialogResult::Ok)
    }

    /// The answer set with `set_answer`; the accepting answer's (Save, or Don't Save when reverting) unless set.
    /// None for an answer the question has no button for.
    fn confirm(&mut self, _: &Context) -> Option<UnsavedAnswer> {
        let chosen = self.chosen();
        self.offered().contains(&chosen).then(|| self.answer(chosen))
    }

    /// `choice` (`save`, `dontSave` or `cancel`), the answer a programmatic OK gives, and `applyToAll` while the
    /// guard has more documents.
    fn json_fields(&self) -> Option<Value> {
        let mut fields = json!({ "choice": answer_name(self.chosen()) });
        if self.offers_apply_to_all() {
            fields["applyToAll"] = Value::Bool(self.apply_to_all);
        }
        if let Some(reason) = &self.reason {
            fields["reason"] = Value::String(reason.clone());
        }
        Some(fields)
    }

    fn set_json_field(&mut self, _: &Context, field: &str, value: Value) -> Result<(), String> {
        match field {
            "choice" => {
                let answer = self.offered().iter().copied().find(|answer| value.as_str() == Some(answer_name(*answer)));
                let Some(answer) = answer else { return Err(FieldRefusal::Rejects(&value).message(field)) };
                self.set_answer(answer);
            }
            "applyToAll" if self.offers_apply_to_all() => match value.as_bool() {
                Some(on) => self.apply_to_all = on,
                None => return Err(FieldRefusal::Rejects(&value).message(field)),
            },
            _ => return Err(FieldRefusal::Unknown.message(field)),
        }
        Ok(())
    }
}

/// What automation calls `answer`.
fn answer_name(answer: CloseAnswer) -> &'static str {
    match answer {
        CloseAnswer::Save => "save",
        CloseAnswer::DontSave => "dontSave",
        CloseAnswer::Cancel => "cancel",
    }
}
