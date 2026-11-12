//! The members of a status `Bar` beside `StatusReadout`: the status message and a background task's progress.

use crate::buttons::IconButton;
use crate::contract::{Props as _, Show as _};
use crate::display::{Divider, Elide, Progress, Shade, Text};
use crate::runtime::env::{Env, Label};
use crate::style::TypeRole;
use craft_behavior::IconId;
use craft_theme::variant::Tone;
use egui::{Response, Ui};

/// A message for the status bar and its tone. A component with something to say in the status bar reports this in
/// its output; the host keeps the newest one and shows it as the last member of its status bar's body.
#[derive(Clone, Debug, PartialEq, crate::contract::Pattern)]
#[non_exhaustive]
pub struct StatusMessage {
    /// The message.
    pub text: Label,
    /// The tone it reads in; `Neutral` reads faint.
    pub tone: Tone,
}

impl StatusMessage {
    /// A neutral message of `text`.
    pub fn new(text: impl Into<Label>) -> Self {
        StatusMessage { text: text.into(), tone: Tone::Neutral }
    }

    /// Sets the tone the message reads in.
    #[must_use]
    pub fn tone(mut self, tone: Tone) -> Self {
        self.tone = tone;
        self
    }

    /// Shows the message after a rule: faint, or in the colour of its tone, on one line that ends in an ellipsis when
    /// it does not fit and then shows whole in a tooltip. The status bar's live region announces it. An empty message
    /// shows nothing, its rule included.
    pub fn show(&self, ui: &mut Ui) -> Option<Response> {
        if self.text.as_str().is_empty() {
            return None;
        }
        Divider::vertical().show(ui);
        let text = Text::new(self.text.clone()).elide(Elide::End);
        let text = if self.tone == Tone::Neutral { text.shade(Shade::Faint) } else { text.tone(self.tone) };
        Some(text.show(ui))
    }
}

/// What a task's progress reports.
#[derive(Debug)]
#[non_exhaustive]
pub struct StatusProgressOutput {
    /// The progress bar's response.
    pub response: Response,
    /// The cancel button was clicked this pass.
    pub cancelled: bool,
}

/// A background task's progress, for a status bar's trailing group: the task's name, a progress bar that moves a
/// segment while the extent is unknown, the percentage and an optional cancel button. The progress bar is named by
/// the task and reports the fraction as its value.
#[derive(crate::contract::Pattern)]
pub struct StatusProgress {
    label: Label,
    fraction: Option<f32>,
    cancel: Option<Label>,
}

impl StatusProgress {
    /// A task named `label`, `fraction` done between 0 and 1, or `None` while it has reported nothing.
    pub fn new(label: impl Into<Label>, fraction: Option<f32>) -> Self {
        StatusProgress { label: label.into(), fraction, cancel: None }
    }

    /// Adds a cancel button named `label`.
    #[must_use]
    pub fn cancel(mut self, label: impl Into<Label>) -> Self {
        self.cancel = Some(label.into());
        self
    }

    /// Shows the name and the bar, the percentage and the cancel button, in reading order.
    pub fn show(self, ui: &mut Ui) -> StatusProgressOutput {
        Text::new(self.label.clone()).shade(Shade::Muted).show(ui);
        let fraction = self.fraction;
        let bar = match fraction {
            Some(fraction) => Progress::new(fraction),
            None => Progress::indeterminate(),
        };
        let response = bar.label(self.label).show(ui);
        if let Some(fraction) = fraction {
            let percent = Env::of(ui).format().percent(f64::from(Progress::clamped(fraction)), 0);
            Text::new(percent).role(TypeRole::Mono).shade(Shade::Muted).show(ui);
        }
        let cancelled = self.cancel.is_some_and(|label| IconButton::new(IconId::new("close")).label(label).show(ui).clicked());
        StatusProgressOutput { response, cancelled }
    }
}
