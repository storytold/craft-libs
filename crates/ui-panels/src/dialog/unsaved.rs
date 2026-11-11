//! `UnsavedChangesDialog` (24.4): the Save, Don't Save and Cancel question for one document, a configuration of
//! `Prompt`.

use super::host::{DialogResult, StackDialog};
use super::prompt::{Prompt, Reply, buttons};
use super::stack::{DialogId, DialogStack, dialog_id};
use super::words::DialogWords;
use crate::choice::Checkbox;
use crate::contract::{Bind, NoValue, Outcome, Props as _, Show as _};
use crate::runtime::env::{Env, Label};
use crate::runtime::overlay::CloseReason;
use crate::runtime::words::CommonWords;
use crate::sections::DialogRole;
use craft_behavior::Mixed;
use craft_host::target::Os;
use craft_quit::{CloseAnswer, CloseScope};
use egui::{Context, Response, Ui};

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
}

impl UnsavedChangesDialog {
    /// The question for the document `document_name`, closing under `scope`; the guard asks about one document.
    pub fn new(scope: CloseScope, document_name: impl Into<String>) -> Self {
        Self { id: dialog_id("unsaved-changes"), scope, name: document_name.into(), remaining: 1, apply_to_all: false, open: true, chosen: None }
    }

    /// The answer a host-side `confirm` gives, as pressing that button would; the accepting answer's until set.
    pub fn set_answer(&mut self, answer: CloseAnswer) {
        self.chosen = Some(answer);
    }

    /// How many documents the guard still asks about, this one included; above one the "Apply to All" box shows.
    #[must_use]
    pub fn remaining(mut self, remaining: usize) -> Self {
        self.remaining = remaining;
        self
    }

    /// The answers in the order the buttons are given: what each means and how it is asked.
    fn replies(&self, words: &DialogWords, cancel: Label, os: Os) -> Vec<(CloseAnswer, Reply)> {
        let cancel = (CloseAnswer::Cancel, Reply::new(cancel, DialogRole::Cancel));
        if self.scope == CloseScope::Revert {
            return vec![(CloseAnswer::DontSave, Reply::new(words.unsaved_revert.clone(), DialogRole::Accept)), cancel];
        }
        if os == Os::Mac {
            vec![
                (CloseAnswer::Save, Reply::new(words.unsaved_save.clone(), DialogRole::Accept)),
                (CloseAnswer::DontSave, Reply::new(words.unsaved_dont_save.clone(), DialogRole::Other)),
                cancel,
            ]
        } else {
            vec![
                (CloseAnswer::Save, Reply::new(words.unsaved_yes.clone(), DialogRole::Accept)),
                (CloseAnswer::DontSave, Reply::new(words.unsaved_no.clone(), DialogRole::Other)),
                cancel,
            ]
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
        let mut prompt = Prompt::new(words.unsaved_title.clone(), message).answers(buttons(&env, replies));
        let mut ticked = hosted && self.apply_to_all;
        if self.remaining > 1 && self.scope != CloseScope::Revert {
            let label = words.unsaved_apply_to_all.clone();
            let tick = &mut ticked;
            let mut value = hosted.then_some(&mut self.apply_to_all);
            prompt = prompt.extras(move |ui| {
                if let Some(Mixed::Same(shown)) =
                    Checkbox::new(value.as_mut().map_or_else(Bind::kept, |value| Bind::bound(&mut **value))).label(label.clone()).show(ui).shown
                {
                    *tick = *shown;
                }
            });
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
    fn confirm(&mut self, _: &Context) -> Option<UnsavedAnswer> {
        let accepting = if self.scope == CloseScope::Revert { CloseAnswer::DontSave } else { CloseAnswer::Save };
        Some(self.answer(self.chosen.unwrap_or(accepting)))
    }
}
