//! `FullWindowTitle`: the text of a full-window dialog's title strip.

use crate::contract::{NoValue, Outcome, Show as _};
use crate::dialog::DialogWords;
use crate::display::{Elide, Text};
use crate::runtime::env::{Env, Label};
use egui::{Response, Ui};

/// What the title strip reports.
#[derive(Debug)]
#[non_exhaustive]
pub struct FullWindowTitleOutput {
    /// The title text's response.
    pub response: Response,
}

impl Outcome for FullWindowTitleOutput {
    type Value = NoValue;

    fn response(&self) -> &Response {
        &self.response
    }
}

/// The title strip's text: the workspace's name, then in brackets what it works on and the zoom on screen,
/// "Liquify (Layer 1, 66.67%)". Without a subject "Liquify (66.67%)"; without a zoom "Adaptive Wide Angle (Layer 1)"; with
/// neither just the name.
///
/// - A trailing ellipsis of the name is dropped, so a menu item's name ("Liquify…") can be passed as it is.
/// - The zoom shows up to two decimals (`Units::zoom_percent`), in the user's language and digits.
/// - [`label`](Self::label) is the text for a dialog's title row; [`show`](Self::show) draws it as one line that
///   elides at the end and shows whole in a tooltip.
#[derive(crate::contract::Pattern)]
#[must_use = "show the title with `show` or take its text with `label`"]
pub struct FullWindowTitle {
    name: Label,
    subject: Option<Label>,
    zoom: Option<f64>,
}

impl FullWindowTitle {
    /// A title of the workspace `name`.
    pub fn new(name: impl Into<Label>) -> Self {
        FullWindowTitle { name: name.into(), subject: None, zoom: None }
    }

    /// What the workspace works on: the layer's or the document's name. An empty one is none.
    pub fn subject(mut self, subject: impl Into<Label>) -> Self {
        self.subject = Some(subject.into()).filter(|subject| !subject.as_str().is_empty());
        self
    }

    /// The zoom on screen as a percentage, `FullWindowView::zoom_percent`.
    pub fn zoom(mut self, percent: f64) -> Self {
        self.zoom = Some(percent);
        self
    }

    /// The text under `env`'s words and number format.
    pub fn label(&self, env: &Env) -> Label {
        let words = env.words::<DialogWords>();
        let name = env.view().resolve(&self.name);
        let name = name.as_str().trim_end_matches('…').trim_end_matches("...").trim_end();
        let zoom = self.zoom.map(|percent| env.units().zoom_percent(percent));
        let subject = self.subject.as_ref().map(|subject| env.view().resolve(subject));
        let text = match (subject.as_ref().map(Label::as_str), zoom.as_deref()) {
            (Some(subject), Some(zoom)) => words.full_window_title_zoom.fill(&[("name", name), ("subject", subject), ("zoom", zoom)]),
            (Some(subject), None) => words.full_window_title_subject.fill(&[("name", name), ("subject", subject)]),
            (None, Some(zoom)) => words.full_window_title_zoom_only.fill(&[("name", name), ("zoom", zoom)]),
            (None, None) => name.to_owned(),
        };
        Label::from(text)
    }

    /// Shows the title as one line.
    pub fn show(self, ui: &mut Ui) -> FullWindowTitleOutput {
        let label = self.label(&Env::of(ui));
        FullWindowTitleOutput { response: Text::new(label).elide(Elide::End).show(ui) }
    }
}
