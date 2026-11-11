//! `RecoveryDialog` (24.4): the offer to recover a document an earlier run autosaved and never saved, a
//! configuration of `Prompt`.

use super::host::{DialogResult, StackDialog};
use super::prompt::{Prompt, Reply, buttons};
use super::stack::{DialogId, DialogStack, dialog_id};
use super::words::DialogWords;
use crate::contract::{Bind, NoValue, Outcome};
use crate::runtime::env::{Env, Label};
use crate::runtime::overlay::CloseReason;
use crate::sections::DialogRole;
use craft_i18n::AgeStyle;
use egui::{Context, Response, Ui};
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
}

impl RecoveredDocument {
    /// A document autosaved `age` ago, with Recover available.
    pub fn new(key: impl Into<String>, name: impl Into<String>, age: Duration) -> Self {
        RecoveredDocument { key: key.into(), name: name.into(), age, blocked: None }
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
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum RecoveryAnswer {
    /// Open the autosaved copy as an unsaved document.
    Recover,
    /// Delete the autosaved copy.
    Discard,
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
    pub answer: Option<RecoveryAnswer>,
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
    open: bool,
}

impl RecoveryDialog {
    /// A dialog offering to recover `document`.
    pub fn new(document: RecoveredDocument) -> Self {
        Self { id: dialog_id("recovery"), document, open: true }
    }

    /// The document asked about; the app reads its key with the answer.
    pub fn document(&self) -> &RecoveredDocument {
        &self.document
    }
}

impl RecoveryDialog {
    /// Shows the offer; `hosted` holds the open state in the dialog's own field, else in memory.
    fn present(&mut self, ui: &mut Ui, hosted: bool) -> RecoveryOutput {
        let env = Env::of(ui);
        let words = env.words::<DialogWords>();
        let age = env.relative_age(self.document.age.as_secs(), AgeStyle::Long);
        let message = words.recovery_message.fill(&[("name", self.document.name.as_str()), ("age", age.as_str())]);
        let reason = self.document.blocked.clone().map(|reason| Label::from_cow(reason.into()));
        let replies = vec![
            Reply::plain(words.recovery_discard.clone(), DialogRole::Other),
            Reply::plain(words.recovery_recover.clone(), DialogRole::Accept).enabled(reason.is_none(), reason),
        ];
        let mut prompt = Prompt::new(words.recovery_title.clone(), message).answers(buttons(&env, replies));
        if hosted {
            prompt = prompt.opened(Bind::bound(&mut self.open));
        }
        let shown = prompt.show(ui);
        let answer = match (shown.closed, shown.picked) {
            (Some(CloseReason::Activated | CloseReason::Enter), Some(0)) => Some(RecoveryAnswer::Discard),
            (Some(CloseReason::Activated | CloseReason::Enter), Some(_)) => Some(RecoveryAnswer::Recover),
            _ => None,
        };
        RecoveryOutput { response: shown.response, closed: shown.closed, answer }
    }

    /// Shows the offer; `answer` is `Some` the pass Discard or Recover answers, and `closed` without an answer means
    /// the dialog was dismissed.
    pub fn show(mut self, ui: &mut Ui) -> RecoveryOutput {
        self.present(ui, false)
    }
}

impl StackDialog for RecoveryDialog {
    type Result = RecoveryAnswer;

    fn id(&self) -> DialogId {
        self.id
    }

    fn show(&mut self, ui: &mut Ui, _: &DialogStack) -> Option<DialogResult<RecoveryAnswer>> {
        let shown = self.present(ui, true);
        shown.closed?;
        Some(shown.answer.map_or(DialogResult::Cancel, DialogResult::Ok))
    }

    /// Recover, the accepting answer; `None` while recovery is blocked.
    fn confirm(&mut self, _: &Context) -> Option<RecoveryAnswer> {
        self.document.blocked.is_none().then_some(RecoveryAnswer::Recover)
    }
}
