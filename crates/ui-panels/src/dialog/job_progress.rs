//! `JobProgress`: the modal window of a running job, a configuration of `Prompt` with a progress bar.

use crate::runtime::env::Label;
use egui::{Response, Ui};

/// Modal progress window for a running job: a progress bar (or an indeterminate one), counts and the percentage, the
/// queued line, and Cancel (and Hide, when `can_hide`). Escape and the close action give Cancel.
// P3(files-w2): fl.F12 `#[pattern(dialog)]` needs `StackDialog` (`dialog_probe`); plain `Pattern` registers it
#[derive(crate::contract::Pattern)]
#[allow(dead_code)] // P3(files-w2): fl.F12
pub struct JobProgress {
    title: Label,
    step: Label,
    fraction: Option<f32>,
    counts: Option<Label>,
    queued: Option<Label>,
    can_hide: bool,
}

impl JobProgress {
    /// A progress window titled `title` showing the current `step`.
    pub fn new(title: impl Into<Label>, step: impl Into<Label>) -> Self {
        let _ = (title, step);
        todo!("fl.F12")
    }

    /// The finished fraction in `0.0..=1.0`; `None` shows an indeterminate bar.
    #[must_use]
    pub fn fraction(self, fraction: Option<f32>) -> Self {
        let _ = fraction;
        todo!("fl.F12")
    }

    /// The counts line, shown muted with the percentage.
    #[must_use]
    pub fn counts(self, counts: impl Into<Label>) -> Self {
        let _ = counts;
        todo!("fl.F12")
    }

    /// The queued line.
    #[must_use]
    pub fn queued(self, queued: impl Into<Label>) -> Self {
        let _ = queued;
        todo!("fl.F12")
    }

    /// Whether the window offers Hide beside Cancel.
    #[must_use]
    pub fn can_hide(self, can_hide: bool) -> Self {
        let _ = can_hide;
        todo!("fl.F12")
    }

    /// Shows the window; `answer` is `Some` the pass Cancel or Hide answers.
    pub fn show(self, ui: &mut Ui) -> JobProgressOutput {
        let _ = ui;
        todo!("fl.F12")
    }
}

/// What [`JobProgress`] reports: its response and the answer given in this pass.
#[derive(Debug)]
#[non_exhaustive]
pub struct JobProgressOutput {
    /// The dialog's response.
    pub response: Response,
    /// Cancel or Hide, the pass a button, Escape or the close action answers; `None` while the job runs on.
    pub answer: Option<JobAnswer>,
}

/// What the user chose in a [`JobProgress`] window.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum JobAnswer {
    /// Stop the job.
    Cancel,
    /// Keep the job running in the background.
    Hide,
}
