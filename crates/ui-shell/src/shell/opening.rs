//! `OpeningCard`: the card shown in the document area while a file opens.

use super::words::ShellWords;
use crate::buttons::Button;
use crate::contract::{Gap, Outcome, Pad, Pattern, Props as _, Show as _, ShowContainer as _, Sizing, clicked};
use crate::display::{Elide, Progress, Shade, Text};
use crate::layout::RowAlign;
use crate::layout_components::{Row, Stack};
use crate::runtime::env::{Env, Label};
use crate::style::{SurfaceKind, TypeRole};
use egui::{Layout, Response, Ui, UiBuilder};

#[cfg(feature = "testing")]
mod stories;

/// A centred card: "Opening {name}…" elided on one line with the full text on hover, the muted message, a progress bar
/// (indeterminate without a fraction), the percentage at the start and Cancel at the end of the last row. Job UI
/// only: the fraction and the cancel answer come from the app's engine job.
///
/// The card takes the room it is shown in and sits at its centre; its height follows its content. Cancel reports
/// `cancelled` in the pass of the click only, so a second pass in the same frame, which gets no input, reports none.
#[must_use = "show the card with `show`"]
#[derive(Pattern)]
pub struct OpeningCard {
    name: Label,
    message: Option<Label>,
    fraction: Option<f32>,
    cancel: Option<Label>,
}

/// What an [`OpeningCard`] reports.
#[derive(Debug)]
#[non_exhaustive]
#[must_use = "the app cancels the job when `cancelled` is set"]
pub struct OpeningCardOutput {
    /// The card's response.
    pub response: Response,
    /// Cancel was picked in this pass.
    pub cancelled: bool,
}

impl Outcome for OpeningCardOutput {
    type Value = crate::contract::NoValue;

    fn response(&self) -> &Response {
        &self.response
    }
}

impl OpeningCard {
    /// A card for the file `name`.
    pub fn new(name: impl Into<Label>) -> Self {
        OpeningCard { name: name.into(), message: None, fraction: None, cancel: None }
    }

    /// The muted message under the title.
    pub fn message(mut self, message: impl Into<Label>) -> Self {
        self.message = Some(message.into());
        self
    }

    /// How far the job is, 0 to 1; `None` shows an indeterminate bar.
    pub fn fraction(mut self, fraction: Option<f32>) -> Self {
        self.fraction = fraction;
        self
    }

    /// The Cancel button's text; without it the card has no Cancel.
    pub fn cancel(mut self, cancel: impl Into<Label>) -> Self {
        self.cancel = Some(cancel.into());
        self
    }

    /// Shows the card centred in `ui`'s free room.
    pub fn show(self, ui: &mut Ui) -> OpeningCardOutput {
        let OpeningCard { name, message, fraction, cancel } = self;
        let env = Env::of(ui);
        let words = env.words::<ShellWords>();
        let title = words.opening.fill(&[("name", name.as_str())]);
        let room = ui.available_rect_before_wrap();
        let width = room.width().min(env.tokens.size.dialog.least);
        let mut cancelled = false;
        let placed = ui.scope_builder(UiBuilder::new().max_rect(room).layout(Layout::centered_and_justified(egui::Direction::TopDown)), |ui| {
            ui.set_max_width(width);
            let _ = Stack::new().pad(Pad::Roomy).gap(Gap::Section).surface(SurfaceKind::Dialog).fill_width().show(ui, |ui, _| {
                let _ = Text::new(title.clone()).role(TypeRole::BodyStrong).elide(Elide::End).sizing(Sizing::FillWidth).show(ui);
                if let Some(message) = &message {
                    let _ = Text::new(message.clone()).shade(Shade::Muted).elide(Elide::End).sizing(Sizing::FillWidth).show(ui);
                }
                let _ = match fraction {
                    Some(fraction) => Progress::new(fraction).label(title.clone()).sizing(Sizing::FillWidth).show(ui),
                    None => Progress::indeterminate().label(title.clone()).sizing(Sizing::FillWidth).show(ui),
                };
                let percent = fraction.map(|fraction| env.units().percent(f64::from((Progress::clamped(fraction) * 100.0).round())));
                if percent.is_some() || cancel.is_some() {
                    // With one child a space-between row would put it at the start edge; Cancel alone belongs at the end.
                    let align = if percent.is_some() { RowAlign::SpaceBetween } else { RowAlign::End };
                    let _ = Row::new().align(align).fill_width().show(ui, |ui, _| {
                        if let Some(percent) = &percent {
                            let _ = Text::new(percent.clone()).shade(Shade::Muted).show(ui);
                        }
                        if let Some(cancel) = &cancel
                            && clicked(&Button::new(cancel.clone()).id_salt("cancel").show(ui))
                        {
                            cancelled = true;
                        }
                    });
                }
            });
        });
        OpeningCardOutput { response: placed.response, cancelled }
    }
}
