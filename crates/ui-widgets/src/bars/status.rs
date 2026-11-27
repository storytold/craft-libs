//! The members of a status `Bar` beside `StatusReadout`: the status message and a background task's progress.

use crate::buttons::{IconButton, Link};
use crate::contract::{Outcome as _, Props as _, Show as _, TooltipSpec};
use crate::display::{Divider, Elide, Progress, Shade, Text};
use crate::primitives::Wrap;
use crate::runtime::commands::CommandRef;
use crate::runtime::env::{Env, Label};
use crate::style::TypeRole;
use craft_behavior::IconId;
use craft_theme::variant::Tone;
use egui::{Response, Ui};

/// A message for the status bar and its tone. A component with something to say in the status bar reports this in
/// its output; the host keeps the newest one and shows it as the last member of its status bar's body.
#[derive(Clone, Debug, PartialEq, crate::contract::Pattern, crate::contract::MemoProps)]
#[non_exhaustive]
pub struct StatusMessage {
    /// The message.
    pub text: Label,
    /// The tone it reads in; `Neutral` reads faint.
    pub tone: Tone,
    /// The command a click on the message runs; the message then shows as a link.
    pub command: Option<CommandRef>,
    /// The tooltip the message shows.
    pub tooltip: Option<Label>,
}

impl StatusMessage {
    /// A neutral message of `text`.
    pub fn new(text: impl Into<Label>) -> Self {
        StatusMessage { text: text.into(), tone: Tone::Neutral, command: None, tooltip: None }
    }

    /// Sets the tone the message reads in.
    #[must_use]
    pub fn tone(mut self, tone: Tone) -> Self {
        self.tone = tone;
        self
    }

    /// Makes the message a link that runs `command` on a click, Enter or Space.
    #[must_use]
    pub fn command(mut self, command: impl Into<CommandRef>) -> Self {
        self.command = Some(command.into());
        self
    }

    /// Sets the tooltip the message shows.
    #[must_use]
    pub fn tooltip(mut self, text: impl Into<Label>) -> Self {
        self.tooltip = Some(text.into());
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
        if let Some(command) = &self.command {
            // The underline keeps colour from being the only cue that the message is a link.
            let mut link = Link::new(self.text.clone()).wrap(Wrap::Elide).command(command.clone());
            if self.tone != Tone::Neutral {
                link = link.tone(self.tone);
            }
            if let Some(tooltip) = &self.tooltip {
                link = link.tooltip(TooltipSpec::new(tooltip.clone()));
            }
            return Some(link.show(ui).response().clone());
        }
        let text = Text::new(self.text.clone()).elide(Elide::End);
        let text = if self.tone == Tone::Neutral { text.shade(Shade::Faint) } else { text.tone(self.tone) };
        let text = match &self.tooltip {
            Some(tooltip) => text.tooltip(TooltipSpec::new(tooltip.clone())),
            None => text,
        };
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
#[derive(crate::contract::Pattern, crate::contract::MemoProps)]
pub struct StatusProgress {
    label: Label,
    fraction: Option<f32>,
    cancel: Option<Label>,
    refused: Option<Label>,
}

impl StatusProgress {
    /// A task named `label`, `fraction` done between 0 and 1, or `None` while it has reported nothing.
    pub fn new(label: impl Into<Label>, fraction: Option<f32>) -> Self {
        StatusProgress { label: label.into(), fraction, cancel: None, refused: None }
    }

    /// Adds a cancel button named `label`.
    #[must_use]
    pub fn cancel(mut self, label: impl Into<Label>) -> Self {
        self.cancel = Some(label.into());
        self
    }

    /// Shows the cancel button disabled, with `reason` as the tooltip that says why the task cannot be cancelled.
    #[must_use]
    pub fn cancel_refused(mut self, reason: impl Into<Label>) -> Self {
        self.refused = Some(reason.into());
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
        let (cancel, refused) = (self.cancel, self.refused);
        let cancelled = cancel.is_some_and(|label| {
            let button = IconButton::new(IconId::new("close")).label(label.clone());
            // A disabled button with a reason stays focusable, so the reason is reachable without a pointer.
            let button = match refused {
                Some(reason) => button.tooltip(TooltipSpec::new(label).disabled_text(reason)).enabled(false),
                None => button,
            };
            crate::contract::clicked(&button.show(ui))
        });
        StatusProgressOutput { response, cancelled }
    }
}
