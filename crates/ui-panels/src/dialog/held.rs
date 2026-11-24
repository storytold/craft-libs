//! The held OK every hosted dialog family shares: OK files its answer for the app, which accepts or refuses it before
//! the dialog closes (a check the app runs, a command that may fail, a save path it picks).

use crate::runtime::env::Label;

/// Where a dialog's held OK stands ([`StackDialog::held_state`](super::StackDialog::held_state)): an OK that files its
/// answer for the app, which answers it before the dialog closes (a check the app runs, a save path it picks).
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub enum HeldOk {
    /// OK filed its answer; the app has not answered it yet.
    Waiting,
    /// The app accepted the held OK: the dialog finishes with it at its next pass or [`DialogHost::confirm`](super::DialogHost::confirm).
    Accepted,
    /// The app refused the last held OK for this reason; OK is enabled again, unless the refusal blocks
    /// ([`HeldAnswer::refuse`]).
    Refused(Label),
}

/// The state of a dialog's held OK, from the filed answer to the app's answer. A hosted dialog keeps one, files the
/// answer its OK gives ([`file`](HeldAnswer::file)), keeps OK disabled while [`is_waiting`](HeldAnswer::is_waiting),
/// hands the answer to the app once ([`take`](HeldAnswer::take)), and closes with [`finish`](HeldAnswer::finish) once
/// the app accepted it; a refusal keeps the refused answer beside its reason, so the dialog can show the reason until
/// its values change.
#[derive(Clone, Debug, PartialEq)]
pub struct HeldAnswer<T> {
    filed: Option<T>,
    /// `take` has not handed out `filed` yet.
    untaken: bool,
    accepted: bool,
    /// The reason, the refused answer and whether the refusal blocks OK until it is withdrawn.
    refused: Option<(Label, T, bool)>,
}

impl<T> Default for HeldAnswer<T> {
    fn default() -> Self {
        HeldAnswer { filed: None, untaken: false, accepted: false, refused: None }
    }
}

impl<T> HeldAnswer<T> {
    /// Nothing held.
    pub fn new() -> Self {
        Self::default()
    }

    /// Holds `answer` for the app; a refusal shown so far ends.
    pub fn file(&mut self, answer: T) {
        self.filed = Some(answer);
        self.untaken = true;
        self.accepted = false;
        self.refused = None;
    }

    /// An answer is filed and not refused: OK waits disabled, and a second OK files nothing.
    pub fn is_waiting(&self) -> bool {
        self.filed.is_some()
    }

    /// The app accepted the filed answer: the dialog closes with it.
    pub fn is_accepted(&self) -> bool {
        self.accepted && self.filed.is_some()
    }

    /// The app's answer to the filed answer: `Ok` accepts it, `Err` refuses it for `reason`, which the dialog shows
    /// with OK enabled again. Returns whether an answer was filed; without one it does nothing.
    pub fn answer(&mut self, answer: Result<(), Label>) -> bool {
        let Some(filed) = self.filed.take() else { return false };
        match answer {
            Ok(()) => {
                self.filed = Some(filed);
                self.accepted = true;
            }
            Err(reason) => {
                self.untaken = false;
                self.refused = Some((reason, filed, false));
            }
        }
        true
    }

    /// The accepted answer, for the pass or the confirm that closes the dialog; `None` until the app accepted one.
    pub fn finish(&mut self) -> Option<T> {
        if !self.is_accepted() {
            return None;
        }
        self.accepted = false;
        self.untaken = false;
        self.filed.take()
    }

    /// Why the app refused the last held answer, and that answer; `None` once OK filed again or the refusal ended.
    pub fn refusal(&self) -> Option<(&Label, &T)> {
        self.refused.as_ref().map(|(reason, answer, _)| (reason, answer))
    }

    /// Takes back the held OK without a refusal: nothing is held, OK is enabled again, and the values stay.
    pub fn withdraw(&mut self) {
        self.filed = None;
        self.untaken = false;
        self.accepted = false;
        self.refused = None;
    }

    /// Ends the refusal shown so far (the values it refused changed).
    pub fn clear_refusal(&mut self) {
        self.refused = None;
    }

    /// The host refuses `answer` for `reason` without a filed answer: OK stays off while
    /// [`refusal_blocks`](HeldAnswer::refusal_blocks), until [`clear_refusal`](HeldAnswer::clear_refusal).
    pub fn refuse(&mut self, reason: Label, answer: T) {
        self.filed = None;
        self.untaken = false;
        self.accepted = false;
        self.refused = Some((reason, answer, true));
    }

    /// Whether the refusal keeps OK off: a host refusal does, an `answer(Err)` does not.
    pub fn refusal_blocks(&self) -> bool {
        self.refused.as_ref().is_some_and(|(_, _, blocks)| *blocks)
    }

    /// Where the held OK stands, for [`StackDialog::held_state`](super::StackDialog::held_state).
    pub fn state(&self) -> Option<HeldOk> {
        match (&self.filed, &self.refused) {
            (Some(_), _) if self.accepted => Some(HeldOk::Accepted),
            (Some(_), _) => Some(HeldOk::Waiting),
            (None, Some((reason, _, _))) => Some(HeldOk::Refused(reason.clone())),
            (None, None) => None,
        }
    }
}

/// A hosted dialog with a held OK: the app opens it with [`held_ok`](HoldsOk::held_ok), takes the answer OK filed with
/// [`take_ok`](HoldsOk::take_ok), and answers it with [`answer_ok`](HoldsOk::answer_ok) once it has accepted or refused
/// it. One path for every dialog family, so no module drains a held OK of its own.
pub trait HoldsOk: super::StackDialog {
    /// What OK files for the app to judge.
    type Answer: Clone + Send;

    /// The dialog with its OK held: OK files its answer instead of closing the dialog.
    #[must_use]
    fn held_ok(self) -> Self
    where
        Self: Sized;

    /// The answer OK filed, once; `None` when none is filed or it was taken already.
    fn take_ok(&mut self) -> Option<Self::Answer>;

    /// The app's answer to the filed answer: `Ok` accepts it, `Err` refuses it for the reason, which the dialog shows
    /// with OK enabled again.
    fn answer_ok(&mut self, answer: Result<(), Label>);
}

impl<T: Clone> HeldAnswer<T> {
    /// The filed answer for the app to judge, once; `None` when none is filed or it was handed out already.
    pub fn take(&mut self) -> Option<T> {
        if !std::mem::take(&mut self.untaken) {
            return None;
        }
        self.filed.clone()
    }
}
