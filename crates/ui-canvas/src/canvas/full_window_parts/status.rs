//! `FullWindowStatus`: the progress or error line of a preview a full-window workspace computes off the UI thread.

use crate::contract::{NoValue, Outcome, Props as _, Show as _, ShowContainer as _};
use crate::dialog::DialogWords;
use crate::display::Progress;
use crate::layout_components::Row;
use crate::notices::{Notice, NoticeLayout};
use crate::runtime::env::{Env, Label};
use crate::runtime::pending::Pending;
use craft_theme::variant::Tone;
use egui::{Response, Ui};
use std::fmt::Display;

/// What a preview's job is doing, as the status line shows it.
#[derive(Clone, Debug, PartialEq)]
enum Phase {
    /// Done: nothing to show.
    Idle,
    /// Running, with the last reported fraction.
    Loading(Option<f32>),
    /// Failed, with the error's text.
    Failed(String),
    /// Ended without delivering.
    Lost,
}

/// What the status line reports.
#[derive(Debug)]
#[non_exhaustive]
pub struct FullWindowStatusOutput {
    /// The line's response: an empty row while nothing shows.
    pub response: Response,
}

impl Outcome for FullWindowStatusOutput {
    type Value = NoValue;

    fn response(&self) -> &Response {
        &self.response
    }
}

/// The status of a preview over a [`Pending`]: nothing while it is ready, a progress bar (with the polite label
/// "Updating preview") while it loads, a notice with the error while it failed and a notice that the preview is
/// unavailable when its job ended without delivering. A stale result needs no ticket here: `Remember::pending`
/// cancels a job whose key changed, so a result that lands is the latest one.
///
/// The zoom bar shows the same progress in its strip: pass [`progress`](Self::progress) to it.
#[derive(crate::contract::Pattern)]
#[must_use = "show the status with `show`"]
pub struct FullWindowStatus {
    phase: Phase,
}

impl FullWindowStatus {
    /// The status of `pending`; a failure shows `Display` of the error.
    pub fn of<T, E: Display>(pending: &Pending<T, E>) -> Self {
        let phase = match pending {
            Pending::Loading { progress } => Phase::Loading(*progress),
            Pending::Ready(_) => Phase::Idle,
            Pending::Failed(error) => Phase::Failed(error.to_string()),
            _ => Phase::Lost,
        };
        FullWindowStatus { phase }
    }

    /// `Some` while loading: the fraction reported so far, `None` inside while none was reported. The zoom bar's
    /// `progress`.
    pub fn progress(&self) -> Option<Option<f32>> {
        match self.phase {
            Phase::Loading(fraction) => Some(fraction),
            _ => None,
        }
    }

    /// Shows the line.
    pub fn show(self, ui: &mut Ui) -> FullWindowStatusOutput {
        let words = Env::of(ui).words::<DialogWords>();
        let row = Row::new().show(ui, |ui, _| match &self.phase {
            Phase::Idle => {}
            Phase::Loading(Some(fraction)) => {
                Progress::new(*fraction).label(words.full_window_updating.clone()).show(ui);
            }
            Phase::Loading(None) => {
                Progress::indeterminate().label(words.full_window_updating.clone()).show(ui);
            }
            Phase::Failed(error) => {
                let text = Label::from(words.full_window_failed.fill(&[("error", error.as_str())]));
                Notice::new(Tone::Bad, text).layout(NoticeLayout::Inline).show(ui);
            }
            Phase::Lost => {
                Notice::new(Tone::Warn, words.full_window_lost.clone()).layout(NoticeLayout::Inline).show(ui);
            }
        });
        let response = row.output;
        FullWindowStatusOutput { response }
    }
}
