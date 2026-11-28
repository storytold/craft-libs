//! `RecoveryDialog` (24.4): the offer to recover a document an earlier run autosaved and never saved, a
//! configuration of `Prompt`.

use super::host::{DialogResult, FieldRefusal, StackDialog};
use super::prompt::{Prompt, Reply, apply_to_all, buttons};
use super::stack::{DialogId, DialogStack, dialog_id};
use super::words::DialogWords;
use crate::contract::{Bind, NoValue, Outcome};
use crate::runtime::env::{Env, Label};
use crate::runtime::overlay::CloseReason;
use crate::sections::DialogRole;
use craft_i18n::AgeStyle;
use egui::{Context, Response, Ui};
use serde_json::{Value, json};
use std::time::Duration;

/// One autosaved document, as the app's recovery folder lists it.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub struct RecoveredDocument {
    /// The entry's key in the recovery folder; the app passes it back to recover or discard.
    pub key: String,
    /// The document's name ("poster.psd", "Untitled-2").
    pub name: String,
    /// How long before the app listed the recovery folder it was autosaved.
    pub age: Duration,
    /// Why Recover is disabled now ("Stop recording first"), the tooltip of the disabled button; `None` enables it.
    pub blocked: Option<String>,
    /// The copy is password-protected: the dialog says so, and Recover stays available (a flag apart from `blocked`).
    pub protected: bool,
}

impl RecoveredDocument {
    /// A document autosaved `age` ago, with Recover available.
    pub fn new(key: impl Into<String>, name: impl Into<String>, age: Duration) -> Self {
        RecoveredDocument { key: key.into(), name: name.into(), age, blocked: None, protected: false }
    }

    /// Marks the copy password-protected.
    #[must_use]
    pub fn protected(mut self) -> Self {
        self.protected = true;
        self
    }

    /// Disables Recover for `reason`.
    #[must_use]
    pub fn blocked(mut self, reason: impl Into<String>) -> Self {
        self.blocked = Some(reason.into());
        self
    }
}

/// What the user chose for one document. Closing the dialog (Escape, the close action) is `DialogResult::Cancel`:
/// the copy stays and is offered again at the next launch.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
#[non_exhaustive]
pub enum RecoveryAnswer {
    /// Open the autosaved copy as an unsaved document.
    Recover,
    /// Delete the autosaved copy.
    Discard,
}

/// The answer the recovery dialog gives: what to do with the document, and whether to do it for the documents
/// still waiting.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub struct RecoveryChoice {
    /// What to do with the document.
    pub answer: RecoveryAnswer,
    /// Apply the answer to every remaining document (the Apply to All box was ticked).
    pub apply_to_all: bool,
}

/// What the recovery dialog reports: its response, how it closed and the answer given.
#[derive(Debug)]
#[non_exhaustive]
pub struct RecoveryOutput {
    /// The dialog's response.
    pub response: Response,
    /// How the dialog closed in this pass; `None` while it stays open.
    pub closed: Option<CloseReason>,
    /// Discard or Recover, the pass a button or Enter answers; `None` when the dialog stays open or was dismissed
    /// (Escape, the close action): the copy stays and is offered again at the next launch.
    pub answer: Option<RecoveryChoice>,
}

impl Outcome for RecoveryOutput {
    type Value = NoValue;

    fn response(&self) -> &Response {
        &self.response
    }
}

/// Recover Unsaved Work, one document at a time: a prompt saying when the document was autosaved, with Discard and
/// Recover (the accepting answer). Escape and the close action keep the copy for the next launch. The app enqueues
/// the first document at `StartupStage::Recovery` and opens the next after taking each answer.
#[derive(crate::contract::Pattern)]
#[pattern(dialog)]
pub struct RecoveryDialog {
    id: DialogId,
    document: RecoveredDocument,
    choice: RecoveryAnswer,
    open: bool,
    /// The documents still waiting, this one included: the Apply to All box shows while it is above 1.
    remaining: usize,
    /// The state of the Apply to All box.
    apply_to_all: bool,
}

impl RecoveryDialog {
    /// A dialog offering to recover `document`.
    pub fn new(document: RecoveredDocument) -> Self {
        Self { id: dialog_id("recovery"), document, choice: RecoveryAnswer::Recover, open: true, remaining: 1, apply_to_all: false }
    }

