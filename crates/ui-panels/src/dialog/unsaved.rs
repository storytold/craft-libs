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

    /// The answer a host-side `confirm` gives, as pressing that button would; the accepting answer's until set. An
    /// answer the question has no button for (Save while reverting) gives none.
    pub fn set_answer(&mut self, answer: CloseAnswer) {
        self.chosen = Some(answer);
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

    /// The answers in the order the buttons are given: what each means and how it is asked.
    fn replies(&self, words: &DialogWords, cancel: Label, os: Os) -> Vec<(CloseAnswer, Reply)> {
        let mac = os == Os::Mac;
        self.offered()
            .iter()
            .map(|&answer| {
                let reply = match answer {
                    CloseAnswer::Save => Reply::new(if mac { &words.unsaved_save } else { &words.unsaved_yes }.clone(), DialogRole::Accept),
                    CloseAnswer::DontSave if self.scope == CloseScope::Revert => Reply::new(words.unsaved_revert.clone(), DialogRole::Accept),
                    CloseAnswer::DontSave => Reply::new(if mac { &words.unsaved_dont_save } else { &words.unsaved_no }.clone(), DialogRole::Other),
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
        let mut prompt = Prompt::new(words.unsaved_title.clone(), message).answers(buttons(&env, replies));
        let mut ticked = hosted && self.apply_to_all;
        if self.offers_apply_to_all() {
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
        Some(fields)
    }

    fn set_json_field(&mut self, _: &Context, field: &str, value: Value) -> Result<(), String> {
        match field {
            "choice" => {
                let answer = self.offered().iter().copied().find(|answer| value.as_str() == Some(answer_name(*answer)));
                let Some(answer) = answer else {
                    let taken: Vec<&str> = self.offered().iter().map(|answer| answer_name(*answer)).collect();
                    return Err(format!("`choice` can't take {value}: the question answers {}", json!(taken)));
                };
                self.set_answer(answer);
            }
            "applyToAll" if self.offers_apply_to_all() => self.apply_to_all = value.as_bool().ok_or("`applyToAll` takes true or false")?,
            _ => return Err(format!("unknown field `{field}`")),
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
