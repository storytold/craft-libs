//! `ZoomField`: the one zoom field, a number field over a canvas view's zoom in percent.

use super::NumberField;
use super::words::TextWords;
use crate::contract::{Bind, NoValue, Outcome, Props as _, Show as _};
use crate::runtime::env::{Env, Label, UnitKind};
use craft_behavior::canvas::zoom;
use craft_behavior::kurbo::Size;
use craft_behavior::{CanvasView, EditEvents, ZoomLadder};
use egui::{Response, Ui};

#[cfg(feature = "suite")]
pub(crate) mod stories;

/// The decimals the zoom range's ends are written with in the message, as the field shows a zoom.
const MESSAGE_DECIMALS: usize = 2;

/// A zoom field: a `NumberField` over the view's device zoom in percent, with the zoom codec and the zoom ladder's
/// range. With no view it shows a disabled empty field. An edit writes the zoom about the view's centre; an empty
/// commit keeps the view.
///
/// - Typing takes digits, `/` and the decimal separator: `a/b` is the zoom `a ÷ b × 100` %, and a second separator
///   applies the number before it and reports [`InvalidZoom`].
/// - Tab writes the typed number in the field's own form and selects it; focus stays and nothing applies.
/// - A press on a part declared `holds_drafts` (the pasteboard, the title bar) keeps the typed zoom unapplied; the
///   next Enter applies it.
#[derive(crate::contract::Pattern)]
pub struct ZoomField {
    label: Label,
    view: Option<CanvasView>,
    document: Size,
    stepper: bool,
    scrub: bool,
    enabled: bool,
    status_bar: bool,
}

impl ZoomField {
    /// A zoom field named `label` over `view` (`None` when no view is known) of a document of `document` units.
    pub fn new(label: impl Into<Label>, view: Option<CanvasView>, document: Size) -> Self {
        ZoomField { label: label.into(), view, document, stepper: false, scrub: false, enabled: true, status_bar: false }
    }

    /// The arrow keys and the wheel step along the zoom ladder.
    #[must_use]
    pub fn stepper(mut self, on: bool) -> Self {
        self.stepper = on;
        self
    }

    /// The caller's side label scrubs the zoom.
    #[must_use]
    pub fn scrub(mut self, on: bool) -> Self {
        self.scrub = on;
        self
    }

    /// Whether the field takes input; it is disabled anyway while there is no view.
    #[must_use]
    pub fn enabled(mut self, on: bool) -> Self {
        self.enabled = on;
        self
    }

    /// Uses the status bar's space and field appearance.
    #[must_use]
    pub(crate) fn status_bar(mut self) -> Self {
        self.status_bar = true;
        self
    }

    /// Shows the field and reports the view changes it made.
    pub fn show(self, ui: &mut Ui) -> ZoomFieldOutput {
        let env = Env::of(ui);
        let pixels_per_point = env.scale.0;
        let document = self.document;
        let device = self.view.as_ref().map(|view| zoom::device(view, pixels_per_point));
        // The document's zoom range widened to the current zoom, so the field never pulls the zoom back.
        let range = device.map(|device| ZoomLadder::percent_range(device, document));
        let mut field = NumberField::new(self.label, Bind::given(device.map(|device| device * 100.0)))
            .codec(env.units().zoom_codec())
            .unit_kind(UnitKind::Percent)
            .zoom_typing()
            .enabled(self.enabled && self.view.is_some());
        if let Some(range) = range.clone() {
            field = field.range(range);
        }
        if self.stepper {
            field = field.stepper(move |percent: f64, dir: i32| ZoomLadder::step(percent / 100.0, dir, document) * 100.0);
        }
        if self.scrub {
            field = field.scrub_by_label();
        }
        if self.status_bar {
            field = field.status_zoom();
        }
        let shown = field.show(ui);
        let invalid_zoom = range.filter(|_| shown.cut).map(|range| InvalidZoom { low: *range.start(), high: *range.end() });
        let view = match self.view {
            Some(current) => shown
                .edit
                .events
                .clone()
                .map(|percent| percent.map_or(current, |percent| zoom::with_device(&current, percent / 100.0, document, pixels_per_point))),
            None => EditEvents::new(),
        };
        ZoomFieldOutput { response: shown.edit.response.clone(), view, invalid_zoom }
    }
}

/// What a zoom field reports.
#[derive(Debug)]
#[non_exhaustive]
pub struct ZoomFieldOutput {
    /// The field's response.
    pub response: Response,
    /// The view changes of this pass.
    pub view: EditEvents<CanvasView>,
    /// The committed text held a second decimal separator, so the number before it applied, in the pass it
    /// committed; the host shows [`InvalidZoom::message`].
    pub invalid_zoom: Option<InvalidZoom>,
}

impl Outcome for ZoomFieldOutput {
    type Value = NoValue;

    fn response(&self) -> &Response {
        &self.response
    }

    #[cfg(feature = "testing")]
    fn story_facts(&self) -> crate::testing::StoryOutput {
        crate::testing::StoryOutput::of_edit(&self.response, &self.view)
    }
}

/// The zoom range the field takes, in percent, for the message a cut zoom shows.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub struct InvalidZoom {
    /// The least zoom the field takes.
    pub low: f64,
    /// The greatest zoom the field takes.
    pub high: f64,
}

impl InvalidZoom {
    /// The message that says which zoom range the field takes, in the env's number format ("An integer between
    /// 0.25% and 12,800.00% is required. Closest value inserted.").
    pub fn message(&self, env: &Env) -> String {
        let format = env.format();
        let low = format.percent(self.low / 100.0, MESSAGE_DECIMALS);
        let high = format.percent(self.high / 100.0, MESSAGE_DECIMALS);
        env.words::<TextWords>().range_integer.fill(&[("low", low.as_str()), ("high", high.as_str())])
    }
}