    /// The documents still waiting, this one included; the Apply to All box shows while it is above 1.
    #[must_use]
    pub fn remaining(mut self, n: usize) -> Self {
        self.remaining = n;
        self
    }

    /// The document asked about; the app reads its key with the answer.
    pub fn document(&self) -> &RecoveredDocument {
        &self.document
    }
}

impl RecoveryDialog {
    /// Whether the Apply to All box shows: more documents wait after this one.
    fn offers_apply_to_all(&self) -> bool {
        self.remaining > 1
    }

    /// The choice for `answer`; the box counts only while it shows.
    fn choice(&self, answer: RecoveryAnswer, ticked: bool) -> RecoveryChoice {
        RecoveryChoice { answer, apply_to_all: ticked && self.offers_apply_to_all() }
    }

    /// Shows the offer; `hosted` holds the open state and the Apply to All box in the dialog's own fields, else in
    /// memory.
    fn present(&mut self, ui: &mut Ui, hosted: bool) -> RecoveryOutput {
        let env = Env::of(ui);
        let words = env.words::<DialogWords>();
        let age = env.relative_age(self.document.age.as_secs(), AgeStyle::Long);
        let message = words.recovery_message.fill(&[("name", self.document.name.as_str()), ("age", age.as_str())]);
        let reason = self.document.blocked.clone().map(|reason| Label::from_cow(reason.into()));
        let replies = vec![
            Reply::plain(words.recovery_discard.clone(), DialogRole::Alternate),
            Reply::plain(words.recovery_recover.clone(), DialogRole::Accept).enabled(reason.is_none(), reason),
        ];
        let mut prompt = Prompt::new(words.recovery_title.clone(), message).answers(buttons(&env, replies));
        if self.document.protected {
            prompt = prompt.paragraph(words.recovery_protected.clone());
        }
        let mut ticked = hosted && self.apply_to_all;
        prompt = apply_to_all(prompt, words.unsaved_apply_to_all.clone(), self.remaining, hosted.then_some(&mut self.apply_to_all), &mut ticked);
        if hosted {
            prompt = prompt.opened(Bind::bound(&mut self.open));
        }
        let shown = prompt.show(ui);
        let answer = match (shown.closed, shown.picked) {
            (Some(CloseReason::Activated | CloseReason::Enter), Some(0)) => Some(RecoveryAnswer::Discard),
            (Some(CloseReason::Activated | CloseReason::Enter), Some(_)) => Some(RecoveryAnswer::Recover),
            _ => None,
        };
        let answer = answer.map(|answer| self.choice(answer, ticked));
        RecoveryOutput { response: shown.response, closed: shown.closed, answer }
    }

    /// Shows the offer; `answer` is `Some` the pass Discard or Recover answers, and `closed` without an answer means
    /// the dialog was dismissed.
    pub fn show(mut self, ui: &mut Ui) -> RecoveryOutput {
        self.present(ui, false)
    }
}

impl StackDialog for RecoveryDialog {
    type Result = RecoveryChoice;

    fn id(&self) -> DialogId {
        self.id
    }

    fn show(&mut self, ui: &mut Ui, _: &DialogStack) -> Option<DialogResult<RecoveryChoice>> {
        let shown = self.present(ui, true);
        shown.closed?;
        Some(shown.answer.map_or(DialogResult::Cancel, DialogResult::Ok))
    }

    /// The chosen answer, Recover by default; `None` for Recover while recovery is blocked.
    fn confirm(&mut self, _: &Context) -> Option<RecoveryChoice> {
        (self.choice != RecoveryAnswer::Recover || self.document.blocked.is_none()).then(|| self.choice(self.choice, self.apply_to_all))
    }

    /// `choice`, the answer OK gives.
    fn json_fields(&self) -> Option<Value> {
        Some(json!({ "choice": self.choice }))
    }

    /// `choice` takes `recover` (not while recovery is blocked) or `discard`.
    fn set_json_field(&mut self, _: &Context, field: &str, value: Value) -> Result<(), String> {
        if field != "choice" {
            return Err(FieldRefusal::Unknown.message(field));
        }
        match serde_json::from_value::<RecoveryAnswer>(value.clone()) {
            Ok(choice) if choice != RecoveryAnswer::Recover || self.document.blocked.is_none() => {
                self.choice = choice;
                Ok(())
            }
            _ => Err(FieldRefusal::Rejects(&value).message(field)),
        }
    }
}
