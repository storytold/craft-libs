//! `JobProgress`: the modal window of a running job, a configuration of `Prompt` with a progress bar.

use super::prompt::{Prompt, Reply, buttons};
use super::words::DialogWords;
use crate::contract::{Bind, Show as _, ShowContainer as _};
use crate::display::{Progress, Shade, Text};
use crate::layout_components::Row;
use crate::runtime::env::{Env, Label};
use crate::runtime::words::CommonWords;
use crate::sections::DialogRole;
use crate::style::TypeRole;
use egui::{Response, Ui};

/// Modal progress window for a running job: a progress bar (or an indeterminate one), counts and the percentage, the
/// queued line, and Cancel (and Hide, when `can_hide`). Escape and the close action give Cancel.
// PLAN: `#[pattern(dialog)]` needs `StackDialog` (`dialog_probe`); the app draws this window directly, so it registers as a plain `Pattern`
#[derive(crate::contract::Pattern)]
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
        JobProgress { title: title.into(), step: step.into(), fraction: None, counts: None, queued: None, can_hide: false }
    }

    /// The finished fraction in `0.0..=1.0`; `None` shows an indeterminate bar.
    #[must_use]
    pub fn fraction(mut self, fraction: Option<f32>) -> Self {
        self.fraction = fraction;
        self
    }

    /// The counts line, shown muted with the percentage.
    #[must_use]
    pub fn counts(mut self, counts: impl Into<Label>) -> Self {
        self.counts = Some(counts.into());
        self
    }

    /// The queued line.
    #[must_use]
    pub fn queued(mut self, queued: impl Into<Label>) -> Self {
        self.queued = Some(queued.into());
        self
    }

    /// Whether the window offers Hide beside Cancel.
    #[must_use]
    pub fn can_hide(mut self, can_hide: bool) -> Self {
        self.can_hide = can_hide;
        self
    }

    /// Shows the window; `answer` is `Some` the pass Cancel or Hide answers.
    pub fn show(self, ui: &mut Ui) -> JobProgressOutput {
        let JobProgress { title, step, fraction, counts, queued, can_hide } = self;
        let env = Env::of(ui);
        let (words, common) = (env.words::<DialogWords>(), env.words::<CommonWords>());
        let percent = fraction.map(|fraction| env.view().format().percent(f64::from(Progress::clamped(fraction)), 0));
        let mut replies = vec![Reply::plain(common.cancel.clone(), DialogRole::Cancel)];
        if can_hide {
            replies.push(Reply::plain(words.job_progress_hide.clone(), DialogRole::Other));
        }
        // The open state is this pass's own: the app draws the window only while a job wants it, so an answer must not
        // leave a session slot closed for the next job.
        let mut open = true;
        let shown = Prompt::new(title, step)
            .answers(buttons(&env, replies))
            .extras(|ui| {
                match fraction {
                    Some(fraction) => Progress::new(fraction).show(ui),
                    None => Progress::indeterminate().show(ui),
                };
                if counts.is_some() || percent.is_some() {
                    Row::new().show(ui, |ui, _| {
                        for text in counts.iter().cloned().chain(percent.iter().cloned().map(Label::from)) {
                            Text::new(text).role(TypeRole::Caption).shade(Shade::Muted).show(ui);
                        }
                    });
                }
                if let Some(queued) = &queued {
                    Text::new(queued.clone()).show(ui);
                }
            })
            .opened(Bind::bound(&mut open))
            .show(ui);
        let answer = shown.picked.map(|index| if index == 0 { JobAnswer::Cancel } else { JobAnswer::Hide });
        JobProgressOutput { response: shown.response, answer }
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
