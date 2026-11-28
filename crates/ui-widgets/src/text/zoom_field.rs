//! `ZoomField`: the one zoom field, a number field over a canvas view's zoom in percent.

use super::NumberField;
use super::words::TextWords;
use crate::contract::{Bind, NoValue, Outcome, Props as _, Show as _};
use crate::runtime::env::{Env, Label, UnitKind};
use craft_behavior::kurbo::Size;
use craft_behavior::{CanvasView, EditEvents, ZoomScheme};
use egui::{Rect, Response, Ui};

#[cfg(feature = "suite")]
pub(crate) mod stories;

/// The decimals the zoom range's ends are written with in the message, as the field shows a zoom.
const MESSAGE_DECIMALS: usize = 2;

/// A zoom field: a `NumberField` over the view's zoom in percent on its [`ZoomScale`] (device pixels unless set),
/// with the zoom codec and the zoom scheme's range. With no view it shows a disabled empty field. An edit writes the zoom about the view's centre; an empty
/// commit keeps the view.
///
/// - Typing takes digits, `-`, `/` and the decimal separator: `a/b` is the zoom `a ÷ b × 100` %. Non-positive
///   entries are rejected; a second separator applies the number before it and reports [`InvalidZoom`].
/// - Tab writes the typed number in the field's own form and selects it; focus stays and nothing applies. With nothing
///   typed, Tab and Shift+Tab move focus as in any field.
/// - A press on a part declared `holds_drafts` (the pasteboard, the title bar) keeps the typed zoom unapplied; the
///   next Enter applies it.
#[derive(crate::contract::Pattern)]
pub struct ZoomField {
    label: Label,
    /// The zoom shown, in view points per document unit (`CanvasView::zoom`).
    zoom: Option<f64>,
    view: Option<CanvasView>,
    document: Size,
    stepper: bool,
    scrub: bool,
    enabled: bool,
    status_bar: bool,
    scale: ZoomScale,
}

/// What a zoom field's 100 % means.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub enum ZoomScale {
    /// One view unit is one device pixel (the default).
    #[default]
    Device,
    /// One view unit is one document point: 100 % is the document's actual size.
    Points,
}

impl ZoomScale {
    /// The pixels one view point counts as at `pixels_per_point`: the screen's for device pixels, one for document
    /// points, so `zoom::device` and `zoom::with_device` read and write the zoom on this scale.
    fn pixels_per_unit(self, pixels_per_point: f32) -> f32 {
        match self {
            ZoomScale::Device => pixels_per_point,
            ZoomScale::Points => 1.0,
        }
    }
}

impl ZoomField {
    /// A zoom field named `label` over `view` (`None` when no view is known) of a document of `document` units.
    pub fn new(label: impl Into<Label>, view: Option<CanvasView>, document: Size) -> Self {
        ZoomField {
            label: label.into(),
            zoom: view.map(|view| view.zoom),
            view,
            document,
            stepper: false,
            scrub: false,
            enabled: true,
            status_bar: false,
            scale: ZoomScale::Device,
        }
    }

    /// A zoom field over `zoom` alone (view points per document unit, as `CanvasView::zoom`; `None` when none is known),
    /// for a host that keeps the rest of the view itself: [`show_zoom`](Self::show_zoom) reports the zoom the field set.
    pub(crate) fn of_zoom(label: impl Into<Label>, zoom: Option<f64>, document: Size) -> Self {
        ZoomField {
            label: label.into(),
            zoom,
            view: None,
            document,
            stepper: false,
            scrub: false,
            enabled: true,
            status_bar: false,
            scale: ZoomScale::Device,
        }
    }

    /// What 100 % means: device pixels (the default) or document points.
    #[must_use]
    pub fn scale(mut self, scale: ZoomScale) -> Self {
        self.scale = scale;
        self
    }

    /// The arrow keys and the wheel step along the zoom scheme's rungs.
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
        self.show_in(ui, None)
    }

    /// Shows the field in a caller-allocated rectangle, including narrow slots such as a rail.
    pub fn show_at(self, ui: &mut Ui, rect: Rect) -> ZoomFieldOutput {
        self.show_in(ui, Some(rect))
    }

    fn show_in(self, ui: &mut Ui, rect: Option<Rect>) -> ZoomFieldOutput {
        let view = self.view;
        let shown = self.show_zoom_in(ui, rect);
        let view = match view {
            Some(current) => shown.zoom.map(|zoom| {
                let mut next = current;
                next.zoom = zoom;
                next
            }),
            None => EditEvents::new(),
        };
        ZoomFieldOutput { response: shown.response, view, invalid_zoom: shown.invalid_zoom }
    }

    /// Shows the field and reports the zoom it set, clamped to the document's zoom range.
    pub(crate) fn show_zoom(self, ui: &mut Ui) -> ZoomEdit {
        self.show_zoom_in(ui, None)
    }

    fn show_zoom_in(self, ui: &mut Ui, rect: Option<Rect>) -> ZoomEdit {
        let env = Env::of(ui);
        let pixels_per_point = self.scale.pixels_per_unit(env.scale.0);
        let scheme = env.view().get::<ZoomScheme>();
        let document = self.document;
        let device = self.zoom.map(|zoom| zoom * f64::from(pixels_per_point));
        // The document's zoom range widened to the current zoom, so the field never pulls the zoom back.
        let range = device.map(|device| scheme.percent_range(device, document));
        let mut field = NumberField::new(self.label, Bind::given(device.map(|device| scheme.percent(device))))
            .codec(env.units().zoom_codec().signed_zoom())
            .unit_kind(UnitKind::Percent)
            .zoom_typing()
            .enabled(self.enabled && self.zoom.is_some());
        if let Some(range) = range.clone() {
            field = field.range(range);
        }
        if self.stepper {
            field = field.stepper(move |percent: f64, dir: i32, _scale: f64| scheme.percent(scheme.step(scheme.device(percent), dir, document)));
        }
        if self.scrub {
            field = field.scrub_by_label();
        }
        if self.status_bar {
            field = field.status_zoom();
        }
        let shown = match rect {
            Some(rect) => field.show_at(ui, rect),
            None => field.show(ui),
        };
        let invalid_zoom = range.filter(|_| shown.cut).map(|range| InvalidZoom { low: *range.start(), high: *range.end() });
        let zoom = match self.zoom {
            Some(current) => shown.edit.events.clone().map(|percent| {
                percent.map_or(current, |percent| CanvasView::points_per_unit(scheme.clamp(scheme.device(percent), document), pixels_per_point))
            }),
            None => EditEvents::new(),
        };
        ZoomEdit { response: shown.edit.response.clone(), zoom, invalid_zoom }
    }
}

/// What a zoom field over a zoom alone reports.
pub(crate) struct ZoomEdit {
    pub(crate) response: Response,
    /// The zoom changes of this pass, in view points per document unit.
    pub(crate) zoom: EditEvents<f64>,
    pub(crate) invalid_zoom: Option<InvalidZoom>,
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
